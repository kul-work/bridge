#!/bin/bash

##############################################################################
# Run All ACC (Access Control & Entitlement) Tests
#
# Purpose: Execute all access control test cases sequentially and generate
#          a comprehensive summary report.
#
# Usage: ./run-all-acc-tests.sh
#
# Tests Executed:
#   ACC-01: Premium Access Granted for Allowed States
#   ACC-02: Premium Access Revoked for Blocked States
#   ACC-03: Token Uniqueness & Fraud Prevention
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - DATABASE_URL configured and db accessible
#   - psql installed and in PATH
#   - GET /api/v1/subscriptions endpoint must exist (premium feature test)
#
# Test Notes:
#   - ACC-01, ACC-02, ACC-03 use generated external_user_id values and do not
#     depend on pre-created account fixtures
#   - ACC-03: Requires /api/v1/verify-purchase endpoint
##############################################################################

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

while [[ $# -gt 0 ]]; do
    case $1 in
        *)
            echo "Unknown option: $1"
            echo "Usage: ./run-all-acc-tests.sh"
            exit 1
            ;;
    esac
done

echo -e "${CYAN}============================================================${NC}"
echo -e "${CYAN}ACC (Access Control & Entitlement) Test Suite${NC}"
echo -e "${CYAN}============================================================${NC}"
echo -e "${BLUE}Time: $(date -u +%Y-%m-%dT%H:%M:%SZ)${NC}"
echo ""

TESTS_PASSED=0
TESTS_FAILED=0
RESULTS=()

run_test() {
    local test_id="$1"
    local test_script="$2"
    local test_name="$3"

    echo -e "${YELLOW}------------------------------------------------------------${NC}"
    echo -e "${YELLOW}Running: $test_id - $test_name${NC}"
    echo -e "${YELLOW}------------------------------------------------------------${NC}"

    if bash "$SCRIPT_DIR/$test_script"; then
        TESTS_PASSED=$((TESTS_PASSED + 1))
        RESULTS+=("{\"test_id\": \"$test_id\", \"test_name\": \"$test_name\", \"status\": \"pass\"}")
        echo -e "${GREEN}$test_id PASSED${NC}"
    else
        TESTS_FAILED=$((TESTS_FAILED + 1))
        RESULTS+=("{\"test_id\": \"$test_id\", \"test_name\": \"$test_name\", \"status\": \"fail\"}")
        echo -e "${RED}$test_id FAILED${NC}"
    fi
    echo ""
}

echo -e "${BLUE}Starting ACC Test Suite...${NC}"
START_TIME=$(date +%s)

run_test "ACC-01" "test-acc-01.sh" "Premium Access Granted for Allowed States"
run_test "ACC-02" "test-acc-02.sh" "Premium Access Revoked for Blocked States"
run_test "ACC-03" "test-acc-03.sh" "Token Uniqueness & Fraud Prevention"

END_TIME=$(date +%s)
DURATION=$((END_TIME - START_TIME))
TOTAL_TESTS=$((TESTS_PASSED + TESTS_FAILED))

echo -e "${CYAN}============================================================${NC}"
echo -e "${CYAN}ACC Test Suite Summary${NC}"
echo -e "${CYAN}============================================================${NC}"
echo -e "${BLUE}Duration: ${DURATION}s${NC}"
echo "Total Tests: $TOTAL_TESTS"
echo -e "${GREEN}Passed: $TESTS_PASSED${NC}"
if [[ $TESTS_FAILED -gt 0 ]]; then
    echo -e "${RED}Failed: $TESTS_FAILED${NC}"
    SUITE_STATUS="fail"
else
    echo "Failed: $TESTS_FAILED"
    SUITE_STATUS="pass"
fi
echo ""

JSON_RESULTS="["
for i in "${!RESULTS[@]}"; do
    if [[ $i -gt 0 ]]; then
        JSON_RESULTS+=", "
    fi
    JSON_RESULTS+="${RESULTS[$i]}"
done
JSON_RESULTS+="]"

cat > acc-suite-summary.json <<EOF
{
  "suite_id": "ACC",
  "suite_name": "Access Control & Entitlement Tests",
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

echo "Summary report saved to: acc-suite-summary.json"
echo ""
cat acc-suite-summary.json
echo ""

if [[ $TESTS_FAILED -gt 0 ]]; then
    exit 1
fi
exit 0
