#!/bin/bash

##############################################################################
# GPBI Master Test Runner
#
# Purpose: Single entry point for all Google Play Billing Integration tests.
#          Orchestrates execution of OTP, SUB, and infrastructure test suites.
#
# Location: tests/gpbi/test-runner.sh
#
# Usage: ./test-runner.sh [--scope SCOPE] [--clear]
#
# Scopes:
#   full      - Run ALL tests (Default)
#   commerce  - Run only commerce logic (OTP + SUB + ACK)
#   infra     - Run infrastructure checks (ACC, ERR, LOG, NET, WHK)
#   smoke     - Run minimal health check (OTP-01, SUB-01, SUB-02, SUB-03, SUB-06, SUB-09, SUB-19B, SUB-PAUSE-01, SUB-PAUSE-02, WHK-01, WHK-02, ACK-01, ERR-01)
#   replay    - Run ONLY tests with fixture replay capability (18 tests, deterministic, Google API fixture used)
#
##############################################################################

set -uo pipefail

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
echo -e "${CYAN}║             Google Play Billing - Master Test Runner       ║${NC}"
echo -e "${CYAN}╚════════════════════════════════════════════════════════════╝${NC}"
echo "Scope: $SCOPE"
echo "Time:  $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo ""

cd "$SCRIPT_DIR"

# verify prerequisites
if [[ ! -f "run-all-sub-tests.sh" ]]; then
    echo -e "${RED}Error: Runner scripts not found in $SCRIPT_DIR${NC}"
    exit 1
fi

# Optional initial clear (before all tests)
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
SUITES_RUN=0
ALL_TESTS_RUN_LIST=""
FAILED_TEST_CODES=""

# Cleanup function to run before each suite
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
    local extra_args="${3:-}"
    
    # Extract suite prefix for cleanup (e.g., run-all-otp-tests.sh -> otp)
    local suite_prefix="${script//run-all-/}"
    suite_prefix="${suite_prefix//-tests.sh/}"
    
    # Cleanup before running
    cleanup_suite "$suite_prefix"
    
    echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${BLUE}▶ Starting Suite: $name${NC}"
    echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    
    local cmd="bash \"$script\""
    if [[ -n "$extra_args" ]]; then
        cmd="$cmd $extra_args"
    fi

    if eval $cmd; then
        echo -e "${GREEN}✓ Suite PASSED: $name${NC}"
    else
        echo -e "${RED}✗ Suite FAILED: $name${NC}"
        FAILED_SUITES=$((FAILED_SUITES + 1))
    fi
    SUITES_RUN=$((SUITES_RUN + 1))

    # Extract counts and failed codes from suite summary using jq
    local summary_file="${suite_prefix}-suite-summary.json"
    if [[ -f "$summary_file" ]]; then
        # Extract all test IDs from JSON
        # Try multiple extraction methods for different JSON structures
        local suite_ids=""
        
        # Method 1: Extract from .tests array (OTP, NET, LOG style)
        suite_ids=$(jq -r '.tests[]? | .test_id' "$summary_file" 2>/dev/null | tr '\n' ' ')
        
        # Method 2: Extract from .suites nested object (SUB, ACK, ACC, WHK, ERR, API style)
        if [[ -z "$suite_ids" ]]; then
            suite_ids=$(jq -r '.suites | to_entries[]? | .value | to_entries[]? | .key' "$summary_file" 2>/dev/null | tr '\n' ' ')
        fi
        
        local suite_id_count=$(echo "$suite_ids" | tr ' ' '\n' | grep -v '^$' | wc -l)
        echo -e "${BLUE}DEBUG: $suite_prefix extracted $suite_id_count IDs${NC}"
        ALL_TESTS_RUN_LIST="$ALL_TESTS_RUN_LIST $suite_ids"

        # Extract failed IDs
        local fails=""
        # Check for failed tests in .tests array
        fails=$(jq -r '.tests[]? | select(.status == "fail") | .test_id' "$summary_file" 2>/dev/null | tr '\n' ' ')
        
        # Check for failed tests in .suites nested structure
        if [[ -z "$fails" ]]; then
            fails=$(jq -r '.suites | to_entries[]? | .value | to_entries[]? | select(.value == "fail") | .key' "$summary_file" 2>/dev/null | tr '\n' ' ')
        fi
        
        if [[ -n "$fails" ]]; then
            FAILED_TEST_CODES="$FAILED_TEST_CODES $fails"
        fi
    fi
    
    echo ""
}

