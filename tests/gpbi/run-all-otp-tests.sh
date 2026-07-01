#!/bin/bash

##############################################################################
# Run All OTP Tests (OTP-01 to OTP-06, RTDN-01 to 04)
# 
# Executes complete OTP test suite with proper setup and cleanup.
#
# Usage: ./run-all-otp-tests.sh [--purge-reports] [--with-polling]
#
# Options:
#   --purge-reports       Delete reports after successful run (default: keep)
#   --with-polling        Validate OTP-04 wait path by approving it from a second process
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Test configuration
WITH_POLLING=false
PURGE_REPORTS=false

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --purge-reports)
            PURGE_REPORTS=true
            shift
            ;;
        --with-polling)
            WITH_POLLING=true
            shift
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

echo -e "${BLUE}================================================${NC}"
echo -e "${BLUE}OTP Test Suite Runner (OTP-01 to 06, RTDN-01 to 04)${NC}"
echo -e "${BLUE}================================================${NC}"
echo ""
echo "Configuration:"
echo "  User model: fixed external_user_id per OTP script"
echo "  Polling: $WITH_POLLING"
echo ""

# Test counters
PASSED=0
FAILED=0
TESTS_RUN=0
declare -a RESULTS

# Function to run a test
run_test() {
    local test_script=$1
    local test_name=$2
    local extra_args=${3:-""}
    
    echo -e "${YELLOW}-----------------------------------------${NC}"
    echo -e "${YELLOW}Running: $test_name${NC}"
    echo -e "${YELLOW}-----------------------------------------${NC}"
    echo ""
    
    # Derive test ID from filename (e.g., test-otp-01.sh -> OTP-01, test-otp-rtdn-01.sh -> OTP-RTDN-01)
    local test_id=$(echo "$test_script" | sed 's/test-//; s/\.sh//' | tr 'a-z' 'A-Z')
    
    TESTS_RUN=$((TESTS_RUN + 1))
    
    if ./"$test_script" $extra_args; then
        echo -e "${GREEN}PASS: $test_name${NC}"
        PASSED=$((PASSED + 1))
        RESULTS+=("{\"test_id\": \"$test_id\", \"test_name\": \"$test_name\", \"status\": \"pass\"}")
    else
        echo -e "${RED}FAIL: $test_name${NC}"
        FAILED=$((FAILED + 1))
        RESULTS+=("{\"test_id\": \"$test_id\", \"test_name\": \"$test_name\", \"status\": \"fail\"}")
    fi
    echo ""
}

run_otp04_with_polling() {
    local test_script="test-otp-04.sh"
    local test_name="OTP-04: Slow Card (Pending State)"
    local test_id="OTP-04"
    local wait_log="otp-04-wait.log"
    local approve_log="otp-04-approve.log"
    local wait_pid=""
    local status=""

    echo -e "${YELLOW}-----------------------------------------${NC}"
    echo -e "${YELLOW}Running: $test_name (--wait-for-approval + --approve)${NC}"
    echo -e "${YELLOW}-----------------------------------------${NC}"
    echo ""

    TESTS_RUN=$((TESTS_RUN + 1))
    rm -f "$wait_log" "$approve_log"

    ./"$test_script" --wait-for-approval > "$wait_log" 2>&1 &
    wait_pid=$!

    for _ in {1..30}; do
        if ! kill -0 "$wait_pid" 2>/dev/null; then
            echo -e "${RED}FAIL: OTP-04 waiter exited before pending state was ready${NC}"
            cat "$wait_log"
            FAILED=$((FAILED + 1))
            RESULTS+=("{\"test_id\": \"$test_id\", \"test_name\": \"$test_name\", \"status\": \"fail\"}")
            return
        fi

        status=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT status FROM pay.payments WHERE provider = '$PROVIDER' AND provider_transaction_id = 'test-inapp-slow-4567' AND product_id = '$PRODUCT_ID_OTP' ORDER BY created_at DESC LIMIT 1;" -t 2>/dev/null | xargs)
        if [[ "$status" == "pending" ]]; then
            break
        fi
        sleep 1
    done

    if [[ "$status" != "pending" ]]; then
        echo -e "${RED}FAIL: OTP-04 pending payment was not created in time${NC}"
        kill "$wait_pid" 2>/dev/null || true
        wait "$wait_pid" 2>/dev/null || true
        cat "$wait_log"
        FAILED=$((FAILED + 1))
        RESULTS+=("{\"test_id\": \"$test_id\", \"test_name\": \"$test_name\", \"status\": \"fail\"}")
        return
    fi

    if ! ./"$test_script" --approve > "$approve_log" 2>&1; then
        echo -e "${RED}FAIL: OTP-04 approval injection failed${NC}"
        kill "$wait_pid" 2>/dev/null || true
        wait "$wait_pid" 2>/dev/null || true
        cat "$approve_log"
        FAILED=$((FAILED + 1))
        RESULTS+=("{\"test_id\": \"$test_id\", \"test_name\": \"$test_name\", \"status\": \"fail\"}")
        return
    fi

    if wait "$wait_pid"; then
        echo -e "${GREEN}PASS: $test_name${NC}"
        PASSED=$((PASSED + 1))
        RESULTS+=("{\"test_id\": \"$test_id\", \"test_name\": \"$test_name\", \"status\": \"pass\"}")
    else
        echo -e "${RED}FAIL: $test_name${NC}"
        echo "--- wait log ---"
        cat "$wait_log"
        echo "--- approve log ---"
        cat "$approve_log"
        FAILED=$((FAILED + 1))
        RESULTS+=("{\"test_id\": \"$test_id\", \"test_name\": \"$test_name\", \"status\": \"fail\"}")
    fi
    echo ""
}

