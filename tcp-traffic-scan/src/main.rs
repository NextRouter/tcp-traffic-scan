use axum::{extract::Query, http::StatusCode, response::IntoResponse, routing::get, Router};
use clap::Parser;
use lazy_static::lazy_static;
use libc;
use prometheus::{Encoder, GaugeVec, Opts, Registry, TextEncoder};
use socket2::{Domain, Socket, Type};
use std::collections::HashMap;
#[cfg(target_os = "linux")]
use std::ffi::CString;
use std::io;
use std::io::Write;
use std::net::{SocketAddr, ToSocketAddrs};
use std::os::unix::io::AsRawFd;
#[cfg(not(target_os = "linux"))]
use std::sync::Once;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;

lazy_static! {
    static ref REGISTRY: Registry = Registry::new();
    static ref BANDWIDTH_GAUGE: GaugeVec = {
        let opts = Opts::new("tcp_bandwidth_bps", "TCP bandwidth estimation in bps")
            .namespace("tcp_traffic_scan");
        let gauge = GaugeVec::new(opts, &["interface", "server_ip"]).unwrap();
        REGISTRY.register(Box::new(gauge.clone())).unwrap();
        gauge
    };
    static ref BANDWIDTH_AVG_GAUGE: GaugeVec = {
        let opts = Opts::new(
            "tcp_bandwidth_avg_bps",
            "TCP bandwidth average per interface in bps",
        )
        .namespace("tcp_traffic_scan");
        let gauge = GaugeVec::new(opts, &["interface"]).unwrap();
        REGISTRY.register(Box::new(gauge.clone())).unwrap();
        gauge
    };
    // Per-interface correction factors: wan0->eth0, wan1->eth1, etc.
    // Key is the actual interface name (e.g., "eth0"), value is correction factor
    static ref CORRECTION_FACTORS: Arc<Mutex<HashMap<String, f64>>> = Arc::new(Mutex::new(HashMap::new()));
    static ref DEFAULT_CORRECTION_FACTOR: Arc<Mutex<f64>> = Arc::new(Mutex::new(1.0));
}

#[derive(serde::Deserialize)]
struct CorrectionQuery {
    value: Option<f64>,
    nic: Option<String>,
}

// Map wan names to actual interface names
fn map_wan_to_interface(wan_name: &str) -> String {
    match wan_name {
        "wan0" => "eth0".to_string(),
        "wan1" => "eth1".to_string(),
        _ => wan_name.to_string(), // If not wan0/wan1, use as-is
    }
}

// Prometheus metrics server on port 59121
async fn start_metrics_server(running: Arc<AtomicBool>) {
    let app = Router::new().route("/metrics", get(metrics_handler));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:59121")
        .await
        .unwrap();

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            while running.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .unwrap();
}

async fn metrics_handler() -> impl IntoResponse {
    let mut buffer = Vec::new();
    let encoder = TextEncoder::new();

    // Get correction factors
    let correction_factors = CORRECTION_FACTORS.lock().unwrap().clone();
    let default_correction = *DEFAULT_CORRECTION_FACTOR.lock().unwrap();

    // Gather metrics
    let metric_families = REGISTRY.gather();

    // Apply correction factor to all gauge values based on interface
    let corrected_families: Vec<_> = metric_families
        .iter()
        .map(|mf| {
            let mut corrected_mf = mf.clone();
            for metric in corrected_mf.mut_metric() {
                if metric.has_gauge() {
                    let original_value = metric.get_gauge().get_value();

                    // Find the interface label to get the correct correction factor
                    let interface = metric
                        .get_label()
                        .iter()
                        .find(|label| label.get_name() == "interface")
                        .map(|label| label.get_value())
                        .unwrap_or("");

                    let correction = correction_factors
                        .get(interface)
                        .copied()
                        .unwrap_or(default_correction);

                    metric.mut_gauge().set_value(original_value * correction);
                }
            }
            corrected_mf
        })
        .collect();

    encoder.encode(&corrected_families, &mut buffer).unwrap();

    (StatusCode::OK, buffer)
}

// HTTP correction server on port 32600
async fn start_correction_server(running: Arc<AtomicBool>) {
    let app = Router::new().route("/tcpflow", get(correction_handler));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:32600")
        .await
        .unwrap();

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            while running.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .unwrap();
}

