#!/bin/bash

##############################################################################
# API & Notification Test Suite Runner
#
# Runs all API and Notification tests:
#   API-01: Rate Limit Headers
#   NOTIF-01: Payment Failure & Acknowledgment
#   NOTIF-02: Notification History
#
# Usage: ./run-all-api-tests.sh
##############################################################################

set -e

# Source configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# Defaults

echo -e "${BLUE}=================================================${NC}"
echo -e "${BLUE}       Running API & Notification Test Suite     ${NC}"
echo -e "${BLUE}=================================================${NC}"
echo ""

FAILED_TESTS=()
declare -A TEST_RESULTS

run_test() {
    local test_id=$1
    local script_name=$2
    local description=$3
    
    echo -e "${YELLOW}Running $test_id: $description...${NC}"
    if ./$script_name; then
        echo -e "${GREEN}✓ $test_id PASSED${NC}"
        TEST_RESULTS[$test_id]="pass"
        echo ""
    else
        echo -e "${RED}✗ $test_id FAILED${NC}"
        FAILED_TESTS+=("$test_id")
        TEST_RESULTS[$test_id]="fail"
        echo ""
        # Continue to next test despite failure
    fi
}

# API-01: Rate Limit Headers
run_test "API-01" "test-api-01.sh" "Rate Limit Headers"

# NOTIF-01: Payment Failure & Acknowledgment
run_test "NOTIF-01" "test-notif-01.sh" "Payment Failure & Acknowledgment"

# NOTIF-02: Notification History
run_test "NOTIF-02" "test-notif-02.sh" "Notification History"

echo -e "${BLUE}============================================================${NC}"
echo -e "${BLUE}                 Test Suite Summary                         ${NC}"
echo -e "${BLUE}============================================================${NC}"

# Count results
PASSED_COUNT=$((3 - ${#FAILED_TESTS[@]}))

# Generate JSON summary for master test runner
cat > api-suite-summary.json <<EOF
{
  "suite": "API & Notifications (3 Tests)",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "total": 3,
  "passed": $PASSED_COUNT,
  "failed": ${#FAILED_TESTS[@]},
  "suites": {
    "api_notifications": {
      "API-01": "${TEST_RESULTS[API-01]:-unknown}",
      "NOTIF-01": "${TEST_RESULTS[NOTIF-01]:-unknown}",
      "NOTIF-02": "${TEST_RESULTS[NOTIF-02]:-unknown}"
    }
  }
}
EOF

echo "Summary saved to: api-suite-summary.json"

if [ ${#FAILED_TESTS[@]} -eq 0 ]; then
    echo -e "${GREEN}All API/Notification tests passed successfully!${NC}"
    exit 0
else
    echo -e "${RED}The following tests failed:${NC}"
    for test in "${FAILED_TESTS[@]}"; do
        echo -e "${RED}  - $test${NC}"
    done
    exit 1
fi
