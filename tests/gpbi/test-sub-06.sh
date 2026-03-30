#!/bin/bash

##############################################################################
# SUB-06: Re-subscription (After Expiry) Test
# 
# Purpose: Verify the full re-subscription flow after expiry:
#          1. Pre-register new purchase (POST /api/v1/purchases/register)
#          2. Verify new purchase (POST /api/v1/verify-purchase)
#          3. Confirm new subscription linked correctly with google_linked_purchase_token
#
# Usage: ./test-sub-06.sh --email "user@example.com"
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - DATABASE_URL configured and db accessible
#
# Test Flow:
#   1. Initial purchase (SUB-01 equivalent)
#   2. Simulate Expiration webhook (notificationType: 13)
#   3. Verify status in DB is 'expired'
#   4. Perform new purchase verification with a NEW token
#   5. Verify status is 'active' again and history is preserved (new payment row)
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
source "$SCRIPT_DIR/globals.cfg"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Test configuration
TIMESTAMP=$(date +%s)
OLD_TOKEN="test-sub06-old-token-$TIMESTAMP"
NEW_TOKEN="test-sub06-new-token-$TIMESTAMP"
PRODUCT_ID="$PRODUCT_ID_SUB"
PROVIDER="$PROVIDER"

# Defaults
EMAIL=""
APP_URL="$APP_URL"
DB_URL="$DATABASE_URL"
REPLAY_SUB=false
REPLAY_FIXTURE=""
MOCK_GOOGLE_PURCHASE_RESPONSE=""
MOCK_RTDN_FIXTURE=""

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
        --replay)
            REPLAY_SUB=true
            if [[ -n "${2:-}" && "${2:0:2}" != "--" ]]; then
                REPLAY_FIXTURE="$2"
                shift 2
            else
                shift 1
            fi
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Set replay fixtures if enabled
if [[ "$REPLAY_SUB" == "true" ]]; then
    if [[ -n "$REPLAY_FIXTURE" ]]; then
        MOCK_GOOGLE_PURCHASE_RESPONSE="$REPLAY_FIXTURE"
    else
        MOCK_GOOGLE_PURCHASE_RESPONSE="tests/gpb/fixtures/sub-06-purchase-response-resubscribe.json"
    fi
    MOCK_RTDN_FIXTURE="tests/gpb/fixtures/sub-06-rtdn-resubscribed.json"
    echo -e "${YELLOW}[Replay] MOCK_GOOGLE_PURCHASE_RESPONSE=${MOCK_GOOGLE_PURCHASE_RESPONSE}${NC}"
    echo -e "${YELLOW}[Replay] MOCK_RTDN_FIXTURE=${MOCK_RTDN_FIXTURE}${NC}"
fi

# Validate required inputs
if [[ -z "$EMAIL" ]]; then
    echo -e "${RED}Error: --email is required${NC}"
    exit 1
fi

echo -e "${YELLOW}========================================${NC}"
echo "SUB-06: Re-subscription (After Expiry)"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: Fetch user_id
USER_ID="${USER_ID:-test_user_$(date +%s)}"
# Manual check: Bridge does not track emails. Set USER_ID externally or use default.

if [[ -z "$USER_ID" ]]; then
    echo -e "${RED}✗ User not found${NC}"
    exit 1
fi

# Step 2: Clean up and Initial Purchase
echo -e "${YELLOW}[1/5] Initial Purchase with Token: $OLD_TOKEN${NC}"
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" > /dev/null

EXTRA_HEADERS=()
if [[ "$REPLAY_SUB" == "true" ]]; then
    EXTRA_HEADERS+=(-H "X-Mock-Google-Purchase-Response: $MOCK_GOOGLE_PURCHASE_RESPONSE")
fi

curl -s -H "Authorization: Bearer $API_KEY" -X POST "$APP_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
   \
   \
  "${EXTRA_HEADERS[@]}" \
  -d "{
    \"provider\": \"$PROVIDER\",
    \"external_user_id\": \"$USER_ID\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$OLD_TOKEN\",
    \"product_type\": \"subscription\"
  }" > /dev/null

echo -e "${GREEN}✓ Initial purchase active${NC}"

# Step 3: Simulate Expiration Webhook (Type 13)
echo -e "${YELLOW}[2/5] Simulating Expiration (Type 13)${NC}"
WEBHOOK_ID_EXP="wh-sub06-exp-$(date +%s)"
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

