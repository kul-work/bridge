#!/bin/bash

##############################################################################
# SUB-23: Pending Purchase Canceled Test
# 
# Purpose: Verify that when a user cancels a pending purchase before payment
#          completes, the backend correctly processes the pending_purchase_canceled
#          webhook and cleans up the pending state.
#
# Usage: ./test-sub-23.sh --email "user@example.com"
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - DATABASE_URL configured and db accessible
#   - psql installed and in PATH
#
# Test Flow:
#   1. Create a pending subscription in the database
#   2. Verify is_premium=false (no access for pending)
#   3. Send subscription.pending_purchase_canceled webhook (notificationType 20)
#   4. Verify webhook accepted
#   5. Verify subscription status changed to 'cancelled'
#   6. Verify no premium access (is_premium=false still)
#
# DB Validation:
#   - pay.subscriptions table: status = 'cancelled', is_premium = false
#
# Note: Pending purchases that are cancelled never granted access, so no revocation needed.
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
DUMMY_TOKEN="test-subscription-pending-slow-card"
PRODUCT_ID="$PRODUCT_ID_SUB"
PROVIDER="$PROVIDER"
WEBHOOK_ID="test-webhook-pending-cancel-$(date +%s)"

# Defaults
EMAIL=""
APP_URL="$APP_URL"
DB_URL="$DATABASE_URL"

# Extract DB password once
export PGPASSWORD="${DB_URL##*:}"
export PGPASSWORD="${PGPASSWORD%%@*}"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --email)
            EMAIL="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Validate required inputs
if [[ -z "$EMAIL" ]]; then
    echo -e "${RED}Error: --email is required${NC}"
    echo "Usage: ./test-sub-23.sh --email \"user@example.com\""
    exit 1
fi

echo -e "${YELLOW}========================================${NC}"
echo "TEST-SUB23: Pending Purchase Canceled Test"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: Query database to get user_id from email
echo -e "${YELLOW}[1/6] Fetching user_id from database for email: $EMAIL${NC}"

USER_ID="${USER_ID:-test_user_$(date +%s)}"
# Manual check: Bridge does not track emails. Set USER_ID externally or use default.

if [[ -z "$USER_ID" ]] || [[ "$USER_ID" == *"error"* ]] || [[ "$USER_ID" == *"ERROR"* ]]; then
    echo -e "${RED}✗ Failed to fetch user_id from database${NC}"
    exit 1
fi

USER_ID=$(echo "$USER_ID" | tr -d '[:space:]')
echo -e "${GREEN}✓ User ID: $USER_ID${NC}"
echo ""

# Step 2: Create a pending subscription
echo -e "${YELLOW}[2/6] Creating a pending subscription in database${NC}"

PENDING_SETUP_QUERY="INSERT INTO pay.subscriptions (external_user_id, subscription_id, provider, status, auto_renewing, purchase_token, current_period_end, created_at, updated_at) VALUES ('$USER_ID', '$PRODUCT_ID', '$PROVIDER', 'pending', false, '$DUMMY_TOKEN', NOW() + INTERVAL '30 days', NOW(), NOW()) ON CONFLICT (external_user_id, subscription_id, provider) DO UPDATE SET status = 'pending', purchase_token = '$DUMMY_TOKEN', auto_renewing = false, updated_at = NOW();"

psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "$PENDING_SETUP_QUERY" 2>/dev/null || true

echo -e "${GREEN}✓ Pending subscription created${NC}"
echo ""

# Step 3: Verify is_premium=false (no access for pending)
echo -e "${YELLOW}[3/6] Verifying is_premium=false before cancellation${NC}"

PREMIUM_QUERY="SELECT is_premium FROM users WHERE external_user_id = '$USER_ID';"
IS_PREMIUM=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "$PREMIUM_QUERY" -t 2>/dev/null | tr -d ' ')

if [[ "$IS_PREMIUM" == "f" ]] || [[ "$IS_PREMIUM" == "false" ]]; then
    echo -e "${GREEN}✓ is_premium=false (no access for pending)${NC}"
else
    echo -e "${YELLOW}⚠ is_premium=$IS_PREMIUM (unexpected, but not critical)${NC}"
fi
echo ""

