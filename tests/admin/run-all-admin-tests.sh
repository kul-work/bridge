#!/bin/bash

##############################################################################
# Run All Admin Interface Tests
##############################################################################

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

echo -e "${CYAN}╔════════════════════════════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║             ADMIN (Interface & Auth) Test Suite                ║${NC}"
echo -e "${CYAN}╚════════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo -e "${CYAN}Time:  $(date -u +%Y-%m-%dT%H:%M:%SZ)${NC}"
echo ""

# --- Start mock Clerk server ---
MOCK_CLERK_PID=""
if curl -s -f "$MOCK_CLERK_URL/token" >/dev/null 2>&1; then
    echo -e "${GREEN}Mock Clerk already running on port $MOCK_CLERK_PORT${NC}"
else
    echo -e "${YELLOW}Starting mock Clerk server on port $MOCK_CLERK_PORT...${NC}"
    python "$SCRIPT_DIR/mock_clerk.py" "$MOCK_CLERK_PORT" &
    MOCK_CLERK_PID=$!
    for i in {1..10}; do
        if curl -s -f "$MOCK_CLERK_URL/token" >/dev/null 2>&1; then
            echo -e "${GREEN}Mock Clerk started (PID $MOCK_CLERK_PID)${NC}"
            break
        fi
        if [[ "$i" -eq 10 ]]; then
            echo -e "${RED}FAIL: Mock Clerk server did not start${NC}"
            exit 1
        fi
        sleep 1
    done
fi

# --- Pre-flight: verify Bridge is not in bypass mode ---
PREFLIGHT_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  "$BRIDGE_API_URL/admin/trigger-jobs" \
  -H "Content-Type: application/json" \
  -d '{"jobs": ["webhook_retry"]}' 2>/dev/null || echo "000")

if [[ "$PREFLIGHT_CODE" != "401" && "$PREFLIGHT_CODE" != "403" ]]; then
    echo -e "${RED}FAIL: Bridge admin auth is not enforcing JWT checks.${NC}"
    echo -e "${RED}       Got HTTP $PREFLIGHT_CODE for unauthenticated request (expected 401/403).${NC}"
    echo -e "${YELLOW}       Ensure .env has:${NC}"
    echo -e "${YELLOW}         BYPASS_ADMIN_AUTH=false${NC}"
    echo -e "${YELLOW}         ADMIN_CLERK_FRONTEND_API=http://localhost:5577${NC}"
    echo -e "${YELLOW}         ADMIN_CLERK_AUTHORIZED_PARTIES=https://admin.bridge.example.com${NC}"
    echo -e "${YELLOW}         ADMIN_CLERK_ORG_ID=org_test${NC}"
    echo -e "${YELLOW}       Then restart Bridge.${NC}"
    if [[ -n "$MOCK_CLERK_PID" ]]; then
        kill "$MOCK_CLERK_PID" >/dev/null 2>&1 || true
    fi
    exit 1
fi
echo -e "${GREEN}Pre-flight: Bridge admin auth is enforcing JWT checks ✓${NC}"
echo ""

TESTS_PASSED=0
TESTS_FAILED=0
RESULTS=()

run_test() {
    local test_id="$1" test_script="$2" test_name="$3"
    echo ""
    echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${YELLOW}Running: $test_id - $test_name${NC}"
    echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""
    
    if bash "$SCRIPT_DIR/$test_script"; then
        TESTS_PASSED=$((TESTS_PASSED + 1))
        RESULTS+=("{\"test_id\": \"$test_id\", \"test_name\": \"$test_name\", \"status\": \"pass\"}")
        echo -e "${GREEN}✓ $test_id PASSED${NC}"
    else
        TESTS_FAILED=$((TESTS_FAILED + 1))
        RESULTS+=("{\"test_id\": \"$test_id\", \"test_name\": \"$test_name\", \"status\": \"fail\"}")
        echo -e "${RED}✗ $test_id FAILED${NC}"
    fi
}

START_TIME=$(date +%s)

run_test "ADMIN-WHK-01" "test-whk-01.sh" "Dead-Lettered Webhook Manually Retried"
run_test "ADMIN-WHK-02" "test-whk-02.sh" "Admin Retry Does Not Reopen Already-Forwarded Delivery"
run_test "ADMIN-JOB-01" "test-job-01.sh" "Manual trigger-jobs Is Idempotent"
run_test "ADMIN-AUTH-01" "test-auth-01.sh" "Admin Endpoint Rejects JWT Without Matching azp"
run_test "ADMIN-AUTH-02" "test-auth-02.sh" "Admin Endpoint Rejects JWT From Wrong Issuer"
run_test "ADMIN-AUTH-03" "test-auth-03.sh" "Admin Endpoint Rejects Missing Bearer Token"
run_test "ADMIN-CSP-01" "test-csp-01.sh" "CSP Blocks External Scripts, Allows Clerk/Captcha Styles"
run_test "ADMIN-AUDIT-01" "test-audit-01.sh" "Admin Actions Are Recorded in Audit Log"

END_TIME=$(date +%s)
DURATION=$((END_TIME - START_TIME))
TOTAL_TESTS=$((TESTS_PASSED + TESTS_FAILED))

echo ""
echo -e "${CYAN}╔════════════════════════════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║                  ADMIN Test Suite Summary                      ║${NC}"
echo -e "${CYAN}╚════════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo -e "Duration: ${DURATION}s"
echo ""
echo -e "Total Tests: $TOTAL_TESTS"
echo -e "${GREEN}Passed: $TESTS_PASSED${NC}"
if [[ $TESTS_FAILED -gt 0 ]]; then
    echo -e "${RED}Failed: $TESTS_FAILED${NC}"
else
    echo -e "Failed: $TESTS_FAILED"
fi
echo ""

if [[ $TESTS_FAILED -eq 0 ]]; then
    SUITE_STATUS="pass"
    echo -e "${GREEN}╔════════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${GREEN}║             ✓ ALL ADMIN TESTS PASSED!                          ║${NC}"
    echo -e "${GREEN}╚════════════════════════════════════════════════════════════════╝${NC}"
else
    SUITE_STATUS="fail"
    echo -e "${RED}╔════════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${RED}║             ✗ SOME ADMIN TESTS FAILED                          ║${NC}"
    echo -e "${RED}╚════════════════════════════════════════════════════════════════╝${NC}"
fi
echo ""

JSON_RESULTS="["
for i in "${!RESULTS[@]}"; do
    if [[ $i -gt 0 ]]; then JSON_RESULTS+=", "; fi
    JSON_RESULTS+="${RESULTS[$i]}"
done
JSON_RESULTS+="]"

cat > admin-suite-summary.json <<EOF
{
  "suite_id": "ADMIN",
  "suite_name": "Bridge Admin Interface & Auth Tests",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "duration_seconds": $DURATION,
  "status": "$SUITE_STATUS",
  "summary": { "total": $TOTAL_TESTS, "passed": $TESTS_PASSED, "failed": $TESTS_FAILED },
  "tests": $JSON_RESULTS
}
EOF

echo "Summary report saved to: admin-suite-summary.json"
echo ""
cat admin-suite-summary.json
echo ""

# --- Stop mock Clerk if we started it ---
if [[ -n "$MOCK_CLERK_PID" ]]; then
    kill "$MOCK_CLERK_PID" >/dev/null 2>&1 || true
    echo -e "${YELLOW}Mock Clerk server stopped (PID $MOCK_CLERK_PID)${NC}"
fi

if [[ $TESTS_FAILED -gt 0 ]]; then exit 1; fi
exit 0