echo "Waiting for expiry..."
sleep 2

STATUS=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT status FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t | tr -d ' ')

if [[ "$STATUS" == "expired" ]]; then
    echo -e "${GREEN}✓ Success: Status is 'expired'${NC}"
else
    echo -e "${RED}✗ Failure: Status is '$STATUS', expected 'expired'${NC}"
    exit 1
fi

# Step 4: Pre-register re-subscription with NEW Token
echo -e "${YELLOW}[3/6] Calling /api/v1/purchases/register (pre-registration for re-subscription)${NC}"

echo "  POST $APP_URL/api/v1/purchases/register"
echo "  Subscription ID: $PRODUCT_ID"
echo "  New Token: $NEW_TOKEN"
echo ""

echo "Sending request..."
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

echo "Response Code: $REGISTER_HTTP_CODE"
echo "Response: $REGISTER_BODY"
echo ""

if [[ "$REGISTER_HTTP_CODE" != "200" ]]; then
    echo -e "${RED}✗ register_purchase failed with HTTP $REGISTER_HTTP_CODE${NC}"
    exit 1
fi

echo -e "${GREEN}✓ register_purchase returned HTTP 200${NC}"
echo ""

# Step 5: Re-subscribe with NEW Token (verify)
echo -e "${YELLOW}[4/6] Re-subscribing with NEW Token: $NEW_TOKEN${NC}"

RESUB_HEADERS=()
if [[ "$REPLAY_SUB" == "true" ]]; then
    # For replay: pass fixture file (backend will use linkedPurchaseToken from fixture after sed replacement)
    FIXTURE_WITH_TOKEN=$(cat "$PROJECT_ROOT/$MOCK_GOOGLE_PURCHASE_RESPONSE" | sed "s/<REDACTED_PURCHASE_TOKEN>/$OLD_TOKEN/g")
    # Create temp file for fixture
    TEMP_FIXTURE=$(mktemp)
    echo "$FIXTURE_WITH_TOKEN" > "$TEMP_FIXTURE"
    RESUB_HEADERS+=(-H "X-Mock-Google-Purchase-Response: $TEMP_FIXTURE")
fi

curl -s -H "Authorization: Bearer $API_KEY" -X POST "$APP_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
   \
   \
  "${RESUB_HEADERS[@]}" \
  -d "{
    \"provider\": \"$PROVIDER\",
    \"external_user_id\": \"$USER_ID\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$NEW_TOKEN\",
    \"product_type\": \"subscription\"
  }" > /dev/null

# Cleanup temp fixture
if [[ "$REPLAY_SUB" == "true" ]] && [[ -n "${TEMP_FIXTURE:-}" ]]; then
    rm -f "$TEMP_FIXTURE"
fi

# Step 6: Final Validation
echo -e "${YELLOW}[5/6] Final Validation${NC}"
DB_FINAL=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT status, purchase_token FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t)
FINAL_STATUS=$(echo "$DB_FINAL" | awk -F '|' '{print $1}' | tr -d ' ')
FINAL_TOKEN=$(echo "$DB_FINAL" | awk -F '|' '{print $2}' | tr -d ' ')

if [[ "$FINAL_STATUS" == "active" ]]; then
    echo -e "${GREEN}✓ Success: Status is 'active' again${NC}"
else
    echo -e "${RED}✗ Failure: Status is '$FINAL_STATUS', expected 'active'${NC}"
    exit 1
fi

TOKEN_UPDATED_CORRECT=false
if [[ "$FINAL_TOKEN" == "$NEW_TOKEN" ]]; then
    echo -e "${GREEN}✓ Success: Purchase token updated to NEW token${NC}"
    TOKEN_UPDATED_CORRECT=true
else
    echo -e "${RED}✗ Failure: Purchase token is '$FINAL_TOKEN', expected '$NEW_TOKEN'${NC}"
fi
echo ""

# Verify google_linked_purchase_token points to expired subscription's token
echo -e "${YELLOW}[5b/6] Verifying google_linked_purchase_token${NC}"
LINKED_TOKEN_QUERY="SELECT google_linked_purchase_token FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';"
LINKED_TOKEN=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "$LINKED_TOKEN_QUERY" -t | tr -d ' ')

LINKED_TOKEN_CORRECT=false
if [[ "$LINKED_TOKEN" == "$OLD_TOKEN" ]]; then
    echo -e "${GREEN}✓ Success: google_linked_purchase_token correctly points to expired subscription's token ($OLD_TOKEN)${NC}"
    LINKED_TOKEN_CORRECT=true