run_smoke_tests() {
    # Run OTP-01 only
    echo "Step 1: OTP-01 (One-Time Purchase)"
    ALL_TESTS_RUN_LIST="$ALL_TESTS_RUN_LIST OTP-01"
    if bash test-otp-01.sh; then
        echo -e "${GREEN}✓ OTP-01 Passed${NC}"
    else 
        echo -e "${RED}✗ OTP-01 Failed${NC}"
        FAILED_SUITES=$((FAILED_SUITES + 1))
        FAILED_TEST_CODES="$FAILED_TEST_CODES OTP-01"
    fi
    
    # Run SUB-01 only
    echo "Step 2: SUB-01 (Subscription Purchase)"
    ALL_TESTS_RUN_LIST="$ALL_TESTS_RUN_LIST SUB-01"
    if bash test-sub-01.sh; then
        echo -e "${GREEN}✓ SUB-01 Passed${NC}"
    else 
        echo -e "${RED}✗ SUB-01 Failed${NC}"
        FAILED_SUITES=$((FAILED_SUITES + 1))
        FAILED_TEST_CODES="$FAILED_TEST_CODES SUB-01"
    fi

    # Run SUB-02 (Renewal) - Depends on SUB-01
    echo "Step 3: SUB-02 (Subscription Renewal)"
    ALL_TESTS_RUN_LIST="$ALL_TESTS_RUN_LIST SUB-02"
    if bash test-sub-02.sh; then
        echo -e "${GREEN}✓ SUB-02 Passed${NC}"
    else 
        echo -e "${RED}✗ SUB-02 Failed${NC}"
        FAILED_SUITES=$((FAILED_SUITES + 1))
        FAILED_TEST_CODES="$FAILED_TEST_CODES SUB-02"
    fi

    # Run SUB-03 (Cancellation) - Depends on SUB-01
    echo "Step 4: SUB-03 (Subscription Cancellation)"
    ALL_TESTS_RUN_LIST="$ALL_TESTS_RUN_LIST SUB-03"
    if bash test-sub-03.sh; then
        echo -e "${GREEN}✓ SUB-03 Passed${NC}"
    else 
        echo -e "${RED}✗ SUB-03 Failed${NC}"
        FAILED_SUITES=$((FAILED_SUITES + 1))
        FAILED_TEST_CODES="$FAILED_TEST_CODES SUB-03"
    fi

    # Run SUB-06 (Re-subscription after Expiry)
    echo "Step 5: SUB-06 (Re-subscription after Expiry)"
    ALL_TESTS_RUN_LIST="$ALL_TESTS_RUN_LIST SUB-06"
    if bash test-sub-06.sh; then
        echo -e "${GREEN}✓ SUB-06 Passed${NC}"
    else 
        echo -e "${RED}✗ SUB-06 Failed${NC}"
        FAILED_SUITES=$((FAILED_SUITES + 1))
        FAILED_TEST_CODES="$FAILED_TEST_CODES SUB-06"
    fi

    # Run SUB-09 (Subscription Revoked/Refunded)
    echo "Step 6: SUB-09 (Subscription Revoked/Refunded)"
    ALL_TESTS_RUN_LIST="$ALL_TESTS_RUN_LIST SUB-09"
    if bash test-sub-09.sh; then
        echo -e "${GREEN}✓ SUB-09 Passed${NC}"
    else 
        echo -e "${RED}✗ SUB-09 Failed${NC}"
        FAILED_SUITES=$((FAILED_SUITES + 1))
        FAILED_TEST_CODES="$FAILED_TEST_CODES SUB-09"
    fi

    # Run SUB-19B (Linking Required)
    echo "Step 7: SUB-19B (Linking Required)"
    ALL_TESTS_RUN_LIST="$ALL_TESTS_RUN_LIST SUB-19B"
    if bash test-sub-19b.sh; then
        echo -e "${GREEN}✓ SUB-19B Passed${NC}"
    else 
        echo -e "${RED}✗ SUB-19B Failed${NC}"
        FAILED_SUITES=$((FAILED_SUITES + 1))
        FAILED_TEST_CODES="$FAILED_TEST_CODES SUB-19B"
    fi

    # Run SUB-PAUSE-01 (Schedule Pause) - Prerequisite for SUB-PAUSE-02
    echo "Step 8a: SUB-PAUSE-01 (Schedule Pause)"
    ALL_TESTS_RUN_LIST="$ALL_TESTS_RUN_LIST SUB-PAUSE-01"
    if bash test-sub-pause-01.sh; then
        echo -e "${GREEN}✓ SUB-PAUSE-01 Passed${NC}"
    else 
        echo -e "${RED}✗ SUB-PAUSE-01 Failed${NC}"
        FAILED_SUITES=$((FAILED_SUITES + 1))
        FAILED_TEST_CODES="$FAILED_TEST_CODES SUB-PAUSE-01"
    fi

    # Run SUB-PAUSE-02 (Pause Takes Effect)
    echo "Step 8b: SUB-PAUSE-02 (Pause Takes Effect)"
    ALL_TESTS_RUN_LIST="$ALL_TESTS_RUN_LIST SUB-PAUSE-02"
    if bash test-sub-pause-02.sh; then
        echo -e "${GREEN}✓ SUB-PAUSE-02 Passed${NC}"
    else 
        echo -e "${RED}✗ SUB-PAUSE-02 Failed${NC}"
        FAILED_SUITES=$((FAILED_SUITES + 1))
        FAILED_TEST_CODES="$FAILED_TEST_CODES SUB-PAUSE-02"
    fi

    # Run WHK-01 (Webhook Verification)
    echo "Step 9: WHK-01 (Webhook Verification)"
    ALL_TESTS_RUN_LIST="$ALL_TESTS_RUN_LIST WHK-01"
    if bash test-whk-01.sh; then
        echo -e "${GREEN}✓ WHK-01 Passed${NC}"
    else 
        echo -e "${RED}✗ WHK-01 Failed${NC}"
        FAILED_SUITES=$((FAILED_SUITES + 1))
        FAILED_TEST_CODES="$FAILED_TEST_CODES WHK-01"
    fi

    # Run WHK-02 (Webhook Idempotency)
    echo "Step 10: WHK-02 (Webhook Idempotency)"
    ALL_TESTS_RUN_LIST="$ALL_TESTS_RUN_LIST WHK-02"
    if bash test-whk-02.sh; then
        echo -e "${GREEN}✓ WHK-02 Passed${NC}"
    else 
        echo -e "${RED}✗ WHK-02 Failed${NC}"
        FAILED_SUITES=$((FAILED_SUITES + 1))
        FAILED_TEST_CODES="$FAILED_TEST_CODES WHK-02"
    fi

    # Run ACK-01 (Acknowledgment)
    echo "Step 11: ACK-01 (Initial Purchase Acknowledgment)"
    ALL_TESTS_RUN_LIST="$ALL_TESTS_RUN_LIST ACK-01"
    if bash test-ack-01.sh; then
        echo -e "${GREEN}✓ ACK-01 Passed${NC}"
    else 
        echo -e "${RED}✗ ACK-01 Failed${NC}"
        FAILED_SUITES=$((FAILED_SUITES + 1))
        FAILED_TEST_CODES="$FAILED_TEST_CODES ACK-01"
    fi
 
    # Run ERR-01 (Basic Validation Check)
    echo "Step 12: ERR-01 (Invalid Token Check)"
    ALL_TESTS_RUN_LIST="$ALL_TESTS_RUN_LIST ERR-01"
    if bash test-err-01.sh; then
        echo -e "${GREEN}✓ ERR-01 Passed${NC}"
    else 
        echo -e "${RED}✗ ERR-01 Failed${NC}"
        FAILED_SUITES=$((FAILED_SUITES + 1))
        FAILED_TEST_CODES="$FAILED_TEST_CODES ERR-01"
    fi
}

