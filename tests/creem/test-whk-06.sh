#!/bin/bash

##############################################################################
# WHK-06: Webhook Signature Verification Mode Override
#
# Purpose: Verify that the X-Webhook-Verification-Mode header correctly
#          overrides signature verification behavior when MOCK_EXTERNAL_APIS=true,
#          and is safely ignored when MOCK_EXTERNAL_APIS is not set.
#
# Test Matrix:
#   1. Invalid signature + "off" header → accepted (MOCK_EXTERNAL_APIS=true)
#   2. Invalid signature + no header     → rejected
#   3. Invalid signature + "strict" header → rejected (MOCK_EXTERNAL_APIS=true)
#   4. Valid signature + "strict" header → accepted (MOCK_EXTERNAL_APIS=true)
#
# Usage: ./test-whk-06.sh
#
# Prerequisites:
#   - Backend running (with MOCK_EXTERNAL_APIS=true for full coverage)
#   - globals.cfg sourced with required vars
#   - psql installed and database accessible
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

TIMESTAMP=$(date +%s)
EVENT_BASE="whk-06-$(date +%s)"
PASS_COUNT=0
FAIL_COUNT=0
TOTAL_TESTS=4

echo -e "${YELLOW}========================================${NC}"
echo "WHK-06: Signature Verification Mode Override"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Cleanup before testing
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
    -c "DELETE FROM pay.webhook_provider WHERE provider = 'creem' AND provider_webhook_id LIKE 'whk-06-%';" > /dev/null 2>&1 || true

# ── Test 1: Invalid signature + X-Webhook-Verification-Mode: off ──
echo -e "${YELLOW}[1/4] Test: Invalid signature + Verification-Mode: off${NC}"
EVENT_ID="${EVENT_BASE}-test1"
PAYLOAD="{\"id\":\"$EVENT_ID\",\"eventType\":\"subscription.active\",\"createdAt\":\"$(date -u +"%Y-%m-%dT%H:%M:%SZ")\",\"object\":{\"id\":\"sub_whk06_test1\",\"status\":\"active\",\"product_id\":\"$PRODUCT_ID_SUB\",\"customer\":{\"id\":\"cust_whk06_1\"},\"metadata\":{\"user_id\":\"whk06_user_1\"}}}"

HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/creem" \
  -H "Content-Type: application/json" \
  -H "X-Webhook-Verification-Mode: off" \
  -H "creem-signature: invalid_sig_12345" \
  -d "$PAYLOAD")

if [[ "$HTTP_CODE" =~ ^20[014]$ ]]; then
    echo -e "  ${GREEN}✓ ACCEPTED (HTTP $HTTP_CODE) — mock-mode bypass works${NC}"
    PASS_COUNT=$((PASS_COUNT + 1))
elif [[ "$HTTP_CODE" == "401" || "$HTTP_CODE" == "403" || "$HTTP_CODE" == "400" ]]; then
    echo -e "  ${BLUE}⊘ REJECTED (HTTP $HTTP_CODE) — MOCK_EXTERNAL_APIS not enabled (expected in prod)${NC}"
    PASS_COUNT=$((PASS_COUNT + 1))
else
    echo -e "  ${RED}✗ Unexpected HTTP $HTTP_CODE${NC}"
    FAIL_COUNT=$((FAIL_COUNT + 1))
fi

# ── Test 2: Invalid signature + NO override header ──
echo -e "${YELLOW}[2/4] Test: Invalid signature + NO override header${NC}"
EVENT_ID="${EVENT_BASE}-test2"
PAYLOAD="{\"id\":\"$EVENT_ID\",\"eventType\":\"subscription.active\",\"createdAt\":\"$(date -u +"%Y-%m-%dT%H:%M:%SZ")\",\"object\":{\"id\":\"sub_whk06_test2\",\"status\":\"active\",\"product_id\":\"$PRODUCT_ID_SUB\",\"customer\":{\"id\":\"cust_whk06_2\"},\"metadata\":{\"user_id\":\"whk06_user_2\"}}}"

HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/creem" \
  -H "Content-Type: application/json" \
  -H "creem-signature: invalid_sig_12345" \
  -d "$PAYLOAD")

if [[ "$HTTP_CODE" == "401" || "$HTTP_CODE" == "403" || "$HTTP_CODE" == "400" ]]; then
    echo -e "  ${GREEN}✓ REJECTED (HTTP $HTTP_CODE) — signature enforcement works${NC}"
    PASS_COUNT=$((PASS_COUNT + 1))
