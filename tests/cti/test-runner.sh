#!/bin/bash

##############################################################################
# CTI Master Test Runner
#
# Purpose: Single entry point for all Contract & Tenant Isolation tests.
#          Orchestrates execution of ISO (isolation) and CONTRACT (endpoint
#          shape) test suites.
#
# Location: tests/cti/test-runner.sh
#
# Usage: ./test-runner.sh [--scope SCOPE] [--clear]
#
# Scopes:
#   full      - Run ALL tests (Default)
#   iso       - Run only cross-app isolation tests
#   contract  - Run only endpoint shape conformance tests
#   smoke     - Run minimal check (ISO-01, ISO-05, CONTRACT-04, CONTRACT-06)
#
##############################################################################

set -uo pipefail

export BRIDGE_TEST_RUNNER=1

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

# Defaults
SCOPE="full"
CLEAR_FIRST=false
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
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

echo -e "${CYAN}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║        Contract & Tenant Isolation - Master Test Runner    ║${NC}"
echo -e "${CYAN}╚════════════════════════════════════════════════════════════╝${NC}"
echo "Scope: $SCOPE"
echo "Time:  $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo ""

cd "$SCRIPT_DIR"

# Source globals to display app info
source "$SCRIPT_DIR/globals.cfg"

echo -e "${BLUE}App A: $APP_A_SLUG ($APP_A_ID)${NC}"
echo -e "${BLUE}App B: $APP_B_SLUG ($APP_B_ID)${NC}"
echo ""

# Verify prerequisites
if [[ ! -f "run-all-iso-tests.sh" ]] || [[ ! -f "run-all-contract-tests.sh" ]]; then
    echo -e "${RED}Error: Runner scripts not found in $SCRIPT_DIR${NC}"
    exit 1
fi

if [[ -z "$APP_A_ID" ]] || [[ -z "$APP_B_ID" ]]; then
    echo -e "${RED}Error: Both App A and App B must be registered in pay.apps.${NC}"
    echo -e "${YELLOW}  App A slug: ${APP_A_SLUG:-not set} → ID: ${APP_A_ID:-not found}${NC}"
    echo -e "${YELLOW}  App B slug: ${APP_B_SLUG:-not set} → ID: ${APP_B_ID:-not found}${NC}"
    exit 1
fi

if [[ "$APP_A_ID" == "$APP_B_ID" ]]; then
    echo -e "${RED}Error: App A and App B must be different apps for isolation testing.${NC}"
    exit 1
fi

# Optional initial clear
if [[ "$CLEAR_FIRST" == "true" ]]; then
    echo -e "${YELLOW}Clearing test data before run...${NC}"
    if [[ -f "cleanup-runner.sh" ]]; then
        bash cleanup-runner.sh
    else
        echo -e "${RED}Warning: cleanup-runner.sh not found, skipping clear.${NC}"
    fi
    echo ""
else
    echo -e "${YELLOW}⚠ Warning: Running without initial clear. Use --clear for reliable results.${NC}"
    echo ""
fi

# Result tracking
FAILED_SUITES=0
FAILED_TESTS=0
SUITES_RUN=0
ALL_TESTS_RUN_LIST=""
FAILED_TEST_CODES=""

cleanup_suite() {
    local suite="$1"
    local script="cleanup-all-${suite}.sh"
    if [[ -f "$script" ]]; then
        echo -e "${YELLOW}Cleaning up $suite data before suite...${NC}"
        bash "$script" 2>/dev/null || true
    fi
}

run_suite() {
    local script="$1"
    local name="$2"

    local suite_prefix="${script//run-all-/}"
    suite_prefix="${suite_prefix//-tests.sh/}"

    cleanup_suite "$suite_prefix"

    echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${BLUE}▶ Starting Suite: $name${NC}"
    echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

    if bash "$script"; then
        echo -e "${GREEN}✓ Suite PASSED: $name${NC}"
    else
        echo -e "${RED}✗ Suite FAILED: $name${NC}"
        FAILED_SUITES=$((FAILED_SUITES + 1))
    fi
    SUITES_RUN=$((SUITES_RUN + 1))

    local summary_file="${suite_prefix}-suite-summary.json"
    if [[ -f "$summary_file" ]]; then
        local suite_ids
        suite_ids=$(jq -r '.tests[]? | .test_id' "$summary_file" 2>/dev/null | tr '\n' ' ')
        ALL_TESTS_RUN_LIST="$ALL_TESTS_RUN_LIST $suite_ids"

        local fails
        fails=$(jq -r '.tests[]? | select(.status == "fail") | .test_id' "$summary_file" 2>/dev/null | tr '\n' ' ')
        if [[ -n "$fails" ]]; then
            FAILED_TEST_CODES="$FAILED_TEST_CODES $fails"
        fi
    fi

    echo ""
}