run_replay_test() {
    local script="$1"
    local test_id="$2"
    local test_name="$3"
    local extra_args="${4:-}"
    
    echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${YELLOW}[Replay] $test_id - $test_name${NC}"
    echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    
    ALL_TESTS_RUN_LIST="$ALL_TESTS_RUN_LIST $test_id"
    SUITES_RUN=$((SUITES_RUN + 1))
    
    if bash "$script" --replay $extra_args; then
        echo -e "${GREEN}✓ $test_id Passed${NC}"
    else
        echo -e "${RED}✗ $test_id Failed${NC}"
        FAILED_SUITES=$((FAILED_SUITES + 1))
        FAILED_TEST_CODES="$FAILED_TEST_CODES $test_id"
    fi
    echo ""
}

run_replay_tests() {
    echo -e "${YELLOW}Running Replay Tests (Fixture-Based, 18 Tests)...${NC}"
    echo ""
    
    # OTP tests with replay (4)
    run_replay_test "test-otp-01.sh"      "OTP-01"      "Successful Purchase"
    run_replay_test "test-otp-04.sh"      "OTP-04"      "Slow Card (Pending State)"
    run_replay_test "test-otp-rtdn-01.sh" "OTP-RTDN-01" "Webhook Purchase Completed"
    run_replay_test "test-otp-rtdn-02.sh" "OTP-RTDN-02" "Webhook Refund Completed"
    
    # SUB core lifecycle with replay (9)
    run_replay_test "test-sub-01.sh" "SUB-01" "Initial Subscription Purchase"
    run_replay_test "test-sub-02.sh" "SUB-02" "Subscription Renewal (Automatic)"
    run_replay_test "test-sub-03.sh" "SUB-03" "User-Initiated Cancellation"
    run_replay_test "test-sub-04.sh" "SUB-04" "Renewal After Grace Period Recovery"
    run_replay_test "test-sub-05.sh" "SUB-05" "Subscription Expiration"
    run_replay_test "test-sub-06.sh" "SUB-06" "Re-subscription (After Expiry)"
    run_replay_test "test-sub-08.sh" "SUB-08" "Account Hold (Payment Failure)"
    run_replay_test "test-sub-09.sh" "SUB-09" "Subscription Revoked (Refund)"
    run_replay_test "test-sub-24.sh" "SUB-24" "Restart After Cancellation"
    
    # SUB price changes with replay (2)
    run_replay_test "test-sub-20.sh" "SUB-20" "Price Change (Opt-In Increase)"
    run_replay_test "test-sub-21.sh" "SUB-21" "Price Step-Up Consent (Korea)"
    
    # SUB pause with replay (3)
    run_replay_test "test-sub-pause-01.sh" "SUB-PAUSE-01" "Schedule Pause"
    run_replay_test "test-sub-pause-02.sh" "SUB-PAUSE-02" "Pause Takes Effect"
    run_replay_test "test-sub-pause-03.sh" "SUB-PAUSE-03" "Manual Resume from Pause"
}

