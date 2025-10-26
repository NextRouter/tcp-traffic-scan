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

# Create systemd service file
echo "Creating systemd service..."
SERVICE_FILE="/etc/systemd/system/tcp-traffic-scan.service"
CURRENT_DIR=$(pwd)/tcp-traffic-scan
BINARY_PATH="$CURRENT_DIR/target/debug/tcp-traffic-scan ${CONVERTED_ARGS[@]}"

# Check if binary exists
if [ ! -f "$BINARY_PATH" ]; then
    echo "Error: Binary not found at $BINARY_PATH"
    exit 1
fi

tee $SERVICE_FILE > /dev/null << EOF
[Unit]
Description=TCP Traffic Scanner
After=network.target

[Service]
Type=simple
User=root
ExecStart=$BINARY_PATH
WorkingDirectory=$CURRENT_DIR
Restart=always
RestartSec=10

[Install]
WantedBy=multi-user.target
EOF

# Reload systemd and enable the service
sudo systemctl daemon-reload
sudo systemctl enable tcp-traffic-scan.service

echo "Service created and enabled. You can start it with:"
echo "sudo systemctl start tcp-traffic-scan.service"