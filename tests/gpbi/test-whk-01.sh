#!/bin/bash

##############################################################################
# WHK-01: Bridge Invalid Pub/Sub Signature Rejection
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m' # No Color

echo -e "${YELLOW}========================================${NC}"
echo "WHK-01: Bridge Invalid Pub/Sub Signature Rejection"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: Send webhook with INVALID/TAMPERED authorization header
echo -e "${YELLOW}[1/3] Sending webhook with INVALID authorization header${NC}"

TIMESTAMP=$(date +%s000)
MESSAGE_ID="whk-01-invalid-sig-$(date +%s)"
PURCHASE_TOKEN="test-whk-01-invalid-token"

# Create DeveloperNotification JSON
NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$TIMESTAMP",
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

# Send webhook with INVALID authorization header (tampered token)
WEBHOOK_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/$PROVIDER" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer INVALID-TAMPERED-TOKEN-12345" \
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

echo -e "${YELLOW}========================================${NC}"
echo -e "${GREEN}✓ WHK-01 Bridge Test PASSED${NC}"
echo -e "${YELLOW}========================================${NC}"
echo ""
exit 0
