#!/bin/bash

##############################################################################
# ADMIN-WHK-01: Dead-Lettered Webhook Manually Retried
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
TEST_RUN_ID="admin-whk-01-${TIMESTAMP}-$$"
REPORT_FILE="whk-01-report.json"
USER_ID="test_admin_whk_01_user_$TEST_RUN_ID"
PURCHASE_TOKEN="test-admin-whk-01-token-$TEST_RUN_ID"
PROVIDER_WEBHOOK_ID="admin-whk-01-provider-$TEST_RUN_ID"
PRODUCT_ID="test_admin_whk_01_sub"
DLQ_REASON="HTTP 500 Internal Server Error"

echo -e "${YELLOW}========================================${NC}"
echo "ADMIN-WHK-01: Dead-Lettered Webhook Manually Retried"
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

if [[ -z "$BRIDGE_APP_ID" ]]; then
    echo -e "${RED}✗ No enabled app found in database.${NC}"
    exit 1
fi

# 1. Seed dead-lettered delivery
echo -e "${YELLOW}[1/4] Seeding dead-lettered webhook delivery${NC}"

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.webhook_delivery WHERE app_id = '$BRIDGE_APP_ID' AND webhook_provider_id IN (
     SELECT id FROM pay.webhook_provider WHERE provider_webhook_id = '$PROVIDER_WEBHOOK_ID'
   );" 2>/dev/null
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.webhook_provider WHERE provider_webhook_id = '$PROVIDER_WEBHOOK_ID';" 2>/dev/null
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" 2>/dev/null

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, provider, purchase_token, status, auto_renewing, current_period_end)
   VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID', 'google_play', '$PURCHASE_TOKEN', 'active', true, NOW() + INTERVAL '30 days');" 2>/dev/null

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "INSERT INTO pay.webhook_provider (id, app_id, provider, provider_webhook_id, event_type, subscription_id, purchase_token, payload, processed, timestamp_epoch_ms)
   VALUES (gen_random_uuid(), '$BRIDGE_APP_ID', 'google_play', '$PROVIDER_WEBHOOK_ID', 'subscription.paid',
           '$PRODUCT_ID', '$PURCHASE_TOKEN', '{\"test\": \"admin-whk-01\"}', true, ${TIMESTAMP}000);" 2>/dev/null

WHK_PROVIDER_ID=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -At -c \
  "SELECT id FROM pay.webhook_provider WHERE provider_webhook_id = '$PROVIDER_WEBHOOK_ID' AND app_id = '$BRIDGE_APP_ID';" 2>/dev/null | tr -d '[:space:]')

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "INSERT INTO pay.webhook_delivery (id, app_id, webhook_provider_id, forward_attempts, forwarded, dead_lettered, dead_lettered_at, dead_letter_reason, last_http_status, last_error)
   VALUES (gen_random_uuid(), '$BRIDGE_APP_ID', '$WHK_PROVIDER_ID', 3, false, true, NOW(), '$DLQ_REASON', 500, '$DLQ_REASON');" 2>/dev/null

DELIVERY_ID=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -At -c \
  "SELECT id FROM pay.webhook_delivery WHERE webhook_provider_id = '$WHK_PROVIDER_ID' AND app_id = '$BRIDGE_APP_ID';" 2>/dev/null | tr -d '[:space:]')

if [[ -z "$DELIVERY_ID" ]]; then
    echo -e "${RED}✗ Failed to seed webhook delivery${NC}"
    exit 1
fi
echo -e "${GREEN}✓ Seeded delivery: $DELIVERY_ID${NC}"

# 2. Call retry endpoint
echo -e "${YELLOW}[2/4] Calling POST /admin/webhooks/$DELIVERY_ID/retry${NC}"
RETRY_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/admin/webhooks/$DELIVERY_ID/retry" \
  -H "Authorization: Bearer $ADMIN_JWT" \
  -H "Content-Type: application/json" 2>/dev/null || echo "error")

