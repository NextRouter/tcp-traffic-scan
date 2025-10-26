#!/bin/bash
set -e

# Convert arguments like -s1 1.1.1.1 to -s 1.1.1.1
CONVERTED_ARGS=()
for arg in "$@"; do
    if [[ $arg == -s[0-9] ]]; then
        CONVERTED_ARGS+=("-s")
    else
        CONVERTED_ARGS+=("$arg")
    fi
done

# Change to the project directory
cd tcp-traffic-scan

# Run the application
cargo build

# Function to create systemd service
create_systemd_service() {
    local service_name="tcp-traffic-scan"
    local service_file="/etc/systemd/system/${service_name}.service"
    local current_dir=$(pwd)
    
    # Get arguments after --install
    shift # Remove --install
    local service_args="$*"
    
    sudo tee "$service_file" > /dev/null <<EOF
[Unit]
Description=TCP Traffic Scanner
After=network.target

[Service]
Type=simple
User=root
WorkingDirectory=${current_dir}/tcp-traffic-scan
ExecStart=${current_dir}/tcp-traffic-scan/target/debug/tcp-traffic-scan ${service_args}
Restart=always
RestartSec=10

[Install]
WantedBy=multi-user.target
EOF

    sudo systemctl daemon-reload
    sudo systemctl enable "$service_name"
    echo "Service created and enabled: $service_name with args: $service_args"
}

# Register with systemctl if --install flag is provided
if [[ "$1" == "--install" ]]; then
    create_systemd_service "$@"
    exit 0
fi

sudo ./target/debug/tcp-traffic-scan "${CONVERTED_ARGS[@]}"