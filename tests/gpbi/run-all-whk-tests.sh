#!/bin/bash

##############################################################################
# Run All WHK (Webhook Integrity) Tests
# 
# Purpose: Execute all webhook integrity test cases sequentially and generate
#          a comprehensive summary report.
#
# Usage: ./run-all-whk-tests.sh --email "user@example.com"
#
# Tests Executed:
#   SUB-01:  [SETUP] Initial Subscription via verify_payment
#   WHK-01:  Invalid Pub/Sub Signature Rejection
#   WHK-01B: Audience Claim Mismatch Rejection
#   WHK-01C: Audience Validation With Correct Audience
#   WHK-01D: Audience Validation Disabled (Dev Mode)
#   WHK-02:  Duplicate Webhook Delivery (Idempotency)
#   WHK-03:  Out-of-Order Webhook Delivery
#   WHK-04:  Webhook Without Prior verify_payment Call
#   WHK-05:  Refund Idempotency Verification
#   WHK-06:  Token-based Webhook Deduplication
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
    echo "Usage: ./run-all-whk-tests.sh --email \"user@example.com\""
    exit 1
fi

echo -e "${CYAN}╔════════════════════════════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║          WHK (Webhook Integrity) Test Suite                    ║${NC}"
echo -e "${CYAN}╚════════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo -e "${BLUE}Email: $EMAIL${NC}"
echo -e "${BLUE}Time:  $(date -u +%Y-%m-%dT%H:%M:%SZ)${NC}"
echo ""

# Test tracking
TESTS_PASSED=0
TESTS_FAILED=0
TESTS_WARNED=0
RESULTS=()
FAILED_TESTS=()

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
            FAILED_TESTS+=("$test_id: $test_name")
            RESULTS+=("{\"test_id\": \"$test_id\", \"test_name\": \"$test_name\", \"status\": \"fail\"}")
            echo ""
            echo -e "${RED}✗ $test_id FAILED${NC}"
        fi
    fi
}

# Run all WHK tests
echo -e "${BLUE}Starting WHK Test Suite...${NC}"
START_TIME=$(date +%s)

# === REJECTION TESTS (no subscription needed) ===
echo -e "${CYAN}[PHASE 1] Running rejection tests (no subscription needed)...${NC}"

# WHK-01: Invalid Pub/Sub Signature Rejection
run_test "WHK-01" "test-whk-01.sh" "Invalid Pub/Sub Signature Rejection"

# WHK-01B: Audience Claim Mismatch Rejection
run_test "WHK-01B" "test-whk-01b.sh" "Audience Claim Mismatch Rejection"

# === SETUP: Create subscription (not counted in WHK totals) ===
echo -e "${CYAN}[SETUP] Running SUB-01 to create subscription for remaining tests (not counted in WHK)...${NC}"
if bash "$SCRIPT_DIR/test-sub-01.sh" --email "$EMAIL"; then
    echo -e "${GREEN}✓ SUB-01 PASSED (setup)${NC}"
else
    echo -e "${RED}✗ SUB-01 FAILED (setup)${NC}"
fi
echo ""

# === ACCEPTANCE/PROCESSING TESTS (subscription needed) ===
echo -e "${CYAN}[PHASE 2] Running acceptance tests (subscription required)...${NC}"

# WHK-01C: Audience Validation With Correct Audience
run_test "WHK-01C" "test-whk-01c.sh" "Audience Validation With Correct Audience"

# WHK-01D: Audience Validation Disabled (Dev Mode)
run_test "WHK-01D" "test-whk-01d.sh" "Audience Validation Disabled (Dev Mode)"

# WHK-02: Duplicate Webhook Delivery (Idempotency)
run_test "WHK-02" "test-whk-02.sh" "Duplicate Webhook Delivery (Idempotency)"

# WHK-03: Out-of-Order Webhook Delivery
run_test "WHK-03" "test-whk-03.sh" "Out-of-Order Webhook Delivery"

# === EDGE CASE TESTS ===
echo -e "${CYAN}[PHASE 3] Running edge case tests...${NC}"

# WHK-04: Webhook Without Prior verify_payment Call
run_test "WHK-04" "test-whk-04.sh" "Webhook Without Prior verify_payment Call"

# WHK-05: Refund Idempotency Verification
run_test "WHK-05" "test-whk-05.sh" "Refund Idempotency Verification"

# WHK-06: Token-based Webhook Deduplication
run_test "WHK-06" "test-whk-06.sh" "Token-based Webhook Deduplication"

END_TIME=$(date +%s)
DURATION=$((END_TIME - START_TIME))

# Calculate totals
TOTAL_TESTS=$((TESTS_PASSED + TESTS_FAILED))

# Generate summary
echo ""
echo -e "${CYAN}╔════════════════════════════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║                    WHK Test Suite Summary                      ║${NC}"
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
    echo -e "${GREEN}║              ✓ ALL WHK TESTS PASSED!                           ║${NC}"
    echo -e "${GREEN}╚════════════════════════════════════════════════════════════════╝${NC}"
else
    SUITE_STATUS="fail"
    echo -e "${RED}╔════════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${RED}║              ✗ SOME WHK TESTS FAILED                           ║${NC}"
    echo -e "${RED}╚════════════════════════════════════════════════════════════════╝${NC}"
    echo ""
    echo -e "${RED}Failed Tests Summary:${NC}"
    for failed_test in "${FAILED_TESTS[@]}"; do
        echo -e "${RED}  • $failed_test${NC}"
    done
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
cat > whk-suite-summary.json <<EOF
{
  "suite_id": "WHK",
  "suite_name": "Webhook Integrity Tests",
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

echo "Summary report saved to: whk-suite-summary.json"
echo ""
cat whk-suite-summary.json
echo ""

# Exit with appropriate code
if [[ $TESTS_FAILED -gt 0 ]]; then
    exit 1
fi
exit 0
