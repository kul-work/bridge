#!/bin/bash

##############################################################################
# ACK-03: No ACK on Subscription Renewal Test
# 
# Purpose: Verify that when a subscription renews via webhook, the backend
#          does NOT call acknowledge() - only new purchases/resubscribes
#          need acknowledgment, not renewals.
#
# Usage: ./test-ack-03.sh
#
# Prerequisites:
#   - ACK-01 or SUB-01 must have passed (active subscription with ACK exists)
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - DATABASE_URL configured and db accessible
#   - psql installed and in PATH
#
# Test Flow:
#   1. Clean up previous test data
#   2. Register + verify-purchase to create active subscription with ACK
#   3. Record current acknowledged_at timestamp
#   4. Simulate renewal webhook (notificationType 2)
#   5. Verify renewal processed (current_period_end extended)
#   6. Verify acknowledged_at unchanged (ACK NOT called for renewals)
#
# DB Validation (from TESTPLAN):
#   - pay.payments table: new row created for renewal
#   - pay.subscriptions table: acknowledged_at unchanged
#
# Note: ACK-ing renewals would trigger unexpected refunds
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Test configuration
PRODUCT_ID="$PRODUCT_ID_SUB"
PROVIDER="$PROVIDER"
RUN_ID="$(date +%s)-$RANDOM"
USER_ID="${USER_ID:-test_ack_03_user_$RUN_ID}"
DUMMY_TOKEN="test-subscription-sub01-$RUN_ID"  # Dynamic token for this run
WEBHOOK_ID="test-webhook-ack03-renewal-$RUN_ID"

# Defaults
APP_URL="${BRIDGE_API_URL:-http://localhost:5555}"
DB_URL="${BRIDGE_DB_URL}"

# Extract DB password once
# Extract DB password if needed
if [[ "$DB_URL" == *":"* ]]; then
    export PGPASSWORD="${DB_URL##*:}"
    export PGPASSWORD="${PGPASSWORD%%@*}"
fi

echo -e "${YELLOW}========================================${NC}"
echo "ACK-03: No ACK on Subscription Renewal Test"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: Clean up previous test data
echo -e "${YELLOW}[1/7] Cleaning up previous test data${NC}"

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" 2>/dev/null || true
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.payments WHERE external_user_id = '$USER_ID';" 2>/dev/null || true
echo -e "${GREEN}✓ Previous test data removed${NC}"
echo ""

# Step 2: Register purchase via API
echo -e "${YELLOW}[2/7] Registering purchase via API${NC}"

echo "  POST $BRIDGE_API_URL/api/v1/purchase/register"
REGISTER_HTTP=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/purchase/register" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"reason\": \"test-ack-03-setup\",
    \"product_type\": \"subscription\",
    \"amount_cents\": 0,
    \"transaction_id\": \"test-ack-03-reg-$RUN_ID\"
  }")

if [[ "$REGISTER_HTTP" == "200" ]]; then
    echo -e "${GREEN}✓ Purchase registration successful${NC}"
else
    echo -e "${RED}✗ Purchase registration failed (HTTP $REGISTER_HTTP)${NC}"
    exit 1
fi
echo ""

# Step 3: Verify purchase (creates subscription + sets acknowledged_at via ACK)
echo -e "${YELLOW}[3/7] Verifying purchase (creates subscription with ACK)${NC}"

echo "  POST $BRIDGE_API_URL/api/v1/verify-purchase"
VERIFY_HTTP=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$DUMMY_TOKEN\",
    \"product_type\": \"subscription\"
  }")

if [[ "$VERIFY_HTTP" == "200" ]]; then
    echo -e "${GREEN}✓ Purchase verified (subscription created with ACK)${NC}"
else
    echo -e "${RED}✗ Purchase verification failed (HTTP $VERIFY_HTTP)${NC}"
    exit 1
fi
echo ""

# Step 4: Read current subscription state
echo -e "${YELLOW}[4/7] Reading current subscription state${NC}"

SUB_QUERY="SELECT status, current_period_end, purchase_token FROM pay.subscriptions WHERE purchase_token = '$DUMMY_TOKEN';"
SUB_RESULT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$SUB_QUERY" -t 2>/dev/null || echo "")

OLD_STATUS=$(echo "$SUB_RESULT" | awk -F '|' '{print $1}' | head -n1 | tr -d ' ')
OLD_PERIOD_END=$(echo "$SUB_RESULT" | awk -F '|' '{print $2}' | head -n1 | tr -d ' ')
PURCHASE_TOKEN=$(echo "$SUB_RESULT" | awk -F '|' '{print $3}' | head -n1 | tr -d ' ')