async fn correction_handler(Query(params): Query<CorrectionQuery>) -> impl IntoResponse {
    if let Some(value) = params.value {
        if value > 0.0 {
            if let Some(nic) = params.nic {
                // Map wan name to actual interface (wan0 -> eth0, wan1 -> eth1)
                let interface = map_wan_to_interface(&nic);
                CORRECTION_FACTORS
                    .lock()
                    .unwrap()
                    .insert(interface.clone(), value);
                (
                    StatusCode::OK,
                    format!(
                        "Correction factor for {} ({}) set to: {}\n",
                        nic, interface, value
                    ),
                )
            } else {
                // Set default correction factor for all interfaces
                *DEFAULT_CORRECTION_FACTOR.lock().unwrap() = value;
                (
                    StatusCode::OK,
                    format!("Default correction factor set to: {}\n", value),
                )
            }
        } else {
            (
                StatusCode::BAD_REQUEST,
                "Value must be greater than 0\n".to_string(),
            )
        }
    } else {
        // Display current correction factors
        let correction_factors = CORRECTION_FACTORS.lock().unwrap().clone();
        let default_correction = *DEFAULT_CORRECTION_FACTOR.lock().unwrap();

        let mut response = format!("Default correction factor: {}\n", default_correction);

        if !correction_factors.is_empty() {
            response.push_str("\nPer-interface correction factors:\n");
            for (interface, factor) in correction_factors.iter() {
                response.push_str(&format!("  {}: {}\n", interface, factor));
            }
        }

        (StatusCode::OK, response)
    }
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Network interfaces to use (can specify multiple)
    #[arg(short, long, action = clap::ArgAction::Append)]
    interface: Vec<String>,

    /// Server IP addresses to measure
    #[arg(short, long, action = clap::ArgAction::Append)]
    server: Vec<String>,
}

