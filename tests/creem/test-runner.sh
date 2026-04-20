#!/bin/bash

##############################################################################
# CBI Master Test Runner
# 
# Purpose: Single entry point for all Creem Billing Integration (CBI) tests.
#          Orchestrates execution of OTP, SUB suites.
#
# Usage: ./test-runner.sh --email "user@example.com" [--scope SCOPE] [--clear]
# Scopes: full, otp, sub, acc, whk, net, err, smoke
#
# Scopes:
#   full      - Run ALL tests (Default)
#   otp       - Run only One-Time Payment tests
#   sub       - Run only Subscription tests
#   acc       - Run only Access Control tests
#   whk       - Run only Webhook tests
#   net       - Run only Network Resilience tests
#   err       - Run only Error Handling tests
#   smoke     - Run minimal health check (OTP-01, SUB-01, SUB-02, SUB-03, SUB-06, SUB-09, WHK-01, WHK-02, ACC-01, ERR-01)
#
##############################################################################

set -euo pipefail

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[1;34m'
CYAN='\033[0;36m'
NC='\033[0m'

# Defaults
EMAIL=""
USER_ID=""
SCOPE="full"
CLEAR_FIRST=false
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
start_time=0
end_time=0

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --email)
            EMAIL="$2"
            shift 2
            ;;
        --scope)
            SCOPE="$2"
            shift 2
            ;;
        --clear)
            CLEAR_FIRST=true
            shift
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

cd "$SCRIPT_DIR"
source "globals.cfg"

# Now that globals.cfg is sourced (providing default EMAIL if needed), 
# ensure USER_ID is generated from the final EMAIL.
if [[ -z "$USER_ID" ]]; then
    USER_ID="creem_$(echo -n "$EMAIL" | md5sum | cut -d' ' -f1 | cut -c1-12)"
fi

echo -e "${CYAN}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║             Creem Billing - Master Test Runner             ║${NC}"
echo -e "${CYAN}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""

# verify prerequisites
if [[ ! -f "globals.cfg" ]]; then
    echo -e "${RED}Error: globals.cfg not found in $SCRIPT_DIR${NC}"
    exit 1
fi

# Optional initial clear
if [[ "$CLEAR_FIRST" == "true" ]]; then
    echo -e "${YELLOW}Clearing test data before run...${NC}"
    # Run both OTP and SUB cleanup
    if [[ -f "cleanup-all-otp.sh" ]]; then bash cleanup-all-otp.sh --email "$EMAIL" 2>/dev/null || true; fi
    if [[ -f "cleanup-all-sub.sh" ]]; then bash cleanup-all-sub.sh --email "$EMAIL" 2>/dev/null || true; fi
    if [[ -f "cleanup-all-acc.sh" ]]; then bash cleanup-all-acc.sh --email "$EMAIL" 2>/dev/null || true; fi
    if [[ -f "cleanup-all-whk.sh" ]]; then bash cleanup-all-whk.sh --email "$EMAIL" 2>/dev/null || true; fi
    if [[ -f "cleanup-all-net.sh" ]]; then bash cleanup-all-net.sh --email "$EMAIL" 2>/dev/null || true; fi
    if [[ -f "cleanup-all-err.sh" ]]; then bash cleanup-all-err.sh --email "$EMAIL" 2>/dev/null || true; fi
    echo ""
fi

# Result tracking
FAILED_SUITES=0
SUITES_RUN=0
ALL_TESTS_RUN_LIST=""
FAILED_TEST_CODES=""

run_suite() {
    local script="$1"
    local name="$2"
    
    # Extract suite prefix for summary file (e.g., run-all-otp-tests.sh -> otp)
    local suite_prefix="${script//run-all-/}"
    suite_prefix="${suite_prefix//-tests.sh/}"

    echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${BLUE}▶ Starting Suite: $name${NC}"
    echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    
    if bash "$script" --email "$EMAIL"; then
        echo -e "${GREEN}✓ Suite PASSED: $name${NC}"
    else
        echo -e "${RED}✗ Suite FAILED: $name${NC}"
        FAILED_SUITES=$((FAILED_SUITES + 1))
    fi
    SUITES_RUN=$((SUITES_RUN + 1))

    # Extract results using jq if summary file exists
    local summary_file="${suite_prefix}-suite-summary.json"
    if [[ -f "$summary_file" ]]; then
        local suite_ids=$(jq -r '.tests[]? | .test_id' "$summary_file" 2>/dev/null | tr '\n' ' ')
        ALL_TESTS_RUN_LIST="$ALL_TESTS_RUN_LIST $suite_ids"

        local fails=$(jq -r '.tests[]? | select(.status == "fail") | .test_id' "$summary_file" 2>/dev/null | tr '\n' ' ')
        if [[ -n "$fails" ]]; then
            FAILED_TEST_CODES="$FAILED_TEST_CODES $fails"
        fi
    fi
    echo ""
}