start_time=$(date +%s)

case $SCOPE in
    full)
        # Commerce First (clean state required)
        run_suite "run-all-otp-tests.sh" "One-Time Products (OTP)"
        run_suite "run-all-sub-tests.sh" "Subscriptions (SUB)"
        run_suite "run-all-sub-paused-tests.sh" "Subscription Pause (PAUSE)"
        run_suite "run-all-ack-tests.sh" "Acknowledgment (ACK)"
        
        # Infrastructure (can run with existing data)
        run_suite "run-all-acc-tests.sh" "Access Control (ACC)"
        run_suite "run-all-whk-tests.sh" "Webhooks Core (WHK)"
        run_suite "run-all-net-tests.sh" "Network & Resilience (NET)"
        
        # Observability & Errors
        run_suite "run-all-err-tests.sh" "Error Handling (ERR)"
        run_suite "run-all-log-tests.sh" "Logging (LOG)"
        run_suite "run-all-api-tests.sh" "API & Notifications (API)"
        ;;        
    commerce)
        run_suite "run-all-otp-tests.sh" "One-Time Products (OTP)"
        run_suite "run-all-sub-tests.sh" "Subscriptions (SUB)"
        run_suite "run-all-sub-paused-tests.sh" "Subscription Pause (PAUSE)"
        run_suite "run-all-ack-tests.sh" "Acknowledgment (ACK)"
        ;;        
    infra)
        run_suite "run-all-acc-tests.sh" "Access Control (ACC)"
        run_suite "run-all-whk-tests.sh" "Webhooks Core (WHK)"
        run_suite "run-all-net-tests.sh" "Network & Resilience (NET)"
        run_suite "run-all-err-tests.sh" "Error Handling (ERR)"
        run_suite "run-all-log-tests.sh" "Logging (LOG)"
        run_suite "run-all-api-tests.sh" "API & Notifications (API)"
        ;;
        
    smoke)
        run_smoke_tests
        ;;
        
    replay)
        run_replay_tests
        ;;
        
    *)
        echo -e "${RED}Error: Invalid scope '$SCOPE'. Use full, commerce, infra, replay, or smoke.${NC}"
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
if [[ "$SCOPE" == "full" ]]; then
    echo "Scope: MASTER"
else
    echo "Scope: ${SCOPE^^}"
fi
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