# Check if scripts are executable
for script in test-otp-{01..05}.sh test-otp-06-missing-price-ack-row.sh test-otp-rtdn-{01..04}.sh; do
    if [[ ! -f "$script" ]]; then
        echo -e "${RED}Error: $script not found${NC}"
        exit 1
    fi
    if [[ ! -x "$script" ]]; then
        chmod +x "$script"
    fi
done

echo -e "${YELLOW}-----------------------------------------${NC}"
echo -e "${YELLOW}Stateless Tests (No DB Dependencies)${NC}"
echo -e "${YELLOW}-----------------------------------------${NC}"
echo ""

# OTP-02: Declined Payment (no DB changes)
run_test "test-otp-02.sh" "OTP-02: Declined Payment"

# OTP-03: User Cancellation (no DB changes)
run_test "test-otp-03.sh" "OTP-03: User Cancellation"

# OTP-04: Slow Card (Pending State) - cleans up internally
if [[ "$WITH_POLLING" == "true" ]]; then
    run_otp04_with_polling
else
    run_test "test-otp-04.sh" "OTP-04: Slow Card (Pending State)"
fi



echo -e "${YELLOW}-----------------------------------------${NC}"
echo -e "${YELLOW}Purchase Tests (With DB Dependencies)${NC}"
echo -e "${YELLOW}-----------------------------------------${NC}"
echo ""

# OTP-01: Successful Purchase
run_test "test-otp-01.sh" "OTP-01: Successful Purchase"

# OTP-05: Refund After Purchase
run_test "test-otp-05.sh" "OTP-05: Refund After Purchase"

# OTP-06: Missing Price ACK Row
run_test "test-otp-06-missing-price-ack-row.sh" "OTP-06: Missing Price ACK Row"

echo ""

# OTP-RTDN-01: Webhook Purchase Completed
run_test "test-otp-rtdn-01.sh" "OTP-RTDN-01: Webhook Purchase Completed"

# OTP-RTDN-02: Webhook Refund Completed
run_test "test-otp-rtdn-02.sh" "OTP-RTDN-02: Webhook Refund Completed"

# Reset status to 'success' for new RTDN types
run_test "test-otp-01.sh" "OTP-01: Reset for RTDN-03"

# OTP-RTDN-03: Webhook OTP Refunded (Type 2)
run_test "test-otp-rtdn-03.sh" "OTP-RTDN-03: Webhook OTP Refunded (Type 2)"

# Reset status to 'success' for new RTDN types
run_test "test-otp-01.sh" "OTP-01: Reset for RTDN-04"

# OTP-RTDN-04: Webhook OTP Canceled (Type 14)
run_test "test-otp-rtdn-04.sh" "OTP-RTDN-04: Webhook OTP Canceled (Type 14)"

# Summary
echo ""
echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}Test Suite Summary${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""
echo "Total Tests Run: $TESTS_RUN"
echo -e "${GREEN}Passed: $PASSED${NC}"
if [[ $FAILED -gt 0 ]]; then
    echo -e "${RED}Failed: $FAILED${NC}"
else
    echo -e "Failed: $FAILED"
fi
echo ""

# Build JSON results array
JSON_RESULTS="["
for i in "${!RESULTS[@]}"; do
    if [[ $i -gt 0 ]]; then
        JSON_RESULTS+=", "
    fi
    JSON_RESULTS+="${RESULTS[$i]}"
done
JSON_RESULTS+="]"

# Generate summary report
cat > otp-suite-summary.json <<EOF
{
  "test_suite": "OTP-01 to OTP-06 + RTDN-01 to 04",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "total_tests": $TESTS_RUN,
  "passed": $PASSED,
  "failed": $FAILED,
  "polling_enabled": $WITH_POLLING,
  "status": $([ $FAILED -eq 0 ] && echo "\"all_passed\"" || echo "\"some_failed\""),
  "tests": $JSON_RESULTS
}
EOF

echo "Summary report saved to: otp-suite-summary.json"
cat otp-suite-summary.json
echo ""

# Exit with appropriate code
if [[ $FAILED -gt 0 ]]; then
    echo -e "${RED}Some tests FAILED${NC}"
    exit 1
else
    echo -e "${GREEN}All tests PASSED${NC}"
    if [[ "$PURGE_REPORTS" == "true" ]]; then
        echo ""
        echo -e "${BLUE}Purging reports (--purge-reports)...${NC}"
        if bash cleanup-all-otp.sh; then
            echo -e "${GREEN}Reports purged successfully${NC}"
        else
            echo -e "${RED}Purge script failed${NC}"
            exit 1
        fi
    fi
    exit 0
fi