run_smoke_tests() {
    echo -e "${YELLOW}Running Smoke Tests (Critical Path Only)...${NC}"
    
    local tests=(
        "test-otp-01.sh|OTP-01"
        "test-sub-01.sh|SUB-01"
        "test-sub-02.sh|SUB-02"
        "test-sub-03.sh|SUB-03"
        "test-sub-06.sh|SUB-06"
        "test-sub-09.sh|SUB-09"
        "test-whk-01.sh|WHK-01"
        "test-whk-02.sh|WHK-02"
        "test-acc-01.sh|ACC-01"
        "test-err-01.sh|ERR-01"
    )

    for item in "${tests[@]}"; do
        local script="${item%|*}"
        local tid="${item#*|}"
        
        echo -e "${BLUE}▶ Running Smoke: $tid${NC}"
        ALL_TESTS_RUN_LIST="$ALL_TESTS_RUN_LIST $tid"
        
        if bash "$script" --email "$EMAIL"; then 
            echo -e "${GREEN}✓ $tid Passed${NC}"
        else 
            echo -e "${RED}✗ $tid Failed${NC}"
            FAILED_SUITES=$((FAILED_SUITES + 1))
            FAILED_TEST_CODES="$FAILED_TEST_CODES $tid"
        fi
        echo ""
    done
}

start_time=$(date +%s)

case $SCOPE in
    full)
        run_suite "run-all-otp-tests.sh" "One-Time Products (OTP)"
        run_suite "run-all-sub-tests.sh" "Subscriptions (SUB)"
        run_suite "run-all-acc-tests.sh" "Access Control (ACC)"
        run_suite "run-all-whk-tests.sh" "Webhooks (WHK)"
        run_suite "run-all-net-tests.sh" "Network Resilience (NET)"
        run_suite "run-all-err-tests.sh" "Error Handling (ERR)"
        ;;
    otp)
        run_suite "run-all-otp-tests.sh" "One-Time Products (OTP)"
        ;;
    sub)
        run_suite "run-all-sub-tests.sh" "Subscriptions (SUB)"
        ;;
    acc)
        run_suite "run-all-acc-tests.sh" "Access Control (ACC)"
        ;;
    whk)
        run_suite "run-all-whk-tests.sh" "Webhooks (WHK)"
        ;;
    net)
        run_suite "run-all-net-tests.sh" "Network Resilience (NET)"
        ;;
    err)
        run_suite "run-all-err-tests.sh" "Error Handling (ERR)"
        ;;
    smoke)
        run_smoke_tests
        ;;
    *)
        echo -e "${RED}Error: Invalid scope '$SCOPE'. Use full, otp, sub, acc, whk, net, err, or smoke.${NC}"
        exit 1
        ;;
esac

end_time=$(date +%s)
duration=$((end_time - start_time))

# Deduplicate test counts and failed codes
TOTAL_UNIQUE_RUN=$(echo "$ALL_TESTS_RUN_LIST" | tr ' ' '\n' | grep -v '^$' | sort -u | wc -l || echo 0)
CLEAN_FAILS=$(echo "$FAILED_TEST_CODES" | tr ' ' '\n' | grep -v '^$' | sort -u | xargs | tr ' ' ',' || echo "")
FAILED_TEST_COUNT=$(echo "$CLEAN_FAILS" | tr ',' '\n' | grep -v '^$' | wc -l | tr -d ' ' || echo 0)

echo -e "${CYAN}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║                  MASTER EXECUTION SUMMARY                  ║${NC}"
echo -e "${CYAN}╚════════════════════════════════════════════════════════════╝${NC}"
echo "Scope: ${SCOPE^^}"
echo "Total Duration: ${duration}s"
echo "Suites Executed: $SUITES_RUN"
echo "Unique Tests Run: $TOTAL_UNIQUE_RUN"

# Beep when done
powershell -Command "[console]::beep(1000, 500)" 2>/dev/null || echo -e "\a"

if [[ $FAILED_SUITES -eq 0 ]]; then
    echo -e "${GREEN}Result: ALL SUITES PASSED${NC}"
    exit 0
else
    echo -e "${RED}Result: FAILURE ($FAILED_SUITES suites failed)${NC}"
    echo -e "${RED}Failed Tests ($FAILED_TEST_COUNT): $CLEAN_FAILS${NC}"
    exit 1
fi
