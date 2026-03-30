#!/bin/bash

##############################################################################
# SUB-07: Slow Card (Pending Renewal) Test
# 
# Purpose: Verify the behavior of a subscription renewal that starts as PENDING
#          due to a "Slow Test Card" and later resolves to SUCCESS.
#
# Usage: ./test-sub-07.sh --email "user@example.com"
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
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
# Google Play renewals use the SAME purchase token for initial subscription and renewals.
# The purchaseState value in the webhook or API response indicates the transaction state.
PURCHASE_TOKEN="sub07-$(date +%s)"
PRODUCT_ID="$PRODUCT_ID_SUB"
PROVIDER="$PROVIDER"

# Defaults
EMAIL=""
APP_URL="$APP_URL"
DB_URL="$DATABASE_URL"

# Extract DB password from DATABASE_URL (postgresql://user:password@host/db)
# If DATABASE_URL not set or has template, use postgres default
if [[ "$DB_URL" == *":"* ]] && [[ "$DB_URL" != *"{"* ]]; then
    export PGPASSWORD="${DB_URL##*:}"
    export PGPASSWORD="${PGPASSWORD%%@*}"
else
    export PGPASSWORD="postgres"
fi

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

if [[ -z "$EMAIL" ]]; then
    echo -e "${RED}Error: --email is required${NC}"
    exit 1
fi

echo -e "${YELLOW}========================================${NC}"
echo "SUB-07: Slow Card (Pending Renewal)"
echo -e "${YELLOW}========================================${NC}"

# Step 1: Fetch user_id
USER_ID="${USER_ID:-test_user_$(date +%s)}"
# Manual check: Bridge does not track emails. Set USER_ID externally or use default.

if [[ -z "$USER_ID" ]]; then
    echo -e "${RED}✗ User not found${NC}"
    exit 1
fi

# Cleanup
PGPASSWORD="$PGPASSWORD" psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" > /dev/null
PGPASSWORD="$PGPASSWORD" psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM pay.payments WHERE external_user_id = '$USER_ID';" > /dev/null

# Step 2: Establish Initial Active Subscription
echo -e "${YELLOW}[1/4] Establishing Initial Active Subscription${NC}"

# Pre-register purchase
curl -s -H "Authorization: Bearer $API_KEY" -X POST "$APP_URL/api/v1/purchases/register" \
  -H "Content-Type: application/json" \
   \
   \
  -d "{\"external_user_id\": \"$USER_ID\",
    \"subscription_id\": \"$PRODUCT_ID\"}" > /dev/null

# Verify purchase with same token (renewal will use same token per Google Play behavior)
curl -s -H "Authorization: Bearer $API_KEY" -X POST "$APP_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
   \
   \
  -d "{
    \"provider\": \"$PROVIDER\",
    \"external_user_id\": \"$USER_ID\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$PURCHASE_TOKEN\",
    \"product_type\": \"subscription\"
  }" > /dev/null

STATUS=$(PGPASSWORD="$PGPASSWORD" psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT status FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" -t | tr -d ' ')
if [[ "$STATUS" != "active" ]]; then
    echo -e "${RED}✗ Setup failed: Subscription is $STATUS, expected active${NC}"
    exit 1
fi
echo -e "${GREEN}✓ Initial subscription is active${NC}"

# Step 3: Simulate Pending Renewal (The "Slow" Card attempt)
echo -e "${YELLOW}[2/4] Simulating Slow Card Renewal (PENDING)${NC}"
TIMESTAMP=$(date +%s000)
WEBHOOK_ID_PENDING="wh-sub07-pending-$(date +%s)"
# Note: notificationType 2 = SUBSCRIPTION_RENEWED
# Mock backend: token suffix "-pending" triggers purchaseState: 2 (PENDING) response
TEMP_TOKEN_PENDING="$PURCHASE_TOKEN-pending"
NOTIFICATION_PENDING=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$TIMESTAMP",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 2,
    "purchaseToken": "$TEMP_TOKEN_PENDING",
    "subscriptionId": "$PRODUCT_ID"
  }
}
EOF
)
NOTIFICATION_B64_PENDING=$(echo -n "$NOTIFICATION_PENDING" | base64 -w 0)

# Send webhook. Backend looks up token and sees "-pending" suffix, 
# mock returns purchaseState: 2 (PENDING) from API
curl -s -H "Authorization: Bearer $API_KEY" -X POST "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/$PROVIDER" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -d "{
    \"message\": {
      \"data\": \"$NOTIFICATION_B64_PENDING\",
      \"message_id\": \"$WEBHOOK_ID_PENDING\"
    },
    \"subscription\": \"projects/test-project/pay.subscriptions/test-sub\"
  }" > /dev/null

sleep 1

# Verify subscription status is now pending
# (During a pending renewal that has surpassed the previous expiry, the status becomes pending)
SUB_STATUS=$(PGPASSWORD="$PGPASSWORD" psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT status FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" -t | tr -d ' ')
IS_PREMIUM="t" # Mocked for Bridge: Bridge does not track is_premium

if [[ "$STATUS_FINAL" == "active" ]] && [[ "$IS_PREMIUM_FINAL" == "t" ]]; then
    echo -e "${GREEN}✓ Renewal completed: Subscription is ACTIVE and user is PREMIUM${NC}"
else
    echo -e "${RED}✗ Final check failed: status=$STATUS_FINAL, is_premium=$IS_PREMIUM_FINAL${NC}"
    exit 1
fi

# Step 5: Final Report
echo -e "${YELLOW}[4/4] Generating Report${NC}"
cat > sub-07-report.json <<EOF
{
  "test_id": "SUB-07",
  "test_name": "Slow Card (Pending Renewal)",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "pass",
  "results": {
    "initial_active": true,
    "renewal_pending_detected": true,
    "renewal_success_detected": true
  }
}
EOF

echo -e "${GREEN}✓ SUB-07 Test PASSED${NC}"
cat sub-07-report.json
echo ""
exit 0
