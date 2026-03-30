#!/bin/bash

##############################################################################
# run-price-tests.sh - Execute Price Change Test Suite (SUB-20, SUB-21)
# 
# Purpose: Run price change subscription tests sequentially.
#          SUB-21 is Korea-only and can be skipped with --skip-kr flag.
#
# Usage: ./run-price-tests.sh --email "user@example.com" [--skip-kr]
#
# Prerequisites:
#   - Active subscription exists
#   - For SUB-20: Price change initiated in Play Console
#   - For SUB-21: Korea market targeting (or skip with --skip-kr)
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
SKIP_KR=false

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --email)
            EMAIL="$2"
            shift 2
            ;;
        --skip-kr)
            SKIP_KR=true
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
    echo "Usage: ./run-price-tests.sh --email \"user@example.com\" [--skip-kr]"
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo -e "${BLUE}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║       PRICE CHANGE TEST SUITE - Opt-In & Step-Up           ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo "Email: $EMAIL"
echo "Skip Korea tests: $SKIP_KR"
echo "Time: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo ""

# Define tests to run
TESTS=(
    "test-sub-20.sh:SUB-20:Price Change (Opt-In Increase)"
    "test-sub-21.sh:SUB-21:Price Step-Up Consent (Korea)"
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
    
    # SUB-21 can be skipped for non-Korea markets
    set +e
    if [[ "$test_id" == "SUB-21" ]] && [[ "$SKIP_KR" == "true" ]]; then
        bash "$script" --email "$EMAIL" --skip-if-not-kr
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
echo -e "${BLUE}║                PRICE SUITE SUMMARY                         ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo "Total Tests: $TOTAL"
echo -e "Passed: ${GREEN}$PASSED${NC}"
echo -e "Failed: ${RED}$FAILED${NC}"
echo -e "Skipped: ${YELLOW}$SKIPPED${NC}"
echo ""

# Generate JSON summary
cat > price-suite-summary.json <<EOF
{
  "suite": "Price Change Tests (SUB-20, SUB-21)",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "email": "$EMAIL",
  "skip_korea": $SKIP_KR,
  "total": $TOTAL,
  "passed": $PASSED,
  "failed": $FAILED,
  "skipped": $SKIPPED,
  "results": {
    "SUB-20": "${RESULTS[SUB-20]:-unknown}",
    "SUB-21": "${RESULTS[SUB-21]:-unknown}"
  }
}
EOF

echo "Summary saved to: price-suite-summary.json"

if [[ $FAILED -gt 0 ]]; then
    exit 1
else
    exit 0
fi
