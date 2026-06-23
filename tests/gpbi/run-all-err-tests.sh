#!/bin/bash

##############################################################################
# Run All ERR (Error & Edge Cases) Tests
# 
# Purpose: Execute all error and edge case test cases sequentially
#          and generate a comprehensive summary report.
#
# Usage: ./run-all-err-tests.sh
#
# Tests Executed:
#   ERR-01: Invalid Purchase Token Format
#   ERR-02: Subscription ID Mismatch
#   ERR-03: Expired Purchase Token
#   ERR-04: Revoked/Refunded Purchase Token
#   ERR-05: Google API Temporarily Unavailable
#   ERR-06: Webhook Payload Malformed
#   ERR-07: Unknown Notification Type
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=false
#   - DATABASE_URL configured and db accessible
#   - psql installed and in PATH
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
CYAN='\033[0;36m'
NC='\033[0m' # No Color

echo -e "${CYAN}╔════════════════════════════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║            ERR (Error & Edge Cases) Test Suite                 ║${NC}"
echo -e "${CYAN}╚════════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo -e "${BLUE}Time:  $(date -u +%Y-%m-%dT%H:%M:%SZ)${NC}"
echo "" 

# Test tracking
TESTS_PASSED=0
TESTS_FAILED=0
RESULTS=()

# Function to run a test
run_test() {
    local test_id="$1"
    local test_script="$2"
    local test_name="$3"
    
    echo ""
    echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${YELLOW}Running: $test_id - $test_name${NC}"
    echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""
    
    if bash "$SCRIPT_DIR/$test_script"; then
        TESTS_PASSED=$((TESTS_PASSED + 1))
        RESULTS+=("{\"test_id\": \"$test_id\", \"test_name\": \"$test_name\", \"status\": \"pass\"}")
        echo ""
        echo -e "${GREEN}✓ $test_id PASSED${NC}"
    else
        EXIT_CODE=$?
        if [[ $EXIT_CODE -eq 0 ]]; then
            TESTS_PASSED=$((TESTS_PASSED + 1))
            RESULTS+=("{\"test_id\": \"$test_id\", \"test_name\": \"$test_name\", \"status\": \"pass\"}")
            echo ""
            echo -e "${GREEN}✓ $test_id PASSED${NC}"
        else
            TESTS_FAILED=$((TESTS_FAILED + 1))
            RESULTS+=("{\"test_id\": \"$test_id\", \"test_name\": \"$test_name\", \"status\": \"fail\"}")
            echo ""
            echo -e "${RED}✗ $test_id FAILED${NC}"
        fi
    fi
}

# Run all ERR tests
echo -e "${BLUE}Starting ERR Test Suite...${NC}"
START_TIME=$(date +%s)

# ERR-01: Invalid Purchase Token Format
run_test "ERR-01" "test-err-01.sh" "Invalid Purchase Token Format"

# ERR-02: Subscription ID Mismatch
run_test "ERR-02" "test-err-02.sh" "Subscription ID Mismatch"

# ERR-03: Expired Purchase Token
run_test "ERR-03" "test-err-03.sh" "Expired Purchase Token"

# ERR-04: Revoked/Refunded Purchase Token
run_test "ERR-04" "test-err-04.sh" "Revoked/Refunded Purchase Token"

# ERR-05: Google API Temporarily Unavailable
run_test "ERR-05" "test-err-05.sh" "Google API Temporarily Unavailable"

# ERR-06: Webhook Payload Malformed
run_test "ERR-06" "test-err-06.sh" "Webhook Payload Malformed"

# ERR-07: Unknown Notification Type
run_test "ERR-07" "test-err-07.sh" "Unknown Notification Type"

END_TIME=$(date +%s)
DURATION=$((END_TIME - START_TIME))

# Calculate totals
TOTAL_TESTS=$((TESTS_PASSED + TESTS_FAILED))

# Generate summary
echo ""
echo -e "${CYAN}╔════════════════════════════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║                    ERR Test Suite Summary                      ║${NC}"
echo -e "${CYAN}╚════════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo -e "${BLUE}Duration: ${DURATION}s${NC}"
echo ""
echo -e "Total Tests: $TOTAL_TESTS"
echo -e "${GREEN}Passed: $TESTS_PASSED${NC}"
if [[ $TESTS_FAILED -gt 0 ]]; then
    echo -e "${RED}Failed: $TESTS_FAILED${NC}"
else
    echo -e "Failed: $TESTS_FAILED"
fi
echo ""

# Overall status
if [[ $TESTS_FAILED -eq 0 ]]; then
    SUITE_STATUS="pass"
    echo -e "${GREEN}╔════════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${GREEN}║              ✓ ALL ERR TESTS PASSED!                           ║${NC}"
    echo -e "${GREEN}╚════════════════════════════════════════════════════════════════╝${NC}"
else
    SUITE_STATUS="fail"
    echo -e "${RED}╔════════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${RED}║              ✗ SOME ERR TESTS FAILED                           ║${NC}"
    echo -e "${RED}╚════════════════════════════════════════════════════════════════╝${NC}"
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

# Generate JSON summary report
cat > err-suite-summary.json <<EOF
{
  "suite_id": "ERR",
  "suite_name": "Error & Edge Cases Tests",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "duration_seconds": $DURATION,
  "status": "$SUITE_STATUS",
  "summary": {
    "total": $TOTAL_TESTS,
    "passed": $TESTS_PASSED,
    "failed": $TESTS_FAILED
  },
  "tests": $JSON_RESULTS
}
EOF

echo "Summary report saved to: err-suite-summary.json"
echo ""
cat err-suite-summary.json
echo ""

# Exit with appropriate code
if [[ $TESTS_FAILED -gt 0 ]]; then
    exit 1
fi
exit 0
