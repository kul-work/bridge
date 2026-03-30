#!/bin/bash

##############################################################################
# run-all-sub-paused-tests.sh - Execute subscription pause test suite
# 
# Purpose: Run SUB-01 and the three pause-related tests (PAUSE-01 through PAUSE-03)
#          sequentially and generate a summary report.
#
# Usage: ./run-all-sub-paused-tests.sh --email "user@example.com"
#
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
CLEANUP_FIRST=false

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --email)
            EMAIL="$2"
            shift 2
            ;;
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

# Validate required inputs
if [[ -z "$EMAIL" ]]; then
    echo -e "${RED}Error: --email is required${NC}"
    echo "Usage: ./run-all-sub-paused-tests.sh --email \"user@example.com\" [--cleanup-first]"
    exit 1
fi

cd "$SCRIPT_DIR"

echo -e "${BLUE}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║      SUBSCRIPTION PAUSE SUITE - Google Play Billing        ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo "Email: $EMAIL"
echo "Time: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo ""

# Optional cleanup
if [[ "$CLEANUP_FIRST" == "true" ]]; then
    echo -e "${YELLOW}Running cleanup first...${NC}"
    bash cleanup-all-sub.sh --email "$EMAIL" || true
    echo ""
fi

# Define tests to run
TESTS=(
    "test-sub-01.sh:SUB-01:Initial Subscription Purchase"
    "test-sub-pause-01.sh:PAUSE-01:Schedule Subscription Pause"
    "test-sub-pause-02.sh:PAUSE-02:Pause Becomes Effective"
    "test-sub-pause-03.sh:PAUSE-03:Subscription Resumed (Restarted)"
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
    bash "$script" --email "$EMAIL"
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
cat > sub-paused-suite-summary.json <<EOF
{
  "suite": "Subscription Pause Tests (4 Tests)",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "email": "$EMAIL",
  "total": $TOTAL,
  "passed": $PASSED,
  "failed": $FAILED,
  "skipped": $SKIPPED,
  "success_rate": "$((TOTAL > 0 ? (PASSED * 100 / TOTAL) : 0)).0%",
  "suites": {
    "pause": {
      "SUB-01": "${RESULTS[SUB-01]:-unknown}",
      "PAUSE-01": "${RESULTS[PAUSE-01]:-unknown}",
      "PAUSE-02": "${RESULTS[PAUSE-02]:-unknown}",
      "PAUSE-03": "${RESULTS[PAUSE-03]:-unknown}"
    }
  }
}
EOF

echo "Summary saved to: sub-paused-suite-summary.json"
echo ""

# Final status
if [[ $FAILED -gt 0 ]]; then
    exit 1
else
    exit 0
fi
