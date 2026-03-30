#!/bin/bash

##############################################################################
# run-all-sub-tests.sh - Execute full subscription test suite
# 
# Purpose: Run all subscription tests (SUB-01 through SUB-XX) sequentially
#          and generate a summary report.
#
# Usage: ./run-all-sub-tests.sh --email "user@example.com" [--email2 "other@example.com"] [--cleanup-first]
#
# Options:
#   --email          Required. Email of test user in database.
#   --email2         Optional. Second user email for multi-account tests (SUB-19).
#   --cleanup-first  Optional. Run cleanup before tests.
#   --purge-reports  Optional. Delete reports after successful run (default: keep).
#
# Output:
#   - Individual test reports: sub-01-report.json, sub-02-report.json, etc.
#   - Suite summary: sub-suite-summary.json
##############################################################################

set -uo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Defaults
EMAIL=""
EMAIL2=""
CLEANUP_FIRST=false
PURGE_REPORTS=false

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
        --cleanup-first)
            CLEANUP_FIRST=true
            shift
            ;;
        --purge-reports)
            PURGE_REPORTS=true
            shift
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
    echo "Usage: ./run-all-sub-tests.sh --email \"user@example.com\" [--email2 \"other@example.com\"] [--cleanup-first]"
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo -e "${BLUE}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║      SUBSCRIPTION TEST SUITE - Google Play Billing         ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo "Email: $EMAIL"
if [[ -n "$EMAIL2" ]]; then
    echo "Email2: $EMAIL2"
fi
echo "Time: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo ""

# Optional cleanup
if [[ "$CLEANUP_FIRST" == "true" ]]; then
    echo -e "${YELLOW}Running cleanup first...${NC}"
    bash cleanup-all-sub.sh --email "$EMAIL" || true
    echo ""
fi

# Define tests to run (in order)
# Note: Some tests depend on previous tests (e.g., SUB-03 depends on SUB-01)
# 
# Suite Structure (6 logical groups, 20 tests total):
#   1. Core Lifecycle (SUB-01 to SUB-09) - Sequential flow
#   2. Restoration & Restart (SUB-16, SUB-24, SUB-17 to SUB-19) - Re-subscribe scenarios (BEFORE trial tests that clean pay.subscriptions)
#   3. Out-of-App Linking (SUB-22) - Resubscribe after expiry with out-of-app context
#   4. Free Trials (SUB-14, SUB-15) - Fresh account requirements (DELETE pay.subscriptions)
#   5. Price Changes (SUB-20, SUB-21) - Opt-in and Korea step-up
#   6. Pending Purchase Handling (SUB-23) - Cleanup of abandoned pending purchases
#
TESTS=(
    # Suite 1: Core Lifecycle (9 tests)
    "test-sub-01.sh:SUB-01:Initial Subscription Purchase"
    "test-sub-02.sh:SUB-02:Subscription Renewal (Automatic)"
    "test-sub-03.sh:SUB-03:User-Initiated Cancellation"
    "test-sub-04.sh:SUB-04:Renewal Success After Grace Period Recovery"
    "test-sub-05.sh:SUB-05:Subscription Expiration"
    "test-sub-06.sh:SUB-06:Re-subscription (After Expiry)"
    "test-sub-07.sh:SUB-07:Slow Card (Pending Renewal)"
    "test-sub-08.sh:SUB-08:Account Hold (Payment Failure)"
    "test-sub-09.sh:SUB-09:Subscription Revoked (Refund)"
    # Suite 2: Restoration & Restart (5 tests) - BEFORE trial tests to preserve subscription state
    "test-sub-16.sh:SUB-16:Resubscribe Before Expiration (User-Initiated)"
    "test-sub-24.sh:SUB-24:Restart After Cancellation - Expiry Extension (Webhook-Driven)"
    "test-sub-17.sh:SUB-17:Restore After Uninstall/Reinstall"
    "test-sub-18.sh:SUB-18:Restore on Multiple Devices"
    "test-sub-19.sh:SUB-19:Restore with Account System"
    "test-sub-19b.sh:SUB-19B:LinkingRequired Response (Different Account Verification)"
    # Suite 3: Out-of-App Linking (1 test) - Tests OutOfAppPurchaseContext feature
    "test-sub-22.sh:SUB-22:Out-of-App Resubscribe Linking (SUB-RESUB-01)"
    # Suite 4: Free Trials (2 tests) - Cleans pay.subscriptions (requires fresh account)
    "test-sub-14.sh:SUB-14:Free Trial - First-Time User"
    "test-sub-15.sh:SUB-15:Free Trial - No Prior Subscriptions"
    # Suite 5: Price Changes (2 tests)
    "test-sub-20.sh:SUB-20:Price Change (Opt-In Increase)"
    "test-sub-21.sh:SUB-21:Price Step-Up Consent (Korea)"
    # Suite 6: Pending Purchase Handling (1 test)
    "test-sub-23.sh:SUB-23:Pending Purchase Canceled"
)

# Track results
TOTAL=0
PASSED=0
FAILED=0
SKIPPED=0

declare -A RESULTS