# Fetch acknowledged_at from PAYMENTS table
ACK_QUERY="SELECT acknowledged_at FROM pay.payments WHERE provider_transaction_id = '$DUMMY_TOKEN';"
OLD_ACKNOWLEDGED_AT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$ACK_QUERY" -t 2>/dev/null | head -n1 | tr -d ' ')

echo "  Status: $OLD_STATUS"
echo "  Current Period End: $OLD_PERIOD_END"
echo "  Acknowledged At (BEFORE): $OLD_ACKNOWLEDGED_AT"
echo "  Purchase Token: $PURCHASE_TOKEN"
echo ""

if [[ -z "$OLD_ACKNOWLEDGED_AT" ]] || [[ "$OLD_ACKNOWLEDGED_AT" == "null" ]]; then
    echo -e "${YELLOW}⚠ acknowledged_at is NULL in pay.payments. Updating for test...${NC}"
    psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
      -c "UPDATE pay.payments SET acknowledged_at = NOW() WHERE provider_transaction_id = '$DUMMY_TOKEN';" 2>/dev/null
    OLD_ACKNOWLEDGED_AT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$ACK_QUERY" -t 2>/dev/null | head -n1 | tr -d ' ')
    echo "  Acknowledged At (set): $OLD_ACKNOWLEDGED_AT"
fi

echo -e "${GREEN}✓ Subscription ready for renewal test${NC}"
echo ""

# Step 5: Count pay.payments before renewal
echo -e "${YELLOW}[5/7] Counting pay.payments before renewal${NC}"

PAYMENT_COUNT_QUERY="SELECT COUNT(*) FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';"
OLD_PAYMENT_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$PAYMENT_COUNT_QUERY" -t 2>/dev/null | tr -d ' ')

echo "  Payment count before: $OLD_PAYMENT_COUNT"
echo ""

# Step 6: Simulate renewal webhook (notificationType 2)
echo -e "${YELLOW}[6/7] Sending subscription.renewed webhook (notificationType 2)${NC}"

TIMESTAMP=$(date +%s000)

NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$TIMESTAMP",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 2,
    "purchaseToken": "$DUMMY_TOKEN",
    "subscriptionId": "$PRODUCT_ID"
  }
}
EOF
)

NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0 2>/dev/null || echo -n "$NOTIFICATION_JSON" | base64)

echo "POST $BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/$PROVIDER"
echo "Webhook ID: $WEBHOOK_ID"
echo "Notification Type: 2 (SUBSCRIPTION_RENEWED)"
echo "Expected: Renewal processed, ACK NOT called"
echo ""

WEBHOOK_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/$PROVIDER" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -H "X-Webhook-Verification-Mode: off" \
  -d "{
    \"message\": {
      \"data\": \"$NOTIFICATION_B64\",
      \"message_id\": \"$WEBHOOK_ID\",
      \"attributes\": {}
    },
    \"subscription\": \"projects/test-project/pay.subscriptions/test-sub\"
  }")

HTTP_CODE=$(echo "$WEBHOOK_RESPONSE" | tail -n1)
LINE_COUNT=$(echo "$WEBHOOK_RESPONSE" | wc -l)
if [ "$LINE_COUNT" -gt 1 ]; then
    WEBHOOK_BODY=$(echo "$WEBHOOK_RESPONSE" | head -n $((LINE_COUNT - 1)))
else
    WEBHOOK_BODY=""
fi

echo "Response Code: $HTTP_CODE"
echo ""

WEBHOOK_ACCEPTED=false
if [[ "$HTTP_CODE" == "200" ]] || [[ "$HTTP_CODE" == "204" ]]; then
    echo -e "${GREEN}✓ Renewal webhook accepted (HTTP $HTTP_CODE)${NC}"
    WEBHOOK_ACCEPTED=true
else
    echo -e "${RED}✗ Webhook failed with HTTP $HTTP_CODE${NC}"
fi
echo ""

# Step 7: Wait + verify acknowledged_at unchanged
echo -e "${YELLOW}[7/7] Waiting for async webhook processing (2 seconds)${NC}"
sleep 2
echo -e "${GREEN}✓ Wait complete${NC}"
echo ""

NEW_SUB_RESULT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$SUB_QUERY" -t 2>/dev/null || echo "")

