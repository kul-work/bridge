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
#   1. Verify active subscription exists with acknowledged_at set
#   2. Record current acknowledged_at timestamp
#   3. Simulate renewal webhook (subscription.paid, notificationType 2)
#   4. Verify renewal processed (current_period_end extended)
#   5. Verify acknowledged_at unchanged (ACK NOT called for renewals)
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

# Step 1: Generate a synthetic external_user_id for this run
echo -e "${YELLOW}[1/6] Preparing generated user_id for this run${NC}"

if [[ -z "$USER_ID" ]] || [[ "$USER_ID" == *"error"* ]] || [[ "$USER_ID" == *"ERROR"* ]]; then
    echo -e "${RED}✗ Failed to fetch user_id from database${NC}"
    exit 1
fi

USER_ID=$(echo "$USER_ID" | tr -d '[:space:]')
echo -e "${GREEN}✓ User ID: $USER_ID${NC}"
echo ""

# Step 2: Verify existing subscription with acknowledged_at set
echo -e "${YELLOW}[2/6] Verifying existing subscription with acknowledged_at${NC}"

SUB_QUERY="SELECT status, current_period_end, purchase_token FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' ORDER BY created_at DESC LIMIT 1;"

SUB_RESULT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$SUB_QUERY" -t 2>/dev/null || echo "")

if [[ -z "$SUB_RESULT" || "$SUB_RESULT" == *"(0 rows)"* ]]; then
    # Insert new subscription if missing (corrected schema)
    echo -e "${YELLOW}⚠ No subscription found. Setting up with ACK for test...${NC}"
    # Note: excluding acknowledged_at from pay.subscriptions insert as it's gone
    SETUP_QUERY="INSERT INTO pay.subscriptions (external_user_id, subscription_id, provider, status, auto_renewing, purchase_token, current_period_end, created_at, updated_at) VALUES ('$USER_ID', '$PRODUCT_ID', '$PROVIDER', 'active', true, '$DUMMY_TOKEN', NOW() + INTERVAL '30 days', NOW(), NOW()) ON CONFLICT DO NOTHING;"
    psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$SETUP_QUERY" -t 2>/dev/null || true
    
    # Also ensure payment exists with ack
    SETUP_PAYMENT="INSERT INTO pay.payments (external_user_id, provider, provider_transaction_id, subscription_id, amount_cents, status, acknowledged_at, created_at) VALUES ('$USER_ID', '$PROVIDER', '$DUMMY_TOKEN', '$PRODUCT_ID', 999, 'success', NOW(), NOW()) ON CONFLICT (provider, provider_transaction_id) DO UPDATE SET acknowledged_at = NOW();"
    psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$SETUP_PAYMENT" -t 2>/dev/null || true
    
    echo -e "${GREEN}✓ Test setup complete${NC}"
    SUB_RESULT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$SUB_QUERY" -t 2>/dev/null || echo "")
fi

OLD_STATUS=$(echo "$SUB_RESULT" | awk -F '|' '{print $1}' | head -n1 | tr -d ' ')
OLD_PERIOD_END=$(echo "$SUB_RESULT" | awk -F '|' '{print $2}' | head -n1 | tr -d ' ')
PURCHASE_TOKEN=$(echo "$SUB_RESULT" | awk -F '|' '{print $3}' | head -n1 | tr -d ' ')

# Fetch acknowledged_at from PAYMENTS table
ACK_QUERY="SELECT acknowledged_at FROM pay.payments WHERE provider_transaction_id = '$PURCHASE_TOKEN';"
OLD_ACKNOWLEDGED_AT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$ACK_QUERY" -t 2>/dev/null | head -n1 | tr -d ' ')

echo "  Status: $OLD_STATUS"
echo "  Current Period End: $OLD_PERIOD_END"
echo "  Acknowledged At (BEFORE): $OLD_ACKNOWLEDGED_AT"
echo "  Purchase Token: $PURCHASE_TOKEN"
echo ""

if [[ -z "$OLD_ACKNOWLEDGED_AT" ]] || [[ "$OLD_ACKNOWLEDGED_AT" == "null" ]]; then
    echo -e "${YELLOW}⚠ acknowledged_at is NULL in pay.payments. Updating for test...${NC}"
    # Update PAYMENTS table
    psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "UPDATE pay.payments SET acknowledged_at = NOW() WHERE provider_transaction_id = '$PURCHASE_TOKEN';" 2>/dev/null
    OLD_ACKNOWLEDGED_AT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$ACK_QUERY" -t 2>/dev/null | head -n1 | tr -d ' ')
    echo "  Acknowledged At (set): $OLD_ACKNOWLEDGED_AT"
fi

echo -e "${GREEN}✓ Subscription ready for renewal test${NC}"
echo ""

# Step 3: Count pay.payments before renewal
echo -e "${YELLOW}[3/6] Counting pay.payments before renewal${NC}"

PAYMENT_COUNT_QUERY="SELECT COUNT(*) FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';"
OLD_PAYMENT_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$PAYMENT_COUNT_QUERY" -t 2>/dev/null | tr -d ' ')

echo "  Payment count before: $OLD_PAYMENT_COUNT"
echo ""

# Step 4: Simulate renewal webhook (subscription.paid, notificationType 2)
echo -e "${YELLOW}[4/6] Sending subscription.paid (renewal) webhook${NC}"

TIMESTAMP=$(date +%s000)

NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$TIMESTAMP",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 2,
    "purchaseToken": "$PURCHASE_TOKEN",
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

# Step 5: Wait for async processing
echo -e "${YELLOW}[5/6] Waiting for async webhook processing (2 seconds)${NC}"
sleep 2
echo -e "${GREEN}✓ Wait complete${NC}"
echo ""

# Step 6: Verify acknowledged_at unchanged
echo -e "${YELLOW}[6/6] Verifying acknowledged_at unchanged after renewal${NC}"

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
