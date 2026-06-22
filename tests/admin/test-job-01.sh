#!/bin/bash

##############################################################################
# ADMIN-JOB-01: Manual trigger-jobs Is Idempotent
##############################################################################

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

TIMESTAMP=$(date +%s)
TEST_STARTED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
TEST_RUN_ID="admin-job-01-${TIMESTAMP}-$$"
REPORT_FILE="job-01-report.json"

echo -e "${YELLOW}========================================${NC}"
echo "ADMIN-JOB-01: Manual trigger-jobs Is Idempotent"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Get JWT
ADMIN_JWT=${ADMIN_JWT:-}
if [[ -z "$ADMIN_JWT" ]]; then
    if curl -s -f "$MOCK_CLERK_URL/token" >/dev/null; then
        ADMIN_JWT=$(curl -s "$MOCK_CLERK_URL/token?org=org_test")
    else
        echo -e "${RED}✗ Mock Clerk server not running and no ADMIN_JWT set.${NC}"
        exit 1
    fi
fi

# Call 1
echo -e "${YELLOW}[1/3] Calling POST /admin/trigger-jobs (First Pass)${NC}"
RESPONSE1=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/admin/trigger-jobs" \
  -H "Authorization: Bearer $ADMIN_JWT" \
  -H "Content-Type: application/json" \
  -d '{"jobs": ["webhook_retry"]}' 2>/dev/null || echo "error")

HTTP_CODE1=$(echo "$RESPONSE1" | tail -n1)
BODY1=$(echo "$RESPONSE1" | sed '$d')

echo -e "${BLUE}  HTTP Code 1: $HTTP_CODE1${NC}"
echo -e "${BLUE}  Body 1: $BODY1${NC}"

# Call 2 (Immediate)
echo -e "${YELLOW}[2/3] Calling POST /admin/trigger-jobs (Second Pass - Immediate)${NC}"
RESPONSE2=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/admin/trigger-jobs" \
  -H "Authorization: Bearer $ADMIN_JWT" \
  -H "Content-Type: application/json" \
  -d '{"jobs": ["webhook_retry"]}' 2>/dev/null || echo "error")

HTTP_CODE2=$(echo "$RESPONSE2" | tail -n1)
BODY2=$(echo "$RESPONSE2" | sed '$d')

echo -e "${BLUE}  HTTP Code 2: $HTTP_CODE2${NC}"
echo -e "${BLUE}  Body 2: $BODY2${NC}"

# Verify results
SHAPE1_OK="false"
if echo "$BODY1" | grep -q "results" && echo "$BODY1" | grep -q "webhook_retry"; then
    SHAPE1_OK="true"
fi

SHAPE2_OK="false"
if echo "$BODY2" | grep -q "results" && echo "$BODY2" | grep -q "webhook_retry"; then
    SHAPE2_OK="true"
fi

STATUS_OK="false"
if [[ "$HTTP_CODE1" == "200" ]] && [[ "$HTTP_CODE2" == "200" ]]; then
    STATUS_OK="true"
    echo -e "${GREEN}✓ Both trigger-jobs calls returned 200 OK${NC}"
else
    echo -e "${RED}✗ Unexpected HTTP codes: Call1=$HTTP_CODE1, Call2=$HTTP_CODE2${NC}"
fi

# Cleanup (none needed, just logging verification)
echo -e "${YELLOW}[3/3] Cleanup${NC}"
echo -e "${GREEN}✓ No test-specific data to clean up${NC}"

TEST_STATUS="fail"
if [[ "$STATUS_OK" == "true" ]] && [[ "$SHAPE1_OK" == "true" ]] && [[ "$SHAPE2_OK" == "true" ]]; then
    TEST_STATUS="pass"
fi

TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "ADMIN-JOB-01",
  "test_name": "Manual trigger-jobs Is Idempotent",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "$TEST_STATUS",
  "results": {
    "http_code_1": "$HTTP_CODE1",
    "http_code_2": "$HTTP_CODE2",
    "shape_1_ok": $SHAPE1_OK,
    "shape_2_ok": $SHAPE2_OK
  }
}
EOF

echo ""
if [[ "$TEST_STATUS" == "pass" ]]; then
    echo -e "${GREEN}✓ ADMIN-JOB-01 PASSED${NC}"
    exit 0
else
    echo -e "${RED}✗ ADMIN-JOB-01 FAILED${NC}"
    exit 1
fi