fn main() {
    let args = Args::parse();

    if args.interface.is_empty() {
        eprintln!("No interfaces specified. Use -i/--interface to add interfaces.");
        std::process::exit(2);
    }

    if args.server.is_empty() {
        eprintln!("No servers specified. Use -s/--server to add targets.");
        std::process::exit(2);
    }

    // Ctrl+C handling
    let running = Arc::new(AtomicBool::new(true));
    {
        let running = running.clone();
        let _ = ctrlc::set_handler(move || {
            running.store(false, Ordering::SeqCst);
        });
    }

    // Create tokio runtime for async servers
    let rt = Runtime::new().unwrap();

    // Start Prometheus metrics server (port 59121)
    {
        let running = running.clone();
        rt.spawn(async move {
            start_metrics_server(running).await;
        });
    }

    // Start HTTP correction server (port 32600)
    {
        let running = running.clone();
        rt.spawn(async move {
            start_correction_server(running).await;
        });
    }

    println!("Prometheus metrics available at http://localhost:59121/metrics");
    println!("Correction factor API available at http://localhost:32600/tcpflow?value=<factor>");
    println!("Starting measurements...");
    println!("==================================");

    // Main loop until Ctrl+C
    let sleep_duration = Duration::from_secs_f64(1.0);
    while running.load(Ordering::SeqCst) {
        for interface in &args.interface {
            let mut results = Vec::new();
            let mut bandwidth_sum = 0.0;
            let mut bandwidth_count = 0;

            for server_str in &args.server {
                match resolve_server_address(server_str) {
                    Ok(server_addr) => match measure_throughput(interface, server_addr) {
                        Ok((rtt, window_size)) => {
                            let throughput_bps = if rtt.as_secs_f64() > 0.0 {
                                (window_size as f64 * 8.0) / rtt.as_secs_f64()
                            } else {
                                0.0
                            };

                            // Update Prometheus metric (in bps)
                            BANDWIDTH_GAUGE
                                .with_label_values(&[interface, &server_addr.ip().to_string()])
                                .set(throughput_bps);

                            // Accumulate for average
                            bandwidth_sum += throughput_bps;
                            bandwidth_count += 1;

                            results.push(format!("{}:{:.0}bps", server_addr.ip(), throughput_bps));
                        }
                        Err(e) => {
                            eprintln!(
                                "Error measuring {} on {}: {}",
                                server_addr.ip(),
                                interface,
                                e
                            );
                            results.push(format!("{}:ERR", server_addr.ip()));
                        }
                    },
                    Err(e) => {
                        eprintln!("Error resolving server address for {}: {}", server_str, e);
                        results.push(format!("{}:N/A", server_str));
                    }
                }

                // Small delay between servers to stagger measurements
                std::thread::sleep(Duration::from_millis(100));
            }

            // Calculate and update average bandwidth for this interface
            if bandwidth_count > 0 {
                let avg_bandwidth = bandwidth_sum / bandwidth_count as f64;
                BANDWIDTH_AVG_GAUGE
                    .with_label_values(&[interface])
                    .set(avg_bandwidth);

                results.push(format!("avg:{:.0}bps", avg_bandwidth));
            }

            // Print interface results in bar format
            println!("{}: |{}|", interface, results.join("|"));

            // Delay between interfaces to stagger measurements
            std::thread::sleep(Duration::from_millis(200));
        }

        let _ = std::io::stdout().flush();

        // Sleep until next iteration or exit if Ctrl+C was pressed
        let start_sleep = Instant::now();
        while running.load(Ordering::SeqCst) {
            let elapsed = start_sleep.elapsed();
            if elapsed >= sleep_duration {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    println!("\nShutting down...");
}

fn resolve_server_address(server_str: &str) -> io::Result<SocketAddr> {
    // Append a default port if not specified, required by ToSocketAddrs
    let addr_with_port = if server_str.contains(':') {
        server_str.to_string()
    } else {
        format!("{}:443", server_str) // Default to port 443 for resolution
    };

    addr_with_port
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Could not resolve address"))
}

fn measure_throughput(interface: &str, addr: SocketAddr) -> io::Result<(Duration, u32)> {
    let domain = if addr.is_ipv4() {
        Domain::IPV4
    } else {
        Domain::IPV6
    };

    let socket = Socket::new(domain, Type::STREAM, None)?;

    // Bind the socket to the specified interface (Linux-only)
    if let Err(e) = bind_socket_to_interface(&socket, interface) {
        eprintln!(
            "Warning: Failed to bind to device '{}'. This might require root privileges. Error: {}",
            interface, e
        );
        // Continue without binding, the OS will choose the interface.
    }

    let start = Instant::now();
    socket.connect_timeout(&addr.into(), Duration::from_secs(5))?;
    let rtt = start.elapsed();

    let fd = socket.as_raw_fd();
    // On most platforms (including macOS and Linux), SO_RCVBUF is an int
    // https://man7.org/linux/man-pages/man7/socket.7.html
    let mut window_size: libc::c_int = 0;
    let mut optlen = std::mem::size_of::<libc::c_int>() as libc::socklen_t;

    let result = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_RCVBUF,
            &mut window_size as *mut _ as *mut libc::c_void,
            &mut optlen,
        )
    };

    if result != 0 {
        return Err(io::Error::last_os_error());
    }

    // Linux doubles the returned value for internal bookkeeping; other OSes generally do not.
    // Apply halving only on Linux to report the actual window size.
    #[cfg(target_os = "linux")]
    let actual_window_size = (window_size / 2) as u32;

    #[cfg(not(target_os = "linux"))]
    let actual_window_size = window_size as u32;

    Ok((rtt, actual_window_size))
}

#[cfg(target_os = "linux")]
fn bind_socket_to_interface(socket: &Socket, interface: &str) -> io::Result<()> {
    // Use libc directly to set SO_BINDTODEVICE, since socket2 may not expose bind_device on all versions.
    // Requires CAP_NET_RAW or root privileges on Linux.
    let fd = socket.as_raw_fd();
    let ifname = CString::new(interface)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Interface name contains NUL"))?;

    let ret = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_BINDTODEVICE,
            ifname.as_ptr() as *const libc::c_void,
            ifname.as_bytes_with_nul().len() as libc::socklen_t,
        )
    };

    if ret == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "linux"))]
fn bind_socket_to_interface(_socket: &Socket, interface: &str) -> io::Result<()> {
    // SO_BINDTODEVICE is not supported on non-Linux platforms.
    // We can print a warning to the user.
    // SO_BINDTODEVICE is not supported on non-Linux platforms.
    // Print a one-time warning to the user to avoid spamming in the loop.
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
            eprintln!(
                "Warning: Binding to a specific interface ('{}') is only supported on Linux. This option will be ignored.",
                interface
            );
        });
    Ok(())
}