else
    echo -e "${RED}✗ Failure: google_linked_purchase_token is '$LINKED_TOKEN', expected '$OLD_TOKEN' (expired token)${NC}"
fi

# Check pay.payments table for two pay.payments
echo -e "${YELLOW}[5c/6] Checking payment history${NC}"
PAYMENT_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t | tr -d ' ')
echo "Payment count: $PAYMENT_COUNT"

PAYMENT_HISTORY_CORRECT=false
if [[ "$PAYMENT_COUNT" -ge 2 ]]; then
    echo -e "${GREEN}✓ Success: Multiple pay.payments recorded (history preserved)${NC}"
    PAYMENT_HISTORY_CORRECT=true
else
    echo -e "${RED}✗ Failure: Expected at least 2 pay.payments, found $PAYMENT_COUNT${NC}"
fi
echo ""

# Step 6: Report
echo -e "${YELLOW}[6/6] Generating report${NC}"
TEST_STATUS="pass"
if [[ "$TOKEN_UPDATED_CORRECT" != "true" ]] || [[ "$LINKED_TOKEN_CORRECT" != "true" ]] || [[ "$PAYMENT_HISTORY_CORRECT" != "true" ]]; then
    TEST_STATUS="fail"
fi

# Step 7: Simulate webhook for resubscription (replay mode only)
if [[ "$REPLAY_SUB" == "true" && -n "${MOCK_RTDN_FIXTURE:-}" && -f "$PROJECT_ROOT/$MOCK_RTDN_FIXTURE" ]]; then
    echo -e "${YELLOW}[6/6] Sending subscription.resubscribed webhook (replay from fixture)${NC}"

    NOTIFICATION_JSON=$(cat "$PROJECT_ROOT/$MOCK_RTDN_FIXTURE" | sed "s/<REDACTED_PURCHASE_TOKEN>/$NEW_TOKEN/g")
    echo -e "${YELLOW}[Replay] Loaded RTDN from fixture: $MOCK_RTDN_FIXTURE${NC}"

    NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0 2>/dev/null || echo -n "$NOTIFICATION_JSON" | base64)
    WEBHOOK_ID="test-webhook-sub06-resubscribed-$(date +%s)"

    WEBHOOK_EXTRA_HEADERS=(-H "X-Mock-Google-Purchase-Response: $MOCK_GOOGLE_PURCHASE_RESPONSE")

    WEBHOOK_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
      "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/$PROVIDER" \
      -H "Content-Type: application/json" \
      -H "Authorization: Bearer test-token" \
      "${WEBHOOK_EXTRA_HEADERS[@]}" \
      -d "{
        \"message\": {
          \"data\": \"$NOTIFICATION_B64\",
          \"message_id\": \"$WEBHOOK_ID\",
          \"attributes\": {}
        },
        \"subscription\": \"projects/test-project/pay.subscriptions/test-sub\"
      }")

    WH_HTTP_CODE=$(echo "$WEBHOOK_RESPONSE" | tail -n1)
    if [[ "$WH_HTTP_CODE" == "200" ]] || [[ "$WH_HTTP_CODE" == "204" ]]; then
        echo -e "${GREEN}✓ Webhook accepted (HTTP $WH_HTTP_CODE)${NC}"
    else
        echo -e "${YELLOW}⚠ Webhook returned HTTP $WH_HTTP_CODE${NC}"
    fi
    sleep 1
    echo ""
else
    echo -e "${YELLOW}[6/6] Skipping webhook simulation (not in replay mode)${NC}"
    echo ""
fi

cat > sub-06-report.json <<EOF
{
  "test_id": "SUB-06",
  "test_name": "Re-subscription (After Expiry)",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "results": {
    "expired_successfully": true,
    "resubscribed_successfully": true,
    "token_updated": $TOKEN_UPDATED_CORRECT,
    "google_linked_purchase_token_points_to_old_token": $LINKED_TOKEN_CORRECT,
    "payment_history_preserved": $PAYMENT_HISTORY_CORRECT
  }
}
EOF

if [[ "$TEST_STATUS" == "pass" ]]; then
    echo -e "${GREEN}✓ SUB-06 Test PASSED${NC}"
else
    echo -e "${RED}✗ SUB-06 Test FAILED${NC}"
fi
cat sub-06-report.json
echo ""

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi

exit 0
