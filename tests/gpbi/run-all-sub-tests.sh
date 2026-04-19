#!/bin/bash

##############################################################################
# run-all-sub-tests.sh - Execute full subscription test suite
# 
# Purpose: Run all subscription tests sequentially and generate a summary report.
#          Tests are now idempotent and manage their own test users.
#
# Usage: ./run-all-sub-tests.sh
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

echo -e "${BLUE}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║      SUBSCRIPTION TEST SUITE - Google Play Billing         ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo "Time: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo ""

# Define tests to run (in order)
TESTS=(
    "test-sub-01.sh:SUB-01:Initial Purchase"
    "test-sub-02.sh:SUB-02:Automatic Renewal"
    "test-sub-03.sh:SUB-03:Cancellation"
    "test-sub-04.sh:SUB-04:Grace Period Recovery"
    "test-sub-05.sh:SUB-05:Expiration"
    "test-sub-06.sh:SUB-06:Re-subscription (After Expiry)"
    "test-sub-07.sh:SUB-07:Slow Card (Pending Renewal)"
    "test-sub-08.sh:SUB-08:Account Hold (Payment Failure)"
    "test-sub-09.sh:SUB-09:Revocation (Refund)"
    "test-sub-14.sh:SUB-14:Trial - First Time"
    "test-sub-15.sh:SUB-15:Trial - No Prior Subs"
    "test-sub-16.sh:SUB-16:Resubscribe Before Expiry"
    "test-sub-17.sh:SUB-17:Restore/Reinstall"
    "test-sub-18.sh:SUB-18:Multi-Device Restore"
    "test-sub-19.sh:SUB-19:Account System Restore"
    "test-sub-20.sh:SUB-20:Price Change"
    "test-sub-21.sh:SUB-21:Price Step-Up (Korea)"
    "test-sub-22.sh:SUB-22:Out-of-App Linking"
    "test-sub-23.sh:SUB-23:Pending Cancel"
    "test-sub-24.sh:SUB-24:Restart After Cancel"
    "test-sub-pause-01.sh:PAUSE-01:Pause Scheduled"
    "test-sub-pause-02.sh:PAUSE-02:Pause Effective"
    "test-sub-pause-03.sh:PAUSE-03:Manual Resume"
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
    
    if [[ ! -f "$script" ]]; then
        echo -e "${RED}✗ Test script not found: $script${NC}"
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
echo -e "${BLUE}║                    TEST SUITE SUMMARY                      ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo "Total Tests: $TOTAL"
echo -e "Passed: ${GREEN}$PASSED${NC}"
echo -e "Failed: ${RED}$FAILED${NC}"
echo -e "Skipped: ${YELLOW}$SKIPPED${NC}"
echo ""

# Beep when done
powershell -Command "[console]::beep(1000, 500)" 2>/dev/null || echo -e "\a"

if [[ $FAILED -gt 0 ]]; then
    exit 1
else
    exit 0
fi
