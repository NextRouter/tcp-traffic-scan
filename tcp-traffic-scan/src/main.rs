use clap::Parser;
use libc;
use socket2::{Domain, Socket, Type};
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
    Arc,
};
use std::time::{Duration, Instant};

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

    // Main loop until Ctrl+C
    let sleep_duration = Duration::from_secs_f64(1.0);
    while running.load(Ordering::SeqCst) {
        println!("==================================");

        for interface in &args.interface {
            let mut results = Vec::new();

            for server_str in &args.server {
                match resolve_server_address(server_str) {
                    Ok(server_addr) => match measure_throughput(interface, server_addr) {
                        Ok((rtt, window_size)) => {
                            let throughput_bps = if rtt.as_secs_f64() > 0.0 {
                                (window_size as f64 * 8.0) / rtt.as_secs_f64()
                            } else {
                                0.0
                            };
                            let throughput_mbps = throughput_bps / 1_000_000.0;
                            results.push(format!(
                                "{}:{:.0}Mbps",
                                server_addr.ip(),
                                throughput_mbps
                            ));
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
