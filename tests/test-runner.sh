#!/bin/bash

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

for test_script in "$SCRIPT_DIR"/*.sh; do
    if [ "$(basename "$test_script")" = "test-runner.sh" ]; then
        continue
    fi

    echo "=== Running $(basename "$test_script") ==="
    bash "$test_script"
    echo ""
done
