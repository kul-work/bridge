#!/bin/bash

##############################################################################
# CBI Network Resilience Test Suite (NET-01 to NET-02)
# 
# Purpose: Orchestrates the execution of all Network Resilience (NET) 
#          test scenarios (NET-01 through NET-02).
#
# Usage: ./run-all-net-tests.sh --email "user@example.com"
##############################################################################

set -euo pipefail

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
GRAY='\033[0;90m'
NC='\033[0m'

EMAIL=""
while [[ $# -gt 0 ]]; do
    case $1 in
        --email) EMAIL="$2"; shift 2 ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done



PASSED=0
FAILED=0
TESTS_RUN=0
declare -a RESULTS
TESTS=("test-net-01.sh" "test-net-02.sh")

echo "========================================"
echo "Running Creem NET (Network) Test Suite"
echo "========================================"

for test_script in "${TESTS[@]}"; do
    # Derive test ID from filename
    test_id=$(echo "$test_script" | sed 's/test-//; s/\.sh//' | tr 'a-z' 'A-Z')
    TESTS_RUN=$((TESTS_RUN + 1))
    
    if bash "$test_script" --email "$EMAIL"; then
        PASSED=$((PASSED + 1))
        RESULTS+=("{\"test_id\": \"$test_id\", \"test_name\": \"$test_script\", \"status\": \"pass\"}")
    else
        FAILED=$((FAILED + 1))
        RESULTS+=("{\"test_id\": \"$test_id\", \"test_name\": \"$test_script\", \"status\": \"fail\"}")
    fi
    echo ""
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
cat > net-suite-summary.json <<EOF
{
  "test_suite": "CBI NET Suite",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "email": "$EMAIL",
  "total_tests": $TESTS_RUN,
  "passed": $PASSED,
  "failed": $FAILED,
  "status": $([ $FAILED -eq 0 ] && echo "\"all_passed\"" || echo "\"some_failed\""),
  "tests": $JSON_RESULTS
}
EOF

echo "========================================"
echo "NET Suite Summary for $EMAIL"
echo -e "Passed: ${GREEN}$PASSED${NC}"
if [[ "$FAILED" -eq 0 ]]; then
    echo -e "Failed: ${GRAY}$FAILED${NC}"
else
    echo -e "Failed: ${RED}$FAILED${NC}"
fi
echo "========================================"

if [[ "$FAILED" -gt 0 ]]; then exit 1; fi
exit 0
