#!/bin/bash

##############################################################################
# Run All NET (Network & Race Conditions) Tests
# 
# Purpose: Execute all network and race condition test cases sequentially
#          and generate a comprehensive summary report.
#
# Usage: ./run-all-net-tests.sh --email "user@example.com"
#
# Tests Executed:
#   SUB-01: [SETUP] Initial Subscription via verify_payment
#   NET-01: Webhook Arrives Before verify_payment
#   NET-02: verify_payment Call Fails / Network Timeout
#   NET-03: Webhook Processing Times Out
#   NET-04: Webhook Arrives While verify_payment In-Flight
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
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

# Defaults
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

# Validate required inputs
if [[ -z "$EMAIL" ]]; then
    echo -e "${RED}Error: --email is required${NC}"
    echo "Usage: ./run-all-net-tests.sh --email \"user@example.com\""
    exit 1
fi

echo -e "${CYAN}╔════════════════════════════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║        NET (Network & Race Conditions) Test Suite              ║${NC}"
echo -e "${CYAN}╚════════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo -e "${BLUE}Email: $EMAIL${NC}"
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
    
    if bash "$SCRIPT_DIR/$test_script" --email "$EMAIL"; then
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

# Run all NET tests
echo -e "${BLUE}Starting NET Test Suite...${NC}"
START_TIME=$(date +%s)

# SUB-01: [SETUP] Ensure subscription exists (not counted in NET totals)
echo -e "${CYAN}[SETUP] Running SUB-01 to ensure subscription exists (not counted in NET)...${NC}"
if bash "$SCRIPT_DIR/test-sub-01.sh" --email "$EMAIL"; then
    echo -e "${GREEN}✓ SUB-01 PASSED (setup)${NC}"
else
    echo -e "${RED}✗ SUB-01 FAILED (setup)${NC}"
fi
echo ""

# NET-01: Webhook Arrives Before verify_payment
run_test "NET-01" "test-net-01.sh" "Webhook Arrives Before verify_payment"

# NET-02: verify_payment Call Fails / Network Timeout
run_test "NET-02" "test-net-02.sh" "verify_payment Call Fails / Network Timeout"

# NET-03: Webhook Processing Times Out
run_test "NET-03" "test-net-03.sh" "Webhook Processing Times Out"

# NET-04: Webhook Arrives While verify_payment In-Flight
run_test "NET-04" "test-net-04.sh" "Webhook Arrives While verify_payment In-Flight"

END_TIME=$(date +%s)
DURATION=$((END_TIME - START_TIME))

# Calculate totals
TOTAL_TESTS=$((TESTS_PASSED + TESTS_FAILED))

# Generate summary
echo ""
echo -e "${CYAN}╔════════════════════════════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║                    NET Test Suite Summary                      ║${NC}"
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
    echo -e "${GREEN}║              ✓ ALL NET TESTS PASSED!                           ║${NC}"
    echo -e "${GREEN}╚════════════════════════════════════════════════════════════════╝${NC}"
else
    SUITE_STATUS="fail"
    echo -e "${RED}╔════════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${RED}║              ✗ SOME NET TESTS FAILED                           ║${NC}"
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
cat > net-suite-summary.json <<EOF
{
  "suite_id": "NET",
  "suite_name": "Network & Race Conditions Tests",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "duration_seconds": $DURATION,
  "email": "$EMAIL",
  "status": "$SUITE_STATUS",
  "summary": {
    "total": $TOTAL_TESTS,
    "passed": $TESTS_PASSED,
    "failed": $TESTS_FAILED
  },
  "tests": $JSON_RESULTS
}
EOF

echo "Summary report saved to: net-suite-summary.json"
echo ""
cat net-suite-summary.json
echo ""

# Exit with appropriate code
if [[ $TESTS_FAILED -gt 0 ]]; then
    exit 1
fi
exit 0