for test_entry in "${TESTS[@]}"; do
    IFS=':' read -r script test_id test_name <<< "$test_entry"
    
    TOTAL=$((TOTAL + 1))
    
    echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${YELLOW}Running: $test_id - $test_name${NC}"
    echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    
    if [[ ! -f "$script" ]]; then
        echo -e "${YELLOW}⚠ Test script not found: $script - SKIPPED${NC}"
        SKIPPED=$((SKIPPED + 1))
        RESULTS[$test_id]="skipped"
        continue
    fi
    
    # Run the test (with extra args for tests that need --email2)
    set +e
    if [[ "$test_id" == "SUB-19" || "$test_id" == "SUB-19B" ]] && [[ -n "$EMAIL2" ]]; then
        bash "$script" --email "$EMAIL" --email2 "$EMAIL2"
    else
        bash "$script" --email "$EMAIL"
    fi
    EXIT_CODE=$?
    set -e
    
    if [[ $EXIT_CODE -eq 0 ]]; then
        echo -e "${GREEN}✓ $test_id PASSED${NC}"
        PASSED=$((PASSED + 1))
        RESULTS[$test_id]="pass"
    else
        echo -e "${RED}✗ $test_id FAILED (exit code: $EXIT_CODE)${NC}"
        FAILED=$((FAILED + 1))
        RESULTS[$test_id]="fail"
    fi
    
    echo ""
done

# Generate summary
echo -e "${BLUE}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║                    TEST SUITE SUMMARY                      ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo "Total Tests: $TOTAL"
echo -e "Passed: ${GREEN}$PASSED${NC}"
if [[ $FAILED -gt 0 ]]; then
    echo -e "Failed: ${RED}$FAILED${NC}"
else
    echo -e "Failed: $FAILED"
fi
echo -e "Skipped: ${YELLOW}$SKIPPED${NC}"
echo ""

# Print individual results
echo "Individual Results:"
for test_entry in "${TESTS[@]}"; do
    IFS=':' read -r script test_id test_name <<< "$test_entry"
    result="${RESULTS[$test_id]:-unknown}"
    
    case $result in
        pass)
            echo -e "  ${GREEN}✓${NC} $test_id: $test_name"
            ;;
        fail)
            echo -e "  ${RED}✗${NC} $test_id: $test_name"
            ;;
        skipped)
            echo -e "  ${YELLOW}○${NC} $test_id: $test_name (skipped)"
            ;;
        *)
            echo -e "  ${YELLOW}?${NC} $test_id: $test_name (unknown)"
            ;;
    esac
done
echo ""

# Generate JSON summary
cat > sub-suite-summary.json <<EOF
{
  "suite": "Subscription Tests (19 Tests)",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "email": "$EMAIL",
  "total": $TOTAL,
  "passed": $PASSED,
  "failed": $FAILED,
  "skipped": $SKIPPED,
  "success_rate": "$((TOTAL > 0 ? (PASSED * 100 / TOTAL) : 0)).0%",
  "suites": {
    "core_lifecycle": {
      "SUB-01": "${RESULTS[SUB-01]:-unknown}",
      "SUB-02": "${RESULTS[SUB-02]:-unknown}",
      "SUB-03": "${RESULTS[SUB-03]:-unknown}",
      "SUB-04": "${RESULTS[SUB-04]:-unknown}",
      "SUB-05": "${RESULTS[SUB-05]:-unknown}",
      "SUB-06": "${RESULTS[SUB-06]:-unknown}",
      "SUB-07": "${RESULTS[SUB-07]:-unknown}",
      "SUB-08": "${RESULTS[SUB-08]:-unknown}",
      "SUB-09": "${RESULTS[SUB-09]:-unknown}"
    },
    "trials": {
      "SUB-14": "${RESULTS[SUB-14]:-unknown}",
      "SUB-15": "${RESULTS[SUB-15]:-unknown}"
    },
    "restoration": {
      "SUB-16": "${RESULTS[SUB-16]:-unknown}",
      "SUB-17": "${RESULTS[SUB-17]:-unknown}",
      "SUB-18": "${RESULTS[SUB-18]:-unknown}",
      "SUB-19": "${RESULTS[SUB-19]:-unknown}",
      "SUB-19B": "${RESULTS[SUB-19B]:-unknown}"
    },
    "out_of_app_linking": {
      "SUB-22": "${RESULTS[SUB-22]:-unknown}"
    },
    "price_changes": {
      "SUB-20": "${RESULTS[SUB-20]:-unknown}",
      "SUB-21": "${RESULTS[SUB-21]:-unknown}"
    },
    "pending_purchase_handling": {
      "SUB-23": "${RESULTS[SUB-23]:-unknown}"
    }
  }
}
EOF

echo "Summary saved to: sub-suite-summary.json"
echo ""

# Final status
if [[ $FAILED -gt 0 ]]; then
    echo -e "${RED}═══════════════════════════════════════════════════════════════${NC}"
    echo -e "${RED}  SUITE FAILED: $FAILED test(s) did not pass${NC}"
    echo -e "${RED}═══════════════════════════════════════════════════════════════${NC}"
    exit 1
else
    echo -e "${GREEN}═══════════════════════════════════════════════════════════════${NC}"
    echo -e "${GREEN}  SUITE PASSED: All $PASSED test(s) successful${NC}"
    echo -e "${GREEN}═══════════════════════════════════════════════════════════════${NC}"
    if [[ "$PURGE_REPORTS" == "true" ]]; then
        echo ""
        echo -e "${BLUE}Purging reports (--purge-reports)...${NC}"
        bash cleanup-all-sub.sh --email "$EMAIL" || true
        echo -e "${GREEN}Reports purged successfully${NC}"
    fi
    exit 0
fi
