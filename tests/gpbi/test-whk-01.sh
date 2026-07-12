#!/bin/bash

##############################################################################
# WHK-01: Bridge Invalid Pub/Sub Signature Rejection
# 
# Purpose: Verify that webhooks with tampered or invalid authorization headers
#          are properly rejected (HTTP 400/403) and not processed.
#
# Usage: ./test-whk-01.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars:
#     * PROVIDER, PACKAGE_NAME, PRODUCT_ID_SUB
#     * WEBHOOK_INGRESS_TOKEN, BRIDGE_API_URL
#
# TESTPLAN Reference:
#   Expected Behavior: Webhook REJECTED with HTTP 400/403.
#                      Backend logs "Pub/Sub signature verification failed".
#                      Ensures only trusted triggers reach processing logic.
#                      Validates that auth middleware enforces signature checks.
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

# Colors for output
YELLOW='\033[1;33m'
RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m' # No Color

# Test configuration
TEST_STARTED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
TIMESTAMP=$(date +%s)
TEST_RUN_ID="whk-01-${TIMESTAMP}-$$"
REPORT_FILE="whk-01-report.json"

echo -e "${YELLOW}========================================${NC}"
echo "WHK-01: Bridge Invalid Pub/Sub Signature Rejection"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

# Step 1: Send webhook with INVALID/TAMPERED authorization header
echo -e "${YELLOW}[1/3] Sending webhook with INVALID authorization header${NC}"

TIMESTAMP_MS=$(date +%s000)
MESSAGE_ID="whk-01-invalid-sig-$TEST_RUN_ID"
PURCHASE_TOKEN="test-whk-01-token-$TEST_RUN_ID"

# Create DeveloperNotification JSON
NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$TIMESTAMP_MS",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 4,
    "purchaseToken": "$PURCHASE_TOKEN",
    "subscriptionId": "$PRODUCT_ID_SUB"
  }
}
EOF
)

# Base64 encode the notification
NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0)

# Send webhook with INVALID authorization header (tampered token).
# Force signature verification even when provider_config has
# verify_webhook_signature=false (CI seed / local mock default).
WEBHOOK_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/$PROVIDER" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer INVALID-TAMPERED-TOKEN-12345" \
  -H "X-Webhook-Verification-Mode: strict" \
  -d "{
    \"message\": {
      \"data\": \"$NOTIFICATION_B64\",
      \"message_id\": \"$MESSAGE_ID\",
      \"attributes\": {}
    },
    \"subscription\": \"projects/test-project/subscriptions/test-sub\"
  }")

WEBHOOK_HTTP_CODE=$(echo "$WEBHOOK_RESPONSE" | tail -n1)
echo "Webhook Response Code: $WEBHOOK_HTTP_CODE"

# Step 2: Verify webhook was rejected
if [[ "$WEBHOOK_HTTP_CODE" == "400" ]] || [[ "$WEBHOOK_HTTP_CODE" == "401" ]] || [[ "$WEBHOOK_HTTP_CODE" == "403" ]]; then
    echo -e "${GREEN}✓ Webhook correctly rejected with HTTP $WEBHOOK_HTTP_CODE${NC}"
else
    echo -e "${RED}✗ Webhook NOT rejected! (HTTP $WEBHOOK_HTTP_CODE)${NC}"
    exit 1
fi

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
TEST_STATUS="pass"
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "WHK-01",
  "test_name": "Bridge Invalid Pub/Sub Signature Rejection",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "$TEST_STATUS",
  "webhook_http_code": "$WEBHOOK_HTTP_CODE",
  "results": {
    "rejected_correctly": true
  }
}
EOF

echo -e "${YELLOW}========================================${NC}"
echo -e "${GREEN}✓ WHK-01 Bridge Test PASSED${NC}"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
echo ""
exit 0
