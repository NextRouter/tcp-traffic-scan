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
cd rtt-traffic-scan

# Run the application
cargo run -- "${CONVERTED_ARGS[@]}"
