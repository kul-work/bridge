#!/bin/bash

##############################################################################
# CBI OTP Test Suite Runner
# 
# Purpose: Orchestrates the execution of all Creem One-Time Payment (OTP) 
#          test scenarios (OTP-01 to OTP-03).
#
# Usage: ./run-all-otp-tests.sh --email "user@example.com"
#
# Scopes:
#   OTP-01: Successful Webhook
#   OTP-02: Sync Redirect Fallback
#   OTP-03: Refund Handling
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

if [[ -z "$EMAIL" ]]; then
    echo -e "${RED}Error: --email is required${NC}"
    exit 1
fi

echo -e "${BLUE}╔════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║       Creem OTP Test Suite (CBI)               ║${NC}"
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
chmod +x test-otp-*.sh cleanup-all-otp.sh

# Run Tests (Specified in Section A of CREEM_BILLING_TESTPLAN.md)
run_test "test-otp-01.sh" "OTP-01: Successful Purchase (Webhook)"
run_test "test-otp-02.sh" "OTP-02: Sync Redirect Verification"
run_test "test-otp-03.sh" "OTP-03: Refund Creation (Webhook)"

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
  "test_suite": "CBI OTP Suite",
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
    echo -e "${GREEN}All Creem OTP tests PASSED${NC}"
    exit 0
else
    echo -e "${RED}Some Creem OTP tests FAILED${NC}"
    exit 1
fi