HTTP_CODE=$(echo "$RETRY_RESPONSE" | tail -n1)
BODY=$(echo "$RETRY_RESPONSE" | sed '$d')

echo -e "${BLUE}  HTTP Code: $HTTP_CODE${NC}"
echo -e "${BLUE}  Body: $BODY${NC}"

RETRY_OK="false"
if [[ "$HTTP_CODE" == "200" ]]; then
    echo -e "${GREEN}✓ Retry API returned 200 OK${NC}"
    RETRY_OK="true"
else
    echo -e "${RED}✗ Retry API failed with HTTP $HTTP_CODE${NC}"
fi

# 3. Verify database state
echo -e "${YELLOW}[3/4] Validating database transitions${NC}"
DB_STATE=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -At -c \
  "SELECT dead_lettered || '|' ||
          COALESCE(dead_lettered_at::text, 'NULL') || '|' ||
          COALESCE(NULLIF(dead_letter_reason, ''), 'NULL') || '|' ||
          forward_attempts || '|' ||
          COALESCE(last_error, 'NULL')
   FROM pay.webhook_delivery WHERE id = '$DELIVERY_ID';" 2>/dev/null)

DLQ=$(echo "$DB_STATE" | cut -d'|' -f1)
DLQ_AT=$(echo "$DB_STATE" | cut -d'|' -f2)
DLQ_REASON=$(echo "$DB_STATE" | cut -d'|' -f3)
ATTEMPTS=$(echo "$DB_STATE" | cut -d'|' -f4)
LAST_ERROR=$(echo "$DB_STATE" | cut -d'|' -f5)

DB_OK="true"
if [[ "$DLQ" != "false" ]] || [[ "$DLQ_AT" != "NULL" ]] || [[ "$DLQ_REASON" != "NULL" ]] || [[ "$ATTEMPTS" != "0" ]] || [[ "$LAST_ERROR" != "NULL" ]]; then
    echo -e "${RED}✗ DB state invalid: dead_lettered=$DLQ, attempts=$ATTEMPTS, last_error=$LAST_ERROR${NC}"
    DB_OK="false"
else
    echo -e "${GREEN}✓ DB state successfully reset${NC}"
fi

# Verify no duplicate payments/subs
SUB_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -At -c \
  "SELECT COUNT(*) FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" 2>/dev/null | tr -d '[:space:]')

DUP_OK="true"
if [[ "$SUB_COUNT" != "1" ]]; then
    echo -e "${RED}✗ Found $SUB_COUNT subscription rows (expected 1)${NC}"
    DUP_OK="false"
else
    echo -e "${GREEN}✓ No duplicate subscriptions created${NC}"
fi

# 4. Cleanup
echo -e "${YELLOW}[4/4] Cleanup${NC}"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.webhook_delivery WHERE id = '$DELIVERY_ID';" 2>/dev/null
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.webhook_provider WHERE id = '$WHK_PROVIDER_ID';" 2>/dev/null
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" 2>/dev/null
echo -e "${GREEN}✓ Cleaned up test data${NC}"

TEST_STATUS="fail"
if [[ "$RETRY_OK" == "true" ]] && [[ "$DB_OK" == "true" ]] && [[ "$DUP_OK" == "true" ]]; then
    TEST_STATUS="pass"
fi

TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "ADMIN-WHK-01",
  "test_name": "Dead-Lettered Webhook Manually Retried",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "$TEST_STATUS",
  "http_code": "$HTTP_CODE",
  "results": {
    "retry_ok": $RETRY_OK,
    "db_ok": $DB_OK,
    "no_duplicates": $DUP_OK
  }
}
EOF

echo ""
if [[ "$TEST_STATUS" == "pass" ]]; then
    echo -e "${GREEN}✓ ADMIN-WHK-01 PASSED${NC}"
    exit 0
else
    echo -e "${RED}✗ ADMIN-WHK-01 FAILED${NC}"
    exit 1
fi