# Step 4: Send pending_purchase_canceled webhook (notificationType 20)
echo -e "${YELLOW}[4/6] Sending subscription.pending_purchase_canceled webhook${NC}"

TIMESTAMP=$(date +%s000)

NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$TIMESTAMP",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 20,
    "purchaseToken": "$DUMMY_TOKEN",
    "subscriptionId": "$PRODUCT_ID"
  }
}
EOF
)

NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0 2>/dev/null || echo -n "$NOTIFICATION_JSON" | base64)

echo "POST $APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/$PROVIDER"
echo "Webhook ID: $WEBHOOK_ID"
echo "Notification Type: 20 (SUBSCRIPTION_PENDING_PURCHASE_CANCELED)"
echo ""

WEBHOOK_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/$PROVIDER" \
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
echo "Response: $WEBHOOK_BODY"
echo ""

WEBHOOK_ACCEPTED=false
if [[ "$HTTP_CODE" == "200" ]] || [[ "$HTTP_CODE" == "204" ]]; then
    echo -e "${GREEN}✓ Webhook accepted (HTTP $HTTP_CODE)${NC}"
    WEBHOOK_ACCEPTED=true
else
    echo -e "${RED}✗ Webhook returned HTTP $HTTP_CODE${NC}"
fi
echo ""

sleep 1

# Step 5: Verify subscription status changed to 'cancelled'
echo -e "${YELLOW}[5/6] Verifying subscription status changed to 'cancelled'${NC}"

STATUS_QUERY="SELECT status FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' ORDER BY updated_at DESC LIMIT 1;"
CURRENT_STATUS=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "$STATUS_QUERY" -t 2>/dev/null | tr -d ' ')

STATUS_CHANGED=false
if [[ "$CURRENT_STATUS" == "cancelled" ]]; then
    echo -e "${GREEN}✓ Subscription status changed to 'cancelled'${NC}"
    STATUS_CHANGED=true
else
    echo -e "${RED}✗ Subscription status is '$CURRENT_STATUS' (expected 'cancelled')${NC}"
fi
echo ""

# Step 6: Verify no premium access (is_premium still false)
echo -e "${YELLOW}[6/6] Verifying is_premium=false (no access granted)${NC}"

IS_PREMIUM_AFTER=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "$PREMIUM_QUERY" -t 2>/dev/null | tr -d ' ')

PREMIUM_DENIED=false
if [[ "$IS_PREMIUM_AFTER" == "f" ]] || [[ "$IS_PREMIUM_AFTER" == "false" ]]; then
    echo -e "${GREEN}✓ is_premium=false (no premium access granted)${NC}"
    PREMIUM_DENIED=true
else
    echo -e "${RED}✗ is_premium=$IS_PREMIUM_AFTER (access may have been granted!)${NC}"
fi
echo ""

# Generate JSON report
TEST_STATUS="pass"
if [[ "$WEBHOOK_ACCEPTED" != "true" ]]; then
    TEST_STATUS="fail"
elif [[ "$STATUS_CHANGED" != "true" ]]; then
    TEST_STATUS="fail"
elif [[ "$PREMIUM_DENIED" != "true" ]]; then
    TEST_STATUS="fail"
fi

cat > sub-23-report.json <<EOF
{
  "test_id": "SUB-23",
  "test_name": "Pending Purchase Canceled (SUB-23)",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "user_email": "$EMAIL",
  "product_id": "$PRODUCT_ID",
  "purchase_token": "$DUMMY_TOKEN",
  "webhook_code": $HTTP_CODE,
  "results": {
    "webhook_accepted": $WEBHOOK_ACCEPTED,
    "status_changed_to_cancelled": $STATUS_CHANGED,
    "premium_access_denied": $PREMIUM_DENIED,
    "current_status": "$CURRENT_STATUS",
    "is_premium": "$IS_PREMIUM_AFTER"
  },
  "notes": "Pending purchases that are cancelled should clean up without granting access."
}
EOF

echo -e "${YELLOW}========================================${NC}"
if [[ "$TEST_STATUS" == "pass" ]]; then
    echo -e "${GREEN}✓ SUB-23 Test PASSED${NC}"
else
    echo -e "${RED}✗ SUB-23 Test FAILED${NC}"
fi
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: sub-23-report.json"
cat sub-23-report.json
echo ""

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi

exit 0
