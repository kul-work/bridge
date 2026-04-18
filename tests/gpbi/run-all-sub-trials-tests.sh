#!/bin/bash

##############################################################################
# run-trials-tests.sh - Execute Free Trials Test Suite (SUB-14, SUB-15)
# 
# Purpose: Run all free trial subscription tests sequentially.
#          These tests require fresh Google accounts that have never purchased.
#
# Usage: ./run-trials-tests.sh [--cleanup-first]
#
# Prerequisites:
#   - Test user must have NEVER purchased this subscription (fresh account)
#   - Backend running with MOCK_EXTERNAL_APIS=true
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
echo -e "${BLUE}║         TRIALS TEST SUITE - Free Trial Subscriptions       ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo "Time: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "" 
echo -e "${YELLOW}Note: These tests require accounts that have NEVER purchased pay.subscriptions${NC}"
echo ""

# Define tests to run
TESTS=(
    "test-sub-14.sh:SUB-14:Free Trial - First-Time User"
    "test-sub-15.sh:SUB-15:Free Trial - No Prior Subscriptions"
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
echo -e "${BLUE}║                 TRIALS SUITE SUMMARY                       ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo "Total Tests: $TOTAL"
echo -e "Passed: ${GREEN}$PASSED${NC}"
echo -e "Failed: ${RED}$FAILED${NC}"
echo -e "Skipped: ${YELLOW}$SKIPPED${NC}"
echo ""

# Generate JSON summary
cat > trials-suite-summary.json <<EOF
{
  "suite": "Trials Tests (SUB-14, SUB-15)",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "total": $TOTAL,
  "passed": $PASSED,
  "failed": $FAILED,
  "skipped": $SKIPPED,
  "results": {
    "SUB-14": "${RESULTS[SUB-14]:-unknown}",
    "SUB-15": "${RESULTS[SUB-15]:-unknown}"
  }
}
EOF

echo "Summary saved to: trials-suite-summary.json"

if [[ $FAILED -gt 0 ]]; then
    exit 1
else
    exit 0
fi