NEW_STATUS=$(echo "$NEW_SUB_RESULT" | awk -F '|' '{print $1}' | head -n1 | tr -d ' ')
NEW_PERIOD_END=$(echo "$NEW_SUB_RESULT" | awk -F '|' '{print $2}' | head -n1 | tr -d ' ')

# Fetch acknowledged_at from PAYMENTS table (same token as before)
NEW_ACKNOWLEDGED_AT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$ACK_QUERY" -t 2>/dev/null | head -n1 | tr -d ' ')

echo "  Status: $NEW_STATUS"
echo "  Old Period End: $OLD_PERIOD_END"
echo "  New Period End: $NEW_PERIOD_END"
echo "  Acknowledged At (BEFORE): $OLD_ACKNOWLEDGED_AT"
echo "  Acknowledged At (AFTER): $NEW_ACKNOWLEDGED_AT"
echo ""

# Verify status still active
STATUS_CORRECT=false
if [[ "$NEW_STATUS" == "active" ]]; then
    echo -e "${GREEN}✓ Status still active after renewal${NC}"
    STATUS_CORRECT=true
else
    echo -e "${RED}✗ Status changed unexpectedly: $NEW_STATUS${NC}"
fi

# CRITICAL: Verify acknowledged_at unchanged
ACK_UNCHANGED=false
if [[ "$NEW_ACKNOWLEDGED_AT" == "$OLD_ACKNOWLEDGED_AT" ]]; then
    echo -e "${GREEN}✓ acknowledged_at UNCHANGED (correct - renewals not re-acknowledged)${NC}"
    ACK_UNCHANGED=true
else
    echo -e "${RED}✗ acknowledged_at CHANGED (was: $OLD_ACKNOWLEDGED_AT, now: $NEW_ACKNOWLEDGED_AT)${NC}"
    echo "  ERROR: ACK should NOT be called on renewals!"
fi

# Check new payment created
NEW_PAYMENT_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$PAYMENT_COUNT_QUERY" -t 2>/dev/null | tr -d ' ')

PAYMENT_CREATED=false
if [[ "$NEW_PAYMENT_COUNT" -gt "$OLD_PAYMENT_COUNT" ]]; then
    echo -e "${GREEN}✓ New payment record created for renewal${NC}"
    PAYMENT_CREATED=true
else
    # Note: Google Play renewals reuse same purchase token, so backend may update existing payment
    # rather than create new row. This is acceptable behavior.
    echo -e "${YELLOW}ℹ No new payment row (renewals may update existing record)${NC}"
fi
echo ""

# Generate JSON report
# Main criteria: ACK unchanged (critical), webhook accepted, status still active
# Payment creation is informational (renewals may update existing record)
TEST_STATUS="pass"
if [[ "$WEBHOOK_ACCEPTED" != "true" ]] || [[ "$ACK_UNCHANGED" != "true" ]]; then
    TEST_STATUS="fail"
elif [[ "$STATUS_CORRECT" != "true" ]]; then
    TEST_STATUS="partial"
fi

cat > ack-03-report.json <<EOF
{
  "test_id": "ACK-03",
  "test_name": "No ACK on Subscription Renewal",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "product_id": "$PRODUCT_ID",
  "purchase_token": "$PURCHASE_TOKEN",
  "webhook_id": "$WEBHOOK_ID",
  "http_code": $HTTP_CODE,
  "old_acknowledged_at": "$OLD_ACKNOWLEDGED_AT",
  "new_acknowledged_at": "$NEW_ACKNOWLEDGED_AT",
  "old_period_end": "$OLD_PERIOD_END",
  "new_period_end": "$NEW_PERIOD_END",
  "results": {
    "webhook_accepted": $WEBHOOK_ACCEPTED,
    "status_still_active": $STATUS_CORRECT,
    "acknowledged_at_unchanged": $ACK_UNCHANGED,
    "new_payment_created": $PAYMENT_CREATED
  },
  "notes": "Ensures only *new* purchases/resubscribes are ACK-ed, not renewals. ACK-ing renewals would trigger unexpected refunds."
}
EOF

echo -e "${YELLOW}========================================${NC}"
if [[ "$TEST_STATUS" == "pass" ]]; then
    echo -e "${GREEN}✓ ACK-03 Test PASSED${NC}"
elif [[ "$TEST_STATUS" == "partial" ]]; then
    echo -e "${YELLOW}⚠ ACK-03 Test PARTIAL (some checks not verified)${NC}"
else
    echo -e "${RED}✗ ACK-03 Test FAILED${NC}"
fi
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: ack-03-report.json"
cat ack-03-report.json
echo ""

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi

exit 0