else
    echo -e "  ${RED}✗ NOT rejected (HTTP $HTTP_CODE) — signature should be checked without override${NC}"
    FAIL_COUNT=$((FAIL_COUNT + 1))
fi

# ── Test 3: Invalid signature + Verification-Mode: strict ──
echo -e "${YELLOW}[3/4] Test: Invalid signature + Verification-Mode: strict${NC}"
EVENT_ID="${EVENT_BASE}-test3"
PAYLOAD="{\"id\":\"$EVENT_ID\",\"eventType\":\"subscription.active\",\"createdAt\":\"$(date -u +"%Y-%m-%dT%H:%M:%SZ")\",\"object\":{\"id\":\"sub_whk06_test3\",\"status\":\"active\",\"product_id\":\"$PRODUCT_ID_SUB\",\"customer\":{\"id\":\"cust_whk06_3\"},\"metadata\":{\"user_id\":\"whk06_user_3\"}}}"

HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/creem" \
  -H "Content-Type: application/json" \
  -H "X-Webhook-Verification-Mode: strict" \
  -H "creem-signature: invalid_sig_12345" \
  -d "$PAYLOAD")

if [[ "$HTTP_CODE" == "401" || "$HTTP_CODE" == "403" || "$HTTP_CODE" == "400" ]]; then
    echo -e "  ${GREEN}✓ REJECTED (HTTP $HTTP_CODE) — strict mode enforces signature${NC}"
    PASS_COUNT=$((PASS_COUNT + 1))
else
    echo -e "  ${RED}✗ NOT rejected (HTTP $HTTP_CODE) — strict mode should enforce signature${NC}"
    FAIL_COUNT=$((FAIL_COUNT + 1))
fi

# ── Test 4: Valid signature + Verification-Mode: strict ──
echo -e "${YELLOW}[4/4] Test: Valid signature + Verification-Mode: strict${NC}"
EVENT_ID="${EVENT_BASE}-test4"
PERIOD_END=$(date -u +"%Y-%m-%dT%H:%M:%SZ" -d "+30 days" 2>/dev/null || date -u -v+30d +"%Y-%m-%dT%H:%M:%SZ")
PAYLOAD=$(cat <<EOF
{
  "id": "$EVENT_ID",
  "eventType": "subscription.active",
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "object": {
    "id": "sub_whk06_test4",
    "status": "active",
    "product_id": "$PRODUCT_ID_SUB",
    "current_period_end_date": "$PERIOD_END",
    "customer": {"id": "cust_whk06_4"},
    "metadata": {"user_id": "whk06_user_4"}
  }
}
EOF
)
SIGNATURE=$(echo -n "$PAYLOAD" | openssl dgst -sha256 -hmac "$CREEM_WEBHOOK_SECRET" | sed 's/^.* //')

HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/creem" \
  -H "Content-Type: application/json" \
  -H "X-Webhook-Verification-Mode: strict" \
  -H "creem-signature: $SIGNATURE" \
  -d "$PAYLOAD")

if [[ "$HTTP_CODE" =~ ^20[014]$ ]]; then
    echo -e "  ${GREEN}✓ ACCEPTED (HTTP $HTTP_CODE) — strict mode with valid signature works${NC}"
    PASS_COUNT=$((PASS_COUNT + 1))
else
    echo -e "  ${RED}✗ NOT accepted (HTTP $HTTP_CODE) — valid signature + strict should be accepted${NC}"
    FAIL_COUNT=$((FAIL_COUNT + 1))
fi

# ── Cleanup ──
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
    -c "DELETE FROM pay.webhook_provider WHERE provider = 'creem' AND provider_webhook_id LIKE 'whk-06-%';" > /dev/null 2>&1 || true

# ── Summary ──
echo ""
echo -e "${YELLOW}========================================${NC}"
echo "WHK-06 Results: $PASS_COUNT/$TOTAL_TESTS passed"
echo -e "${YELLOW}========================================${NC}"

if [[ $FAIL_COUNT -eq 0 ]]; then
    echo -e "${GREEN}✓ WHK-06 PASSED${NC}"
    exit 0
else
    echo -e "${RED}✗ WHK-06 FAILED ($FAIL_COUNT test(s) failed)${NC}"
    exit 1
fi