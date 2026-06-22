#!/bin/bash

##############################################################################
# ADMIN-AUTH-02: Admin Endpoint Rejects JWT From Wrong Issuer
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
TEST_RUN_ID="admin-auth-02-${TIMESTAMP}-$$"
REPORT_FILE="auth-02-report.json"

echo -e "${YELLOW}========================================${NC}"
echo "ADMIN-AUTH-02: Admin Endpoint Rejects JWT From Wrong Issuer"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Obtain wrong issuer token
if curl -s -f "$MOCK_CLERK_URL/token" >/dev/null; then
    ADMIN_JWT=$(curl -s "$MOCK_CLERK_URL/token?wrong_iss=true")
else
    echo -e "${RED}✗ Mock Clerk server not running. This test requires a running mock Clerk server.${NC}"
    exit 1
fi

echo -e "${YELLOW}[1/2] Calling POST /admin/trigger-jobs with wrong issuer token${NC}"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/admin/trigger-jobs" \
  -H "Authorization: Bearer $ADMIN_JWT" \
  -H "Content-Type: application/json" \
  -d '{"jobs": ["webhook_retry"]}' 2>/dev/null || echo "error")

HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
BODY=$(echo "$RESPONSE" | sed '$d')

echo -e "${BLUE}  HTTP Code: $HTTP_CODE${NC}"
echo -e "${BLUE}  Body: $BODY${NC}"

TEST_STATUS="fail"
if [[ "$HTTP_CODE" == "401" ]] || [[ "$HTTP_CODE" == "403" ]]; then
    echo -e "${GREEN}✓ Successfully rejected wrong issuer (HTTP $HTTP_CODE)${NC}"
    TEST_STATUS="pass"
else
    echo -e "${RED}✗ Expected 401/403, got $HTTP_CODE${NC}"
fi

# Cleanup
echo -e "${YELLOW}[2/2] Cleanup${NC}"
echo -e "${GREEN}✓ Cleaned up${NC}"

TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "ADMIN-AUTH-02",
  "test_name": "Admin Endpoint Rejects JWT From Wrong Issuer",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "$TEST_STATUS",
  "http_code": "$HTTP_CODE"
}
EOF

echo ""
if [[ "$TEST_STATUS" == "pass" ]]; then
    echo -e "${GREEN}✓ ADMIN-AUTH-02 PASSED${NC}"
    exit 0
else
    echo -e "${RED}✗ ADMIN-AUTH-02 FAILED${NC}"
    exit 1
fi
