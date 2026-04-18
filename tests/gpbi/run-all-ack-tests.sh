#!/bin/bash

##############################################################################
# run-ack-tests.sh - Execute Acknowledgment Test Suite (ACK-01 to ACK-03)
# 
# Purpose: Run all acknowledgment-related tests sequentially.
#          Tests verify ACK behavior on initial purchase, retry, and renewals.
#
# Usage: ./run-ack-tests.sh
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

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo -e "${BLUE}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║       ACKNOWLEDGMENT TEST SUITE - ACK Behavior Tests       ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo "Time: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "" 

# Define tests to run (in order)
# SUB-01 is prerequisite - creates the subscription that ACK tests depend on
# Note: SUB-01 is run but NOT counted in ACK suite totals (counted in SUB suite)
PREREQUISITE_TESTS=(
    "test-sub-01.sh:SUB-01:Initial Subscription (prerequisite)"
)

TESTS=(
    "test-ack-01.sh:ACK-01:ACK on Initial Purchase"
    "test-ack-02.sh:ACK-02:ACK Failure & Retry Queue"
    "test-ack-03.sh:ACK-03:No ACK on Renewal"
)

# Track results
TOTAL=0
PASSED=0
FAILED=0
SKIPPED=0

declare -A RESULTS

# Run prerequisites without counting them in totals
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${YELLOW}Running Prerequisites (not counted in ACK suite totals)${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

for test_entry in "${PREREQUISITE_TESTS[@]}"; do
    IFS=':' read -r script test_id test_name <<< "$test_entry"
    
    echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${YELLOW}Running: $test_id - $test_name${NC}"
    echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    
    if [[ ! -f "$script" ]]; then
        echo -e "${YELLOW}⚠ Test script not found: $script - SKIPPED${NC}"
        continue
    fi
    
    set +e
    bash "$script"
    EXIT_CODE=$?
    set -e
    
    if [[ $EXIT_CODE -eq 0 ]]; then
        echo -e "${GREEN}✓ $test_id PASSED (prerequisite)${NC}"
    else
        echo -e "${RED}✗ $test_id FAILED (prerequisite)${NC}"
    fi
    echo ""
done

echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${YELLOW}Running ACK Tests${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo ""

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
echo -e "${BLUE}║                  ACK SUITE SUMMARY                         ║${NC}"
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

# Generate JSON summary (only ACK tests, not prerequisites)
cat > ack-suite-summary.json <<EOF
{
  "suite": "Acknowledgment Tests (3 Tests)",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "total": $TOTAL,
  "passed": $PASSED,
  "failed": $FAILED,
  "skipped": $SKIPPED,
  "note": "Prerequisites (SUB-01) run separately, counted in SUB suite",
  "suites": {
    "acknowledgment": {
      "ACK-01": "${RESULTS[ACK-01]:-unknown}",
      "ACK-02": "${RESULTS[ACK-02]:-unknown}",
      "ACK-03": "${RESULTS[ACK-03]:-unknown}"
    }
  }
}
EOF

echo "Summary saved to: ack-suite-summary.json"
echo ""

if [[ $FAILED -gt 0 ]]; then
    echo -e "${RED}═══════════════════════════════════════════════════════════════${NC}"
    echo -e "${RED}  SUITE FAILED: $FAILED test(s) did not pass${NC}"
    echo -e "${RED}═══════════════════════════════════════════════════════════════${NC}"
    exit 1
else
    echo -e "${GREEN}═══════════════════════════════════════════════════════════════${NC}"
    echo -e "${GREEN}  SUITE PASSED: All $PASSED test(s) successful${NC}"
    echo -e "${GREEN}═══════════════════════════════════════════════════════════════${NC}"
    exit 0
fi
