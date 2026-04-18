#!/bin/bash

##############################################################################
# run-all-sub-tests.sh - Execute full subscription test suite
# 
# Purpose: Run all subscription tests (SUB-01 through SUB-09) sequentially
#          and generate a summary report.
#
# Usage: ./run-all-sub-tests.sh [--cleanup-first]
#
# Options:
#   --cleanup-first  Optional. Run cleanup before tests.
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
CLEANUP_FIRST=false

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --cleanup-first)
            CLEANUP_FIRST=true
            shift
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo -e "${BLUE}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║      SUBSCRIPTION TEST SUITE - Google Play Billing         ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════╗${NC}"
echo ""
echo "Time: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo ""

# Optional cleanup
if [[ "$CLEANUP_FIRST" == "true" ]]; then
    echo -e "${YELLOW}Running cleanup first...${NC}"
    bash cleanup-all-sub.sh || true
    echo ""
fi

# Define tests to run (in order)
# Core Lifecycle Tests: SUB-01 to SUB-09
# Other test groups have their own runner scripts:
#   - run-all-sub-trials-tests.sh (SUB-14, SUB-15)
#   - run-all-sub-restore-tests.sh (SUB-16 to SUB-19)
#   - run-all-sub-price-tests.sh (SUB-20, SUB-21)
#   - run-all-ack-tests.sh (ACK-01 to ACK-03)
#
TESTS=(
    "test-sub-01.sh:SUB-01:Initial Subscription Purchase"
    "test-sub-02.sh:SUB-02:Subscription Renewal (Automatic)"
    "test-sub-03.sh:SUB-03:User-Initiated Cancellation"
    "test-sub-04.sh:SUB-04:Renewal Success After Grace Period Recovery"
    "test-sub-05.sh:SUB-05:Subscription Expiration"
    "test-sub-06.sh:SUB-06:Re-subscription (After Expiry)"
    "test-sub-07.sh:SUB-07:Slow Card (Pending Renewal)"
    "test-sub-08.sh:SUB-08:Account Hold (Payment Failure)"
    "test-sub-09.sh:SUB-09:Subscription Revoked (Refund)"
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
    
    # Run the test
    set +e
    bash "$script"
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
echo -e "Failed: ${RED}$FAILED${NC}"
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
cat > core-suite-summary.json <<EOF
{
  "suite": "Core Lifecycle Tests (SUB-01 to SUB-09)",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "total": $TOTAL,
  "passed": $PASSED,
  "failed": $FAILED,
  "skipped": $SKIPPED,
  "results": {
    "SUB-01": "${RESULTS[SUB-01]:-unknown}",
    "SUB-02": "${RESULTS[SUB-02]:-unknown}",
    "SUB-03": "${RESULTS[SUB-03]:-unknown}",
    "SUB-04": "${RESULTS[SUB-04]:-unknown}",
    "SUB-05": "${RESULTS[SUB-05]:-unknown}",
    "SUB-06": "${RESULTS[SUB-06]:-unknown}",
    "SUB-07": "${RESULTS[SUB-07]:-unknown}",
    "SUB-08": "${RESULTS[SUB-08]:-unknown}",
    "SUB-09": "${RESULTS[SUB-09]:-unknown}"
  }
}
EOF

echo "Summary saved to: sub-core-suite-summary.json"
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
    echo ""
    echo -e "${BLUE}Running cleanup script...${NC}"
    bash cleanup-all-sub.sh || true
    echo -e "${GREEN}Cleanup completed successfully${NC}"
    exit 0
fi
