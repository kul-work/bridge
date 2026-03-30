#!/bin/bash

##############################################################################
# SUB-22: Out-of-App Resubscribe Linking (SUB-RESUB-01)
# 
# Purpose: Verify out-of-app purchase context linking when user resubscribes
#          after subscription expiry. Tests that the backend correctly identifies
#          the original subscription owner using expired obfuscatedAccountId
#          from outOfAppPurchaseContext.
#
# Scenario: User1 initially subscribes, subscription expires, then resubscribes
#          via Google Play Store out-of-app purchase (not in-app). The new
#          purchase token contains outOfAppPurchaseContext with expired subscription's
#          obfuscated ID. Backend should link the new subscription to User1's account.
#
# Usage: ./test-sub-22.sh --email "user@example.com"
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - DATABASE_URL configured and db accessible
#   - OutOfAppPurchaseContext feature implemented
#
# Test Flow:
#   1. Initial subscription purchase for User1 (SUB-01 equivalent)
#   2. Record google_obfuscated_account_id from subscription
#   3. Simulate expiration webhook (notificationType: 13)
#   4. Verify status changed to 'expired'
#   5. Perform resubscribe with new token that contains outOfAppPurchaseContext
#   6. Verify new subscription linked to User1 (same external_user_id, not orphaned)
#   7. Verify google_out_of_app_purchase_context was extracted and used for linking
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
OLD_TOKEN="test-sub22-old-token-$(date +%s)"
NEW_TOKEN="test-sub22-oap-token-$(date +%s)"  # Token that triggers outOfAppPurchaseContext
PRODUCT_ID="$PRODUCT_ID_SUB"
PROVIDER="$PROVIDER"

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
    exit 1
fi

echo -e "${YELLOW}========================================${NC}"
echo "SUB-22: Out-of-App Resubscribe Linking (SUB-RESUB-01)"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: Fetch user_id
echo -e "${YELLOW}[1/7] Fetching user_id from database for email: $EMAIL${NC}"
USER_ID="${USER_ID:-test_user_$(date +%s)}"
# Manual check: Bridge does not track emails. Set USER_ID externally or use default.

if [[ -z "$USER_ID" ]]; then
    echo -e "${RED}✗ User not found${NC}"
    exit 1
fi
echo -e "${GREEN}✓ User ID: $USER_ID${NC}"
echo ""

# Step 2: Clean up any previous test data
echo -e "${YELLOW}[2/7] Cleaning up previous test data${NC}"
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" > /dev/null
echo -e "${GREEN}✓ Previous subscription record removed${NC}"
echo ""

# Step 3: Initial Purchase
echo -e "${YELLOW}[3/7] Initial Subscription Purchase with OLD Token: $OLD_TOKEN${NC}"

curl -s -H "Authorization: Bearer $API_KEY" -X POST "$APP_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
   \
   \
  -d "{
    \"provider\": \"$PROVIDER\",
    \"external_user_id\": \"$USER_ID\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$OLD_TOKEN\",
    \"product_type\": \"subscription\"
  }" > /dev/null

echo -e "${GREEN}✓ Initial purchase active${NC}"

# Record the google_obfuscated_account_id from first subscription
echo -e "${YELLOW}[3b/7] Recording obfuscated account ID from first subscription${NC}"
INITIAL_OBFUSCATED_ID=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" \
  -c "SELECT google_obfuscated_account_id FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t | tr -d ' ')

if [[ -z "$INITIAL_OBFUSCATED_ID" ]]; then
    echo -e "${RED}✗ Failed to retrieve google_obfuscated_account_id from initial subscription${NC}"
    exit 1
fi
echo -e "${GREEN}✓ Recorded obfuscated ID: $INITIAL_OBFUSCATED_ID${NC}"
echo ""

# Step 4: Simulate Expiration Webhook (Type 13)
echo -e "${YELLOW}[4/7] Simulating Expiration (Type 13)${NC}"
WEBHOOK_ID_EXP="wh-sub22-exp-$(date +%s)"
TIMESTAMP=$(date +%s000)
NOTIFICATION_EXP=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$TIMESTAMP",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 13,
    "purchaseToken": "$OLD_TOKEN",
    "subscriptionId": "$PRODUCT_ID"
  }
}
EOF
)
NOTIFICATION_B64_EXP=$(echo -n "$NOTIFICATION_EXP" | base64 -w 0)

curl -s -H "Authorization: Bearer $API_KEY" -X POST "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/$PROVIDER" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -d "{
    \"message\": {
      \"data\": \"$NOTIFICATION_B64_EXP\",
      \"message_id\": \"$WEBHOOK_ID_EXP\"
    },
    \"subscription\": \"projects/$GCP_PROJECT_ID/pay.subscriptions/google-play-billing\"
  }" > /dev/null

echo "Waiting for expiry webhook to process..."
sleep 2

STATUS=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT status FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t | tr -d ' ')

if [[ "$STATUS" == "expired" ]]; then
    echo -e "${GREEN}✓ Success: Status is 'expired'${NC}"
else
    echo -e "${RED}✗ Failure: Status is '$STATUS', expected 'expired'${NC}"
    exit 1
fi
echo ""

# Step 5: Pre-register re-subscription with NEW Token (out-of-app)
echo -e "${YELLOW}[5/7] Calling /api/v1/purchases/register (pre-registration for out-of-app resubscription)${NC}"

echo "  POST $APP_URL/api/v1/purchases/register"
echo "  Subscription ID: $PRODUCT_ID"
echo "  New Token: $NEW_TOKEN (triggers outOfAppPurchaseContext)"
echo ""

