#!/bin/bash

##############################################################################
# run-restore-tests.sh - Execute Restore Test Suite (SUB-16 to SUB-19)
# 
# Purpose: Run all subscription restoration tests sequentially.
#          These tests cover re-subscription and restore scenarios.
#
# Usage: ./run-restore-tests.sh --email "user@example.com" [--email2 "user2@example.com"]
#
# Prerequisites:
#   - Active or cancelled subscription exists (run core lifecycle tests first)
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
EMAIL=""
EMAIL2=""

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
    echo "Usage: ./run-restore-tests.sh --email \"user@example.com\" [--email2 \"user2@example.com\"]"
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo -e "${BLUE}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║       RESTORE TEST SUITE - Re-subscription & Recovery      ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo "Email: $EMAIL"
if [[ -n "$EMAIL2" ]]; then
    echo "Email2: $EMAIL2 (for multi-account tests)"
fi
echo "Time: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo ""

# Define tests to run
TESTS=(
    "test-sub-16.sh:SUB-16:Resubscribe Before Expiration"
    "test-sub-17.sh:SUB-17:Restore After Uninstall/Reinstall"
    "test-sub-18.sh:SUB-18:Restore on Multiple Devices"
    "test-sub-19.sh:SUB-19:Restore with Account System"
    "test-sub-19b.sh:SUB-19B:LinkingRequired Response (Different Account Verification)"
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
    
    # SUB-19 needs two emails for multi-account test
    set +e
    if [[ "$test_id" == "SUB-19" ]] && [[ -n "$EMAIL2" ]]; then
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
echo -e "${BLUE}║                RESTORE SUITE SUMMARY                       ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo "Total Tests: $TOTAL"
echo -e "Passed: ${GREEN}$PASSED${NC}"
echo -e "Failed: ${RED}$FAILED${NC}"
echo -e "Skipped: ${YELLOW}$SKIPPED${NC}"
echo ""

# Generate JSON summary
cat > restore-suite-summary.json <<EOF
{
  "suite": "Restore Tests (SUB-16 to SUB-19)",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "email": "$EMAIL",
  "email2": "$EMAIL2",
  "total": $TOTAL,
  "passed": $PASSED,
  "failed": $FAILED,
  "skipped": $SKIPPED,
  "results": {
    "SUB-16": "${RESULTS[SUB-16]:-unknown}",
    "SUB-17": "${RESULTS[SUB-17]:-unknown}",
    "SUB-18": "${RESULTS[SUB-18]:-unknown}",
    "SUB-19": "${RESULTS[SUB-19]:-unknown}"
  }
}
EOF

echo "Summary saved to: restore-suite-summary.json"

if [[ $FAILED -gt 0 ]]; then
    exit 1
else
    exit 0
fi
