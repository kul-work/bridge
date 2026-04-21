#!/bin/bash

##############################################################################
# CBI Access Control Test Suite (ACC-01 to ACC-04)
# 
# Purpose: Orchestrates the execution of all Access Control (ACC) 
#          test scenarios (ACC-01 through ACC-04).
#
# Usage: ./run-all-acc-tests.sh --email "user@example.com"
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
GRAY='\033[0;90m'
NC='\033[0m'

# Defaults
EMAIL=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --email)
            EMAIL="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done



echo -e "${YELLOW}========================================${NC}"
echo "Running all CBI ACC tests for: $EMAIL"
echo -e "${YELLOW}========================================${NC}"

TESTS=("test-acc-01.sh" "test-acc-02.sh" "test-acc-03.sh" "test-acc-04.sh")
PASSED=0
FAILED=0
TESTS_RUN=0
declare -a RESULTS

for test in "${TESTS[@]}"; do
    echo -e "\n${YELLOW}>>> Running $test${NC}"
    
    # Derive test ID from filename
    test_id=$(echo "$test" | sed 's/test-//; s/\.sh//' | tr 'a-z' 'A-Z')
    TESTS_RUN=$((TESTS_RUN + 1))
    
    if bash "$SCRIPT_DIR/$test" --email "$EMAIL"; then
        PASSED=$((PASSED + 1))
        RESULTS+=("{\"test_id\": \"$test_id\", \"test_name\": \"$test\", \"status\": \"pass\"}")
    else
        FAILED=$((FAILED + 1))
        RESULTS+=("{\"test_id\": \"$test_id\", \"test_name\": \"$test\", \"status\": \"fail\"}")
    fi
done

# Build JSON results array
JSON_RESULTS="["
for i in "${!RESULTS[@]}"; do
    if [[ $i -gt 0 ]]; then
        JSON_RESULTS+=", "
    fi
    JSON_RESULTS+="${RESULTS[$i]}"
done
JSON_RESULTS+="]"

# Generate summary report
cat > acc-suite-summary.json <<EOF
{
  "test_suite": "CBI ACC Suite",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "email": "$EMAIL",
  "total_tests": $TESTS_RUN,
  "passed": $PASSED,
  "failed": $FAILED,
  "status": $([ $FAILED -eq 0 ] && echo "\"all_passed\"" || echo "\"some_failed\""),
  "tests": $JSON_RESULTS
}
EOF

echo -e "\n${YELLOW}========================================${NC}"
echo "CBI ACC Tests Summary:"
echo -e "  Passed: ${GREEN}$PASSED${NC}"
if [[ $FAILED -eq 0 ]]; then
    echo -e "  Failed: ${GRAY}$FAILED${NC}"
else
    echo -e "  Failed: ${RED}$FAILED${NC}"
fi
echo -e "${YELLOW}========================================${NC}"

if [[ $FAILED -eq 0 ]]; then
    exit 0
else
    exit 1
fi