run_smoke_tests() {
    echo "Step 1: ISO-01 (Cross-App Subscription Visibility)"
    ALL_TESTS_RUN_LIST="$ALL_TESTS_RUN_LIST ISO-01"
    if bash test-iso-01.sh; then
        echo -e "${GREEN}✓ ISO-01 Passed${NC}"
    else
        echo -e "${RED}✗ ISO-01 Failed${NC}"
        FAILED_TESTS=$((FAILED_TESTS + 1))
        FAILED_TEST_CODES="$FAILED_TEST_CODES ISO-01"
    fi

    echo "Step 2: ISO-05 (Checkout Idempotency Key Isolation)"
    ALL_TESTS_RUN_LIST="$ALL_TESTS_RUN_LIST ISO-05"
    if bash test-iso-05.sh; then
        echo -e "${GREEN}✓ ISO-05 Passed${NC}"
    else
        echo -e "${RED}✗ ISO-05 Failed${NC}"
        FAILED_TESTS=$((FAILED_TESTS + 1))
        FAILED_TEST_CODES="$FAILED_TEST_CODES ISO-05"
    fi

    echo "Step 3: CONTRACT-04 (Subscription Status Endpoint Shape)"
    ALL_TESTS_RUN_LIST="$ALL_TESTS_RUN_LIST CONTRACT-04"
    if bash test-contract-04.sh; then
        echo -e "${GREEN}✓ CONTRACT-04 Passed${NC}"
    else
        echo -e "${RED}✗ CONTRACT-04 Failed${NC}"
        FAILED_TESTS=$((FAILED_TESTS + 1))
        FAILED_TEST_CODES="$FAILED_TEST_CODES CONTRACT-04"
    fi

    echo "Step 4: CONTRACT-06 (Signed Email Lookup)"
    ALL_TESTS_RUN_LIST="$ALL_TESTS_RUN_LIST CONTRACT-06"
    if bash test-contract-06.sh; then
        echo -e "${GREEN}✓ CONTRACT-06 Passed${NC}"
    else
        echo -e "${RED}✗ CONTRACT-06 Failed${NC}"
        FAILED_TESTS=$((FAILED_TESTS + 1))
        FAILED_TEST_CODES="$FAILED_TEST_CODES CONTRACT-06"
    fi
}

start_time=$(date +%s)

case $SCOPE in
    full)
        run_suite "run-all-iso-tests.sh" "Cross-App Isolation (ISO)"
        run_suite "run-all-contract-tests.sh" "Endpoint Shape Conformance (CONTRACT)"
        ;;
    iso)
        run_suite "run-all-iso-tests.sh" "Cross-App Isolation (ISO)"
        ;;
    contract)
        run_suite "run-all-contract-tests.sh" "Endpoint Shape Conformance (CONTRACT)"
        ;;
    smoke)
        run_smoke_tests
        ;;
    *)
        echo -e "${RED}Error: Invalid scope '$SCOPE'. Use full, iso, contract, or smoke.${NC}"
        exit 1
        ;;
esac

end_time=$(date +%s)
duration=$((end_time - start_time))

TOTAL_UNIQUE_RUN=$(echo "$ALL_TESTS_RUN_LIST" | tr ' ' '\n' | grep -v '^$' | sort -u | wc -l | tr -d '[:space:]' || echo 0)
CLEAN_FAILS=$(echo "$FAILED_TEST_CODES" | tr ' ' '\n' | grep -v '^$' | sort -u | xargs | tr ' ' ',' || echo "")
FAILED_TEST_COUNT=$(echo "$CLEAN_FAILS" | tr ',' '\n' | grep -v '^$' | wc -l | tr -d '[:space:]' || echo 0)

echo -e "${CYAN}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║                  MASTER EXECUTION SUMMARY                  ║${NC}"
echo -e "${CYAN}╚════════════════════════════════════════════════════════════╝${NC}"
echo "Scope: ${SCOPE^^}"
echo "Total Duration: ${duration}s"
echo "Suites Executed: $SUITES_RUN"
echo "Unique Tests Run: $TOTAL_UNIQUE_RUN"

if [[ $FAILED_SUITES -eq 0 && $FAILED_TEST_COUNT -eq 0 ]]; then
    echo -e "${GREEN}Result: ALL SUITES PASSED${NC}"
    exit 0
else
    echo -e "${RED}Result: FAILURE ($FAILED_SUITES suite(s) failed, $FAILED_TEST_COUNT test(s) failed)${NC}"
    if [[ $FAILED_TEST_COUNT -gt 0 ]]; then
        echo -e "${RED}Failed Tests: $CLEAN_FAILS${NC}"
    fi
    exit 1
fi