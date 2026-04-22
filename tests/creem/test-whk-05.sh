#!/bin/bash

##############################################################################
# WHK-05: Creem Checkout.Completed Ingress + Normalization
#
# Purpose:
#   Validate one high-leverage Creem path in a single test:
#   1) Ingress signature validation using creem-signature
#   2) webhook_provider persistence
#   3) Processor normalization for checkout.completed (recurring)
#   4) external_user_id resolution from object.checkout.metadata.user_id
#
# Prerequisites:
#   - Bridge server running at BRIDGE_API_URL
#   - tests/creem/globals.cfg configured
#   - App has a Creem provider_config with webhook_secret
##############################################################################

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${YELLOW}========================================${NC}"
echo "WHK-05: Creem checkout.completed ingress + normalization"
echo -e "${YELLOW}========================================${NC}"
echo ""

APP_URL="${BRIDGE_API_URL:-http://localhost:5555}"
APP_ID="${BRIDGE_APP_ID}"

RUN_ID="$(date +%s)-$RANDOM"
EVENT_ID="evt_creem_e2e_${RUN_ID}"
EXTERNAL_USER_ID="test_creem_user_${RUN_ID}"
SUBSCRIPTION_ID="sub_creem_e2e_${RUN_ID}"
PRODUCT_ID="prod_creem_e2e"

echo -e "${YELLOW}[1/6] Loading Creem webhook secret${NC}"
CREEM_WEBHOOK_SECRET=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -t -A -c "SELECT config->>'webhook_secret' FROM pay.provider_configs WHERE app_id = '$APP_ID' AND provider = 'creem' LIMIT 1;")
CREEM_WEBHOOK_SECRET=$(echo "$CREEM_WEBHOOK_SECRET" | tr -d '[:space:]')

if [[ -z "$CREEM_WEBHOOK_SECRET" ]]; then
    echo -e "${RED}[FAIL] Missing Creem webhook_secret in pay.provider_configs${NC}"
    exit 1
fi

echo -e "${GREEN}[OK] Creem webhook_secret loaded${NC}"
echo ""

echo -e "${YELLOW}[2/6] Cleaning old fixtures${NC}"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.webhook_provider WHERE app_id = '$APP_ID' AND provider = 'creem' AND provider_webhook_id = '$EVENT_ID';" >/dev/null 2>&1 || true
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.subscriptions WHERE app_id = '$APP_ID' AND external_user_id = '$EXTERNAL_USER_ID' AND subscription_id = '$SUBSCRIPTION_ID' AND provider = 'creem';" >/dev/null 2>&1 || true

echo -e "${GREEN}[OK] Fixture cleanup done${NC}"
echo ""

echo -e "${YELLOW}[3/6] Building signed recurring checkout.completed payload${NC}"
CREATED_AT=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
PAYLOAD=$(cat <<EOF
{
  "id": "${EVENT_ID}",
  "eventType": "checkout.completed",
  "createdAt": "${CREATED_AT}",
  "object": {
    "id": "co_${RUN_ID}",
    "billing_type": "recurring",
    "product_id": "${PRODUCT_ID}",
    "amount": 1999,
    "checkout": {
      "metadata": {
        "user_id": "${EXTERNAL_USER_ID}"
      }
    },
    "subscription": {
      "id": "${SUBSCRIPTION_ID}",
      "status": "paid",
      "current_period_end_date": "2030-01-01T00:00:00Z"
    }
  }
}
EOF
)

SIGNATURE=$(printf '%s' "$PAYLOAD" | openssl dgst -sha256 -hmac "$CREEM_WEBHOOK_SECRET" | awk '{print $2}')

if [[ -z "$SIGNATURE" ]]; then
    echo -e "${RED}[FAIL] Failed to compute HMAC signature${NC}"
    exit 1
fi

echo -e "${GREEN}[OK] Payload + signature prepared${NC}"
echo ""

echo -e "${YELLOW}[4/6] Sending Creem webhook with creem-signature${NC}"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/creem" \
  -H "Content-Type: application/json" \
  -H "creem-signature: $SIGNATURE" \
  -H "X-Webhook-Verification-Mode: off" \
  -d "$PAYLOAD")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)

if [[ "$HTTP_CODE" != "200" && "$HTTP_CODE" != "204" ]]; then
    echo "Response body: $(echo "$RESPONSE" | head -n -1)"
    echo -e "${RED}[FAIL] Webhook not accepted (HTTP $HTTP_CODE)${NC}"
    exit 1
fi

echo -e "${GREEN}[OK] Webhook accepted (HTTP $HTTP_CODE)${NC}"
echo ""

echo -e "${YELLOW}[5/6] Verifying persisted webhook_provider row${NC}"
sleep 2

read -r PROCESSED SUPPRESSED STORED_EVENT <<<"$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -t -A -F ' ' -c "SELECT processed::text, suppressed::text, event_type FROM pay.webhook_provider WHERE app_id = '$APP_ID' AND provider = 'creem' AND provider_webhook_id = '$EVENT_ID' LIMIT 1;")"

if [[ -z "${STORED_EVENT:-}" ]]; then
    echo -e "${RED}[FAIL] No webhook_provider row persisted for event${NC}"
    exit 1
fi

if [[ "$STORED_EVENT" != "checkout.completed" ]]; then
    echo -e "${RED}[FAIL] Unexpected stored event_type: $STORED_EVENT${NC}"
    exit 1
fi

if [[ "$PROCESSED" != "true" ]]; then
    echo -e "${RED}[FAIL] webhook_provider.processed expected true, got: $PROCESSED${NC}"
    exit 1
fi

if [[ "$SUPPRESSED" != "false" ]]; then
    echo -e "${RED}[FAIL] webhook_provider.suppressed expected false, got: $SUPPRESSED${NC}"
    exit 1
fi

echo -e "${GREEN}[OK] webhook_provider persisted + processed + not suppressed${NC}"
echo ""

echo -e "${YELLOW}[6/6] Verifying user resolution + normalization effect in subscriptions${NC}"
read -r SUB_STATUS SUB_AUTO_RENEW <<<"$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -t -A -F ' ' -c "SELECT status, COALESCE(auto_renewing::text,'null') FROM pay.subscriptions WHERE app_id = '$APP_ID' AND external_user_id = '$EXTERNAL_USER_ID' AND subscription_id = '$SUBSCRIPTION_ID' AND provider = 'creem' LIMIT 1;")"

if [[ -z "${SUB_STATUS:-}" ]]; then
    echo -e "${RED}[FAIL] No subscription row created for resolved external_user_id${NC}"
    exit 1
fi

if [[ "$SUB_STATUS" != "active" ]]; then
    echo -e "${RED}[FAIL] Expected normalized status 'active', got: $SUB_STATUS${NC}"
    exit 1
fi

echo -e "${GREEN}[OK] external_user_id resolved and recurring checkout normalized into active subscription${NC}"
echo ""

echo -e "${GREEN}[PASS] WHK-05 passed${NC}"
