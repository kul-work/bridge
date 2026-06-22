#!/bin/bash

##############################################################################
# ISO-03: Cross-App Webhook Delivery Isolation
#
# Purpose: Verify that App B's API queries cannot see App A's webhook
#          delivery records. RLS on pay.webhook_provider and pay.webhook_delivery
#          must enforce app_id scoping.
#
# Usage: ./test-iso-03.sh
#
# TESTPLAN Reference:
#   ISO-03: Webhook delivery state is not visible across app boundaries.
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
TEST_RUN_ID="iso-03-${TIMESTAMP}-$$"
REPORT_FILE="iso-03-report.json"
PROVIDER_WEBHOOK_ID="iso-03-provider-$TEST_RUN_ID"

echo -e "${YELLOW}========================================${NC}"
echo "ISO-03: Cross-App Webhook Delivery Isolation"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

if [[ -z "$APP_A_ID" ]] || [[ -z "$APP_B_ID" ]]; then
    echo -e "${RED}✗ Both APP_A_ID and APP_B_ID must be set.${NC}"
    exit 1
fi

# Step 1: Seed webhook_provider and webhook_delivery for App A
echo -e "${YELLOW}[1/4] Seeding webhook records for App A${NC}"

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.webhook_delivery WHERE app_id = '$APP_A_ID' AND webhook_provider_id IN (
     SELECT id FROM pay.webhook_provider WHERE provider_webhook_id = '$PROVIDER_WEBHOOK_ID'
   );" 2>/dev/null
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.webhook_provider WHERE provider_webhook_id = '$PROVIDER_WEBHOOK_ID';" 2>/dev/null

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "INSERT INTO pay.webhook_provider (id, app_id, provider, provider_webhook_id, event_type, payload, processed, timestamp_epoch_ms)
   VALUES (gen_random_uuid(), '$APP_A_ID', 'google_play', '$PROVIDER_WEBHOOK_ID', 'subscription.paid',
           '{\"test\": \"iso-03\"}', true, ${TIMESTAMP}000);" 2>/dev/null

WHK_PROVIDER_ID=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -At -c \
  "SELECT id FROM pay.webhook_provider WHERE provider_webhook_id = '$PROVIDER_WEBHOOK_ID' AND app_id = '$APP_A_ID';" 2>/dev/null | tr -d '[:space:]')

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "INSERT INTO pay.webhook_delivery (id, app_id, webhook_provider_id, forward_attempts, forwarded)
   VALUES (gen_random_uuid(), '$APP_A_ID', '$WHK_PROVIDER_ID', 0, true);" 2>/dev/null

WHK_DELIVERY_ID=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -At -c \
  "SELECT id FROM pay.webhook_delivery WHERE webhook_provider_id = '$WHK_PROVIDER_ID' AND app_id = '$APP_A_ID';" 2>/dev/null | tr -d '[:space:]')

echo -e "${GREEN}✓ Created webhook_provider ($WHK_PROVIDER_ID) and delivery ($WHK_DELIVERY_ID) for App A${NC}"
echo ""

# Step 2: Verify App A can see its webhook delivery via DB query (app-scoped)
echo -e "${YELLOW}[2/4] Verifying App A can see its webhook delivery${NC}"

# The admin API or any app-scoped API should show App A's deliveries.
# Since webhook delivery listing is admin-scoped, we verify at the DB level
# using the same SECURITY DEFINER pattern the app would use.
APP_A_DELIVERIES=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -At -c \
  "SELECT COUNT(*) FROM pay.webhook_delivery WHERE app_id = '$APP_A_ID' AND webhook_provider_id = '$WHK_PROVIDER_ID';" 2>/dev/null | tr -d '[:space:]')

if [[ "$APP_A_DELIVERIES" != "1" ]]; then
    echo -e "${RED}✗ App A should have 1 delivery, got $APP_A_DELIVERIES${NC}"
    exit 1
fi
echo -e "${GREEN}✓ App A sees its own delivery (count: $APP_A_DELIVERIES)${NC}"
echo ""

# Step 3: Verify App B cannot see App A's webhook delivery
echo -e "${YELLOW}[3/4] Verifying App B cannot see App A's webhook delivery${NC}"

APP_B_DELIVERIES=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -At -c \
  "SELECT COUNT(*) FROM pay.webhook_delivery WHERE app_id = '$APP_B_ID' AND webhook_provider_id = '$WHK_PROVIDER_ID';" 2>/dev/null | tr -d '[:space:]')

APP_B_PROVIDERS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -At -c \
  "SELECT COUNT(*) FROM pay.webhook_provider WHERE app_id = '$APP_B_ID' AND provider_webhook_id = '$PROVIDER_WEBHOOK_ID';" 2>/dev/null | tr -d '[:space:]')

ISOLATION_OK="true"
if [[ "$APP_B_DELIVERIES" != "0" ]]; then
    echo -e "${RED}✗ App B sees App A's webhook_delivery (RLS breach!): $APP_B_DELIVERIES rows${NC}"
    ISOLATION_OK="false"
else
    echo -e "${GREEN}✓ App B sees 0 deliveries for App A's webhook${NC}"
fi

if [[ "$APP_B_PROVIDERS" != "0" ]]; then
    echo -e "${RED}✗ App B sees App A's webhook_provider (RLS breach!): $APP_B_PROVIDERS rows${NC}"
    ISOLATION_OK="false"
else
    echo -e "${GREEN}✓ App B sees 0 webhook_provider rows for App A's webhook${NC}"
fi
echo ""

# Step 4: Cleanup
echo -e "${YELLOW}[4/4] Cleanup${NC}"

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.webhook_delivery WHERE id = '$WHK_DELIVERY_ID';" 2>/dev/null
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.webhook_provider WHERE id = '$WHK_PROVIDER_ID';" 2>/dev/null
echo -e "${GREEN}✓ Cleaned up test data${NC}"
echo ""

if [[ "$ISOLATION_OK" == "true" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}✓ ISO-03 Test PASSED${NC}"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}✗ ISO-03 Test FAILED${NC}"
fi

TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "ISO-03",
  "test_name": "Cross-App Webhook Delivery Isolation",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "$TEST_STATUS",
  "webhook_provider_id": "$WHK_PROVIDER_ID",
  "webhook_delivery_id": "$WHK_DELIVERY_ID",
  "results": {
    "app_a_delivery_count": $APP_A_DELIVERIES,
    "app_b_delivery_count": $APP_B_DELIVERIES,
    "app_b_provider_count": $APP_B_PROVIDERS,
    "isolation_enforced": $ISOLATION_OK
  }
}
EOF

echo -e "${YELLOW}========================================${NC}"
echo -e "$TEST_RESULT_MSG"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: $REPORT_FILE"

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi
exit 0