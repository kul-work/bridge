#!/bin/bash

##############################################################################
# Run All ACC (Access Control & Entitlement) Tests
# 
# Purpose: Execute all access control test cases sequentially and generate
#          a comprehensive summary report.
#
# Usage: ./run-all-acc-tests.sh --email "user@example.com" [--email2 "other@example.com"]
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
#   - GET /api/v1/story endpoint must exist (premium feature test)
#
# Test Notes:
#   - ACC-01, ACC-02: Tests manipulate users.is_premium and users.premium_expires_at
#                     directly via SQL (no dependency on other tests)
#   - ACC-03: Requires /api/v1/verify-purchase endpoint
#
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

# Defaults
EMAIL=""
EMAIL2=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --email)
            EMAIL="$2"
            shift 2
            ;;
        --email2)
            EMAIL2="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Strip quotes from email addresses (in case they're passed with literal quotes)
EMAIL="${EMAIL%\"}"
EMAIL="${EMAIL#\"}"
EMAIL2="${EMAIL2%\"}"
EMAIL2="${EMAIL2#\"}"

# Validate required inputs
if [[ -z "$EMAIL" ]]; then
    echo -e "${RED}Error: --email is required${NC}"
    echo "Usage: ./run-all-acc-tests.sh --email \"user@example.com\" [--email2 \"other@example.com\"]"
    exit 1
fi

echo -e "${CYAN}╔════════════════════════════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║       ACC (Access Control & Entitlement) Test Suite            ║${NC}"
echo -e "${CYAN}╚════════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo -e "${BLUE}Email: $EMAIL${NC}"
if [[ ! -z "$EMAIL2" ]]; then
    echo -e "${BLUE}Email2: $EMAIL2${NC}"
fi
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
    local extra_args="${4:-}"
    
    echo ""
    echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${YELLOW}Running: $test_id - $test_name${NC}"
    echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""
    
    local cmd="bash \"$SCRIPT_DIR/$test_script\" --email \"$EMAIL\""
    if [[ ! -z "$extra_args" ]]; then
        cmd="$cmd $extra_args"
    fi
    
    if eval $cmd; then
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

# Run all ACC tests
echo -e "${BLUE}Starting ACC Test Suite...${NC}"
START_TIME=$(date +%s)

# ACC-01: Premium Access Granted for Allowed States
run_test "ACC-01" "test-acc-01.sh" "Premium Access Granted for Allowed States"

# ACC-02: Premium Access Revoked for Blocked States
run_test "ACC-02" "test-acc-02.sh" "Premium Access Revoked for Blocked States"

# ACC-03: Token Uniqueness & Fraud Prevention
ACC03_EXTRA=""
if [[ ! -z "$EMAIL2" ]]; then
    ACC03_EXTRA="--email2 \"$EMAIL2\""
fi
run_test "ACC-03" "test-acc-03.sh" "Token Uniqueness & Fraud Prevention" "$ACC03_EXTRA"

END_TIME=$(date +%s)
DURATION=$((END_TIME - START_TIME))

# Calculate totals
TOTAL_TESTS=$((TESTS_PASSED + TESTS_FAILED))

# Generate summary
echo ""
echo -e "${CYAN}╔════════════════════════════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║                    ACC Test Suite Summary                      ║${NC}"
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
    echo -e "${GREEN}║              ✓ ALL ACC TESTS PASSED!                           ║${NC}"
    echo -e "${GREEN}╚════════════════════════════════════════════════════════════════╝${NC}"
else
    SUITE_STATUS="fail"
    echo -e "${RED}╔════════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${RED}║              ✗ SOME ACC TESTS FAILED                           ║${NC}"
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
cat > acc-suite-summary.json <<EOF
{
  "suite_id": "ACC",
  "suite_name": "Access Control & Entitlement Tests",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "duration_seconds": $DURATION,
  "email": "$EMAIL",
  "email2": "$EMAIL2",
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

# Exit with appropriate code
if [[ $TESTS_FAILED -gt 0 ]]; then
    exit 1
fi
exit 0
