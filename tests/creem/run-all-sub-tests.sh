#!/bin/bash

##############################################################################
# CBI Subscription Test Suite (B1-B5)
# 
# Purpose: Orchestrates the execution of all Creem Subscription (SUB) 
#          test scenarios (SUB-01 through SUB-05).
#
# Usage: ./run-all-sub-tests.sh --email "user@example.com"
#
# Scopes:
#   SUB-01: Initial Active Subscription
#   SUB-02: Subscription Renewal
#   SUB-03: User Cancellation
#   SUB-04: Grace Period (Past Due)
#   SUB-05: Expiration (Inactive)
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
GRAY='\033[0;90m'
NC='\033[0m' # No Color

EMAIL=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --email)
            EMAIL="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done



echo -e "${BLUE}╔════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║       Creem Subscription Suite (CBI)           ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════╝${NC}"
echo ""

# Test counters
PASSED=0
FAILED=0
TESTS_RUN=0
declare -a RESULTS

run_test() {
    local script=$1
    local name=$2
    
    echo -e "${YELLOW}Running: $name${NC}"
    
    # Derive test ID from filename
    local test_id=$(echo "$script" | sed 's/test-//; s/\.sh//' | tr 'a-z' 'A-Z')
    
    TESTS_RUN=$((TESTS_RUN + 1))
    
    if bash "$script" --email "$EMAIL"; then
        echo -e "${GREEN}✓ $name PASSED${NC}"
        PASSED=$((PASSED + 1))
        RESULTS+=("{\"test_id\": \"$test_id\", \"test_name\": \"$name\", \"status\": \"pass\"}")
    else
        echo -e "${RED}✗ $name FAILED${NC}"
        FAILED=$((FAILED + 1))
        RESULTS+=("{\"test_id\": \"$test_id\", \"test_name\": \"$name\", \"status\": \"fail\"}")
    fi
    echo ""
}

# Ensure scripts are executable
chmod +x test-sub-*.sh cleanup-all-sub.sh

# Run Tests
run_test "test-sub-01.sh" "SUB-01: Initial Subscription (Active)"
run_test "test-sub-02.sh" "SUB-02: Subscription Renewal"
run_test "test-sub-03.sh" "SUB-03: User-Initiated Cancellation"
run_test "test-sub-04.sh" "SUB-04: Grace Period (Past Due)"
run_test "test-sub-05.sh" "SUB-05: Subscription Expiration"
run_test "test-sub-06.sh" "SUB-06: Scheduled Cancellation"
run_test "test-sub-07.sh" "SUB-07: Immediate Cancellation"
run_test "test-sub-08.sh" "SUB-08: Resume Scheduled Cancellation"
run_test "test-sub-09.sh" "SUB-09: Plan Upgrade/Downgrade"
run_test "test-sub-10.sh" "SUB-10: Admin Pause"
run_test "test-sub-11.sh" "SUB-11: Incomplete Checkout — 3DS Completed"
run_test "test-sub-12.sh" "SUB-12: Subscription Payment Refunded"
run_test "test-sub-13.sh" "SUB-13: Payment Recovery from Past Due"
run_test "test-sub-14.sh" "SUB-14: Scheduled Cancellation Period Ends (Expiry)"
run_test "test-sub-15.sh" "SUB-15: Admin Resumes Paused Subscription"

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
cat > sub-suite-summary.json <<EOF
{
  "test_suite": "CBI SUB Suite",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "email": "$EMAIL",
  "total_tests": $TESTS_RUN,
  "passed": $PASSED,
  "failed": $FAILED,
  "status": $([ $FAILED -eq 0 ] && echo "\"all_passed\"" || echo "\"some_failed\""),
  "tests": $JSON_RESULTS
}
EOF

# Summary
if [[ $FAILED -eq 0 ]]; then
    echo -e "${BLUE}Summary: $PASSED passed, ${GRAY}$FAILED${BLUE} failed, $TESTS_RUN total${NC}"
else
    echo -e "${BLUE}Summary: $PASSED passed, ${RED}$FAILED${BLUE} failed, $TESTS_RUN total${NC}"
fi

if [[ $FAILED -eq 0 ]]; then
    echo -e "${GREEN}All Creem Subscription tests PASSED${NC}"
    exit 0
else
    echo -e "${RED}Some Creem Subscription tests FAILED${NC}"
    exit 1
fi