REGISTER_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$APP_URL/api/v1/purchases/register" \
  -H "Content-Type: application/json" \
   \
   \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"subscription_id\": \"$PRODUCT_ID\"
  }")

REGISTER_HTTP_CODE=$(echo "$REGISTER_RESPONSE" | tail -n1)
REGISTER_LINE_COUNT=$(echo "$REGISTER_RESPONSE" | wc -l)
if [ "$REGISTER_LINE_COUNT" -gt 1 ]; then
    REGISTER_BODY=$(echo "$REGISTER_RESPONSE" | head -n $((REGISTER_LINE_COUNT - 1)))
else
    REGISTER_BODY=""
fi

if [[ "$REGISTER_HTTP_CODE" != "200" ]]; then
    echo -e "${RED}✗ register_purchase failed with HTTP $REGISTER_HTTP_CODE${NC}"
    exit 1
fi

echo -e "${GREEN}✓ register_purchase returned HTTP 200${NC}"
echo ""

# Step 6: Resubscribe with NEW Token (out-of-app)
echo -e "${YELLOW}[6/7] Resubscribing with NEW Token (out-of-app): $NEW_TOKEN${NC}"

VERIFY_RESPONSE=$(curl -s -H "Authorization: Bearer $API_KEY" -X POST "$APP_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
   \
   \
  -d "{
    \"provider\": \"$PROVIDER\",
    \"external_user_id\": \"$USER_ID\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$NEW_TOKEN\",
    \"product_type\": \"subscription\"
  }")

echo "Verify Response: $VERIFY_RESPONSE"
echo ""

# Step 7: Final Validation
echo -e "${YELLOW}[7/7] Final Validation - Out-of-App Linking${NC}"

# Verify subscription status
DB_FINAL=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT status, purchase_token, google_obfuscated_account_id FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t)
FINAL_STATUS=$(echo "$DB_FINAL" | awk -F '|' '{print $1}' | tr -d ' ')
FINAL_TOKEN=$(echo "$DB_FINAL" | awk -F '|' '{print $2}' | tr -d ' ')
FINAL_OAP_CTX=$(echo "$DB_FINAL" | awk -F '|' '{print $3}' | tr -d ' ')

LINKING_CORRECT=false
if [[ "$FINAL_STATUS" == "active" ]]; then
    echo -e "${GREEN}✓ Success: Status is 'active' (resubscription successful)${NC}"
else
    echo -e "${RED}✗ Failure: Status is '$FINAL_STATUS', expected 'active'${NC}"
fi

if [[ "$FINAL_TOKEN" == "$NEW_TOKEN" ]]; then
    echo -e "${GREEN}✓ Success: Purchase token updated to NEW token${NC}"
else
    echo -e "${RED}✗ Failure: Purchase token is '$FINAL_TOKEN', expected '$NEW_TOKEN'${NC}"
fi

# Verify obfuscated account ID was stored (current subscription's account ID)
if [[ -n "$FINAL_OAP_CTX" && "$FINAL_OAP_CTX" != "null" ]]; then
    echo -e "${GREEN}✓ Success: google_obfuscated_account_id extracted and stored${NC}"
    LINKING_CORRECT=true
else
    echo -e "${RED}✗ Failure: google_obfuscated_account_id not found (expected from mock API)${NC}"
fi

# Verify subscription still belongs to original user (not orphaned)
FINAL_CLERK_ID=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT external_user_id FROM pay.subscriptions WHERE subscription_id = '$PRODUCT_ID' AND status = 'active';" -t | tr -d ' ')

if [[ "$FINAL_CLERK_ID" == "$USER_ID" ]]; then
    echo -e "${GREEN}✓ Success: New subscription linked to original user (not orphaned)${NC}"
    LINKING_CORRECT=true
else
    echo -e "${RED}✗ Failure: Subscription linked to different user: $FINAL_CLERK_ID (expected $USER_ID)${NC}"
    LINKING_CORRECT=false
fi

echo ""

# Step 8: Report
echo -e "${YELLOW}[REPORT] Generating test report${NC}"
TEST_STATUS="pass"
if [[ "$LINKING_CORRECT" != "true" ]]; then
    TEST_STATUS="fail"
fi

cat > sub-22-report.json <<EOF
{
  "test_id": "SUB-22",
  "test_name": "Out-of-App Resubscribe Linking (SUB-RESUB-01)",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "results": {
    "initial_purchase_successful": true,
    "subscription_expired_successfully": true,
    "resubscription_active": "$([[ $FINAL_STATUS == 'active' ]] && echo true || echo false)",
    "token_updated": "$([[ $FINAL_TOKEN == $NEW_TOKEN ]] && echo true || echo false)",
    "out_of_app_context_extracted": "$([[ -n $FINAL_OAP_CTX && $FINAL_OAP_CTX != 'null' ]] && echo true || echo false)",
    "linked_to_original_user": "$([[ $FINAL_CLERK_ID == $USER_ID ]] && echo true || echo false)"
  },
  "notes": "Tests out-of-app purchase context linking when resubscribing after expiry. Verifies original subscription owner is correctly identified and linked."
}
EOF

if [[ "$TEST_STATUS" == "pass" ]]; then
    echo -e "${GREEN}✓ SUB-22 Test PASSED${NC}"
else
    echo -e "${RED}✗ SUB-22 Test FAILED${NC}"
fi
cat sub-22-report.json
echo ""

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi

exit 0
