#!/bin/bash

##############################################################################
# Run All OTP Tests (OTP-01 to OTP-05)
# 
# Executes complete OTP test suite with proper setup and cleanup.
#
# Usage: ./run-all-otp-tests.sh [--purge-reports]
#
# Options:
#   --purge-reports       Delete reports after successful run (default: keep)
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
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

echo -e "${BLUE}╔════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║  OTP Test Suite Runner (OTP-01 to 05, RTDN)    ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════╝${NC}"
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
    
    echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${YELLOW}Running: $test_name${NC}"
    echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""
    
    # Derive test ID from filename (e.g., test-otp-01.sh -> OTP-01, test-otp-rtdn-01.sh -> OTP-RTDN-01)
    local test_id=$(echo "$test_script" | sed 's/test-//; s/\.sh//' | tr 'a-z' 'A-Z')
    
    TESTS_RUN=$((TESTS_RUN + 1))
    
    if ./"$test_script" $extra_args; then
        echo -e "${GREEN}✓ $test_name PASSED${NC}"
        PASSED=$((PASSED + 1))
        RESULTS+=("{\"test_id\": \"$test_id\", \"test_name\": \"$test_name\", \"status\": \"pass\"}")
    else
        echo -e "${RED}✗ $test_name FAILED${NC}"
        FAILED=$((FAILED + 1))
        RESULTS+=("{\"test_id\": \"$test_id\", \"test_name\": \"$test_name\", \"status\": \"fail\"}")
    fi
    echo ""
}

# Check if scripts are executable
for script in test-otp-{01..05}.sh test-otp-rtdn-{01..02}.sh; do
    if [[ ! -f "$script" ]]; then
        echo -e "${RED}Error: $script not found${NC}"
        exit 1
    fi
    if [[ ! -x "$script" ]]; then
        chmod +x "$script"
    fi
done

echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${YELLOW}Stateless Tests (No DB Dependencies)${NC}"
echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo ""

# OTP-02: Declined Payment (no DB changes)
run_test "test-otp-02.sh" "OTP-02: Declined Payment"

# OTP-03: User Cancellation (no DB changes)
run_test "test-otp-03.sh" "OTP-03: User Cancellation"

# OTP-04: Slow Card (Pending State) - cleans up internally
if [[ "$WITH_POLLING" == "true" ]]; then
    run_test "test-otp-04.sh" "OTP-04: Slow Card (Pending State)" "--wait-for-approval"
else
    run_test "test-otp-04.sh" "OTP-04: Slow Card (Pending State)"
fi



echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${YELLOW}Purchase Tests (With DB Dependencies)${NC}"
echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo ""

# OTP-01: Successful Purchase
run_test "test-otp-01.sh" "OTP-01: Successful Purchase"

# OTP-05: Refund After Purchase
run_test "test-otp-05.sh" "OTP-05: Refund After Purchase"

echo ""

# OTP-RTDN-01: Webhook Purchase Completed
run_test "test-otp-rtdn-01.sh" "OTP-RTDN-01: Webhook Purchase Completed"

# OTP-RTDN-02: Webhook Refund Completed
run_test "test-otp-rtdn-02.sh" "OTP-RTDN-02: Webhook Refund Completed"

# Summary
echo ""
echo -e "${BLUE}╔════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║         Test Suite Summary             ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════╝${NC}"
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
  "test_suite": "OTP-01 to OTP-05 + RTDN-01/02",
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
