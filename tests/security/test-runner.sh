#!/bin/bash

##############################################################################
# Bridge Security Test Runner
#
# Purpose: Single entry point for Bridge cross-app tenant isolation security
#          tests. Runs the selected shell scripts against an already-running
#          Bridge backend, prints a suite header/summary, and writes a JSON
#          suite report.
#
# Kind: Shell security test runner for a running backend, not an in-process
#       Rust integration test.
#
# Usage: ./test-runner.sh [--scope SCOPE]
# Scopes:
#   full  - Run all security tests. Default.
#   tenant- Run cross-app tenant isolation tests.
#
# Prerequisites:
#   - Bridge backend is already running.
#   - curl, jq, psql, and bash are installed and in PATH.
#   - tests/security/globals.cfg is configured, optionally via
#     tests/security/.env (must contain BRIDGE_APP_A_API_KEY and
#     BRIDGE_APP_B_API_KEY).
##############################################################################

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[1;34m'
CYAN='\033[0;36m'
NC='\033[0m'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCOPE="full"
SUMMARY_FILE="$SCRIPT_DIR/security-suite-summary.json"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --scope)
            if [[ $# -lt 2 ]]; then
                echo -e "${RED}--scope requires a value${NC}"
                exit 1
            fi
            SCOPE="$2"
            shift 2
            ;;
        -h|--help)
            sed -n '2,18p' "$0"
            exit 0
            ;;
        *)
            echo -e "${RED}Unknown option: $1${NC}"
            exit 1
            ;;
    esac
done

cd "$SCRIPT_DIR"

if [[ ! -f "globals.cfg" ]]; then
    echo -e "${RED}Error: globals.cfg not found in $SCRIPT_DIR${NC}"
    exit 1
fi

source "globals.cfg"

BRIDGE_API_URL="${BRIDGE_API_URL%/}"

echo "Waiting for Bridge backend to be ready at $BRIDGE_API_URL..."
READY=0
for i in {1..15}; do
    if curl -s "$BRIDGE_API_URL/health" >/dev/null; then
        echo "Bridge ready at $BRIDGE_API_URL."
        READY=1
        break
    fi
    sleep 1
done
if [[ "$READY" -ne 1 ]]; then
    echo -e "${RED}Bridge not reachable at $BRIDGE_API_URL${NC}"
    exit 1
fi

declare -a TESTS=()
case "$SCOPE" in
    full|tenant)
        TESTS=(
            "CROSS-APP-READ|test-cross-app-read.sh|Cross-App Tenant Isolation"
        )
        ;;
    *)
        echo -e "${RED}Error: invalid scope '$SCOPE'. Use full or tenant.${NC}"
        exit 1
        ;;
esac

echo -e "${CYAN}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║            Bridge Security Test Runner                     ║${NC}"
echo -e "${CYAN}╚════════════════════════════════════════════════════════════╝${NC}"
echo "Scope: $SCOPE"
echo "API URL: $BRIDGE_API_URL"
echo ""

STARTED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
START_SECONDS=$(date +%s)
PASSED=0
FAILED=0
RESULT_ROWS=()
FAILED_IDS=()

run_test() {
    local id="$1"
    local script="$2"
    local name="$3"
    local test_start test_end duration status

    if [[ ! -f "$script" ]]; then
        echo -e "${RED}✗ Missing security script: $script${NC}"
        FAILED=$((FAILED + 1))
        FAILED_IDS+=("$id")
        RESULT_ROWS+=("{\"test_id\":\"$id\",\"script\":\"$script\",\"name\":\"$name\",\"status\":\"missing\",\"duration_seconds\":0}")
        return
    fi

    echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${BLUE}▶ Running $id: $name${NC}"
    echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

    test_start=$(date +%s)
    if bash "$script"; then
        status="pass"
        PASSED=$((PASSED + 1))
        echo -e "${GREEN}✓ $id PASSED${NC}"
    else
        status="fail"
        FAILED=$((FAILED + 1))
        FAILED_IDS+=("$id")
        echo -e "${RED}✗ $id FAILED${NC}"
    fi
    test_end=$(date +%s)
    duration=$((test_end - test_start))
    RESULT_ROWS+=("{\"test_id\":\"$id\",\"script\":\"$script\",\"name\":\"$name\",\"status\":\"$status\",\"duration_seconds\":$duration}")
}

for test_entry in "${TESTS[@]}"; do
    IFS='|' read -r id script name <<< "$test_entry"
    run_test "$id" "$script" "$name"
done

END_SECONDS=$(date +%s)
TOTAL_DURATION=$((END_SECONDS - START_SECONDS))
FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)

echo ""
echo -e "${CYAN}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║                      Suite Summary                         ║${NC}"
echo -e "${CYAN}╠════════════════════════════════════════════════════════════╣${NC}"
printf  "${CYAN}║${NC}  Passed: %-50s${CYAN}║${NC}\n" "$PASSED"
printf  "${CYAN}║${NC}  Failed: %-50s${CYAN}║${NC}\n" "$FAILED"
printf  "${CYAN}║${NC}  Duration: %-48s${CYAN}║${NC}\n" "${TOTAL_DURATION}s"
echo -e "${CYAN}╚════════════════════════════════════════════════════════════╝${NC}"

if [[ "$FAILED" -gt 0 ]]; then
    echo -e "${RED}Failed tests: ${FAILED_IDS[*]}${NC}"
fi

RESULTS_JOINED=$(printf ",%s" "${RESULT_ROWS[@]}")
RESULTS_JOINED="${RESULTS_JOINED:1}"

cat > "$SUMMARY_FILE" <<EOF
{
  "suite": "bridge-security",
  "scope": "$SCOPE",
  "started_at": "$STARTED_AT",
  "finished_at": "$FINISHED_AT",
  "duration_seconds": $TOTAL_DURATION,
  "passed": $PASSED,
  "failed": $FAILED,
  "tests": [$RESULTS_JOINED]
}
EOF

echo "Suite report saved to: $SUMMARY_FILE"

if [[ "$FAILED" -gt 0 ]]; then
    exit 1
fi
