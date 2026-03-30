#!/bin/bash

##############################################################################
# SUB-04: Renewal Success After Grace Period Recovery Test
# 
# Purpose: Verify that a subscription that enters grace period and then 
#          recovers is properly processed.
#
# Usage: ./test-sub-04.sh --email "user@example.com" [--replay]
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - DATABASE_URL configured and db accessible
#   - psql installed and in PATH
#
# Test Flow:
#   1. Initial purchase (SUB-01 equivalent)
#   2. Simulate Grace Period webhook (notificationType: 6)
#   3. Verify status is 'in_grace_period' and grace timestamps set
#   4. Simulate Recovery webhook (notificationType: 1)
#   5. Verify status is 'active' and grace timestamps cleared
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
DUMMY_TOKEN="test-subscription-sub04-$(date +%s)"
PRODUCT_ID="$PRODUCT_ID_SUB"
PROVIDER="$PROVIDER"

# Defaults
EMAIL=""
APP_URL="$APP_URL"
DB_URL="$DATABASE_URL"

REPLAY_SUB=false
MOCK_RTDN_GRACE_FIXTURE=""
MOCK_RTDN_RECOVERED_FIXTURE=""

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
            shift 1
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

if [[ "$REPLAY_SUB" == "true" ]]; then
    MOCK_RTDN_GRACE_FIXTURE="tests/gpb/fixtures/sub-04-rtdn-grace.json"
    MOCK_RTDN_RECOVERED_FIXTURE="tests/gpb/fixtures/sub-04-rtdn-recovered.json"
    MOCK_GOOGLE_PURCHASE_RESPONSE_GRACE="tests/gpb/fixtures/sub-04-purchase-response-grace.json"
    MOCK_GOOGLE_PURCHASE_RESPONSE_RECOVERED="tests/gpb/fixtures/sub-01-purchase-response.json"
    echo -e "${YELLOW}[Replay] MOCK_RTDN_GRACE_FIXTURE=${MOCK_RTDN_GRACE_FIXTURE}${NC}"
    echo -e "${YELLOW}[Replay] MOCK_RTDN_RECOVERED_FIXTURE=${MOCK_RTDN_RECOVERED_FIXTURE}${NC}"
    echo -e "${YELLOW}[Replay] MOCK_GOOGLE_PURCHASE_RESPONSE_GRACE=${MOCK_GOOGLE_PURCHASE_RESPONSE_GRACE}${NC}"
    echo -e "${YELLOW}[Replay] MOCK_GOOGLE_PURCHASE_RESPONSE_RECOVERED=${MOCK_GOOGLE_PURCHASE_RESPONSE_RECOVERED}${NC}"
fi

# Validate required inputs
if [[ -z "$EMAIL" ]]; then
    echo -e "${RED}Error: --email is required${NC}"
    echo "Usage: ./test-sub-04.sh --email \"user@example.com\""
    exit 1
fi

echo -e "${YELLOW}========================================${NC}"
echo "SUB-04: Renewal Success After Grace Period Recovery"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: Fetch user_id
echo -e "${YELLOW}[1/6] Fetching user_id for: $EMAIL${NC}"
USER_ID="${USER_ID:-test_user_$(date +%s)}"
# Manual check: Bridge does not track emails. Set USER_ID externally or use default.

if [[ -z "$USER_ID" ]]; then
    echo -e "${RED}✗ User not found${NC}"
    exit 1
fi
echo "User ID: $USER_ID"

# Step 2: Clean up and Initial Purchase
echo -e "${YELLOW}[2/6] Initial Purchase (verify_payment)${NC}"
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" > /dev/null

curl -s -H "Authorization: Bearer $API_KEY" -X POST "$APP_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
   \
   \
  -d "{
    \"provider\": \"$PROVIDER\",
    \"external_user_id\": \"$USER_ID\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$DUMMY_TOKEN\",
    \"product_type\": \"subscription\"
  }" > /dev/null

echo -e "${GREEN}✓ Initial purchase verified${NC}"

# Step 3: Simulate Grace Period Webhook (Type 6)
echo -e "${YELLOW}[3/6] Sending Grace Period Webhook (Type 6)${NC}"
WEBHOOK_ID_GRACE="wh-sub04-grace-$(date +%s)"
TIMESTAMP=$(date +%s000)
if [[ "$REPLAY_SUB" == "true" && -n "${MOCK_RTDN_GRACE_FIXTURE:-}" && -f "$MOCK_RTDN_GRACE_FIXTURE" ]]; then
    NOTIFICATION_GRACE=$(cat "$MOCK_RTDN_GRACE_FIXTURE" | sed "s/<REDACTED_PURCHASE_TOKEN>/$DUMMY_TOKEN/g" | sed "s/hiha_monthly/$PRODUCT_ID/g")
    echo -e "${YELLOW}[Replay] Loaded RTDN from fixture: $MOCK_RTDN_GRACE_FIXTURE${NC}"
else
NOTIFICATION_GRACE=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$TIMESTAMP",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 6,
    "purchaseToken": "$DUMMY_TOKEN",
    "subscriptionId": "$PRODUCT_ID"
  }
}
EOF
)
fi
NOTIFICATION_B64_GRACE=$(echo -n "$NOTIFICATION_GRACE" | base64 -w 0)

WEBHOOK_EXTRA_HEADERS=()
if [[ "$REPLAY_SUB" == "true" && -n "${MOCK_GOOGLE_PURCHASE_RESPONSE_GRACE:-}" ]]; then
    WEBHOOK_EXTRA_HEADERS+=(-H "X-Mock-Google-Purchase-Response: $MOCK_GOOGLE_PURCHASE_RESPONSE_GRACE")
fi

curl -s -H "Authorization: Bearer $API_KEY" -X POST "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/$PROVIDER" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  "${WEBHOOK_EXTRA_HEADERS[@]}" \
  -d "{
    \"message\": {
      \"data\": \"$NOTIFICATION_B64_GRACE\",
      \"message_id\": \"$WEBHOOK_ID_GRACE\"
    },
    \"subscription\": \"projects/$GCP_PROJECT_ID/pay.subscriptions/google-play-billing\"
  }" > /dev/null

echo "Waiting for async processing..."
sleep 2

# Verify status in DB
DB_STATUS=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT status, google_grace_period_start FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t)
STATUS=$(echo "$DB_STATUS" | awk -F '|' '{print $1}' | tr -d ' ')
GRACE_START=$(echo "$DB_STATUS" | awk -F '|' '{print $2}' | tr -d ' ')

if [[ "$STATUS" == "in_grace_period" ]] && [[ -n "$GRACE_START" ]]; then
    echo -e "${GREEN}✓ Success: Status is 'in_grace_period' and grace period started${NC}"
else
    echo -e "${RED}✗ Failure: Status is '$STATUS', expected 'in_grace_period'${NC}"
    echo "DB Result: $DB_STATUS"
    exit 1
fi

# Step 4: Simulate Recovery Webhook (Type 1)
echo -e "${YELLOW}[4/6] Sending Recovery Webhook (Type 1)${NC}"
WEBHOOK_ID_RECOVER="wh-sub04-recover-$(date +%s)"
if [[ "$REPLAY_SUB" == "true" && -n "${MOCK_RTDN_RECOVERED_FIXTURE:-}" && -f "$MOCK_RTDN_RECOVERED_FIXTURE" ]]; then
    NOTIFICATION_RECOVER=$(cat "$MOCK_RTDN_RECOVERED_FIXTURE" | sed "s/<REDACTED_PURCHASE_TOKEN>/$DUMMY_TOKEN/g" | sed "s/hiha_monthly/$PRODUCT_ID/g")
    echo -e "${YELLOW}[Replay] Loaded RTDN from fixture: $MOCK_RTDN_RECOVERED_FIXTURE${NC}"
else
NOTIFICATION_RECOVER=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$TIMESTAMP",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 1,
    "purchaseToken": "$DUMMY_TOKEN",
    "subscriptionId": "$PRODUCT_ID"
  }
}
EOF
)
fi
NOTIFICATION_B64_RECOVER=$(echo -n "$NOTIFICATION_RECOVER" | base64 -w 0)

WEBHOOK_EXTRA_HEADERS_RECOVER=()
if [[ "$REPLAY_SUB" == "true" && -n "${MOCK_GOOGLE_PURCHASE_RESPONSE_RECOVERED:-}" ]]; then
    WEBHOOK_EXTRA_HEADERS_RECOVER+=(-H "X-Mock-Google-Purchase-Response: $MOCK_GOOGLE_PURCHASE_RESPONSE_RECOVERED")
fi

curl -s -H "Authorization: Bearer $API_KEY" -X POST "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/$PROVIDER" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  "${WEBHOOK_EXTRA_HEADERS_RECOVER[@]}" \
  -d "{
    \"message\": {
      \"data\": \"$NOTIFICATION_B64_RECOVER\",
      \"message_id\": \"$WEBHOOK_ID_RECOVER\"
    },
    \"subscription\": \"projects/$GCP_PROJECT_ID/pay.subscriptions/google-play-billing\"
  }" > /dev/null

echo "Waiting for async processing..."
sleep 2

# Step 5: Final Validation
echo -e "${YELLOW}[5/6] Final Validation${NC}"
DB_FINAL=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT status, google_grace_period_start, google_grace_period_end FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t)
FINAL_STATUS=$(echo "$DB_FINAL" | awk -F '|' '{print $1}' | tr -d ' ')
FINAL_GRACE_START=$(echo "$DB_FINAL" | awk -F '|' '{print $2}' | tr -d ' ')
FINAL_GRACE_END=$(echo "$DB_FINAL" | awk -F '|' '{print $3}' | tr -d ' ')

if [[ "$FINAL_STATUS" == "active" ]]; then
    echo -e "${GREEN}✓ Success: Status is 'active' after recovery${NC}"
else
    echo -e "${RED}✗ Failure: Status is '$FINAL_STATUS', expected 'active'${NC}"
    exit 1
fi

# Verify grace fields are cleared
GRACE_CLEARED=false
if [[ -z "$FINAL_GRACE_START" ]] && [[ -z "$FINAL_GRACE_END" ]]; then
    echo -e "${GREEN}✓ Success: Grace period fields cleared (NULL)${NC}"
    GRACE_CLEARED=true
else
    echo -e "${RED}✗ Failure: Grace fields NOT cleared. Start: '$FINAL_GRACE_START', End: '$FINAL_GRACE_END'${NC}"
fi

# Step 5a: Verify new payment record created with status='success' upon recovery
echo -e "${YELLOW}[5a/6] Verifying payment record for recovery${NC}"
PAYMENT_RECOVERY=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*), status FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' AND status = 'success' GROUP BY status;" -t)
PAYMENT_COUNT=$(echo "$PAYMENT_RECOVERY" | awk -F '|' '{print $1}' | tr -d ' ')
PAYMENT_STATUS=$(echo "$PAYMENT_RECOVERY" | awk -F '|' '{print $2}' | tr -d ' ')

RECOVERY_PAYMENT_CORRECT=false
if [[ -n "$PAYMENT_COUNT" ]] && [[ "$PAYMENT_COUNT" -ge 1 ]]; then
    if [[ "$PAYMENT_STATUS" == "success" ]]; then
        echo -e "${GREEN}✓ Success: New payment record created with status='success'${NC}"
        echo "  Payment count: $PAYMENT_COUNT"
        RECOVERY_PAYMENT_CORRECT=true
    else
        echo -e "${RED}✗ Failure: Payment status is '$PAYMENT_STATUS', expected 'success'${NC}"
    fi
else
    echo -e "${RED}✗ Failure: No payment record found with status='success'${NC}"
fi
echo ""

# Step 6: Generate report
TEST_STATUS="pass"
if [[ "$RECOVERY_PAYMENT_CORRECT" != "true" ]] || [[ "$GRACE_CLEARED" != "true" ]]; then
    TEST_STATUS="fail"
fi

cat > sub-04-report.json <<EOF
{
  "test_id": "SUB-04",
  "test_name": "Renewal Success After Grace Period Recovery",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "results": {
    "initial_active": true,
    "entered_grace_period": true,
    "recovered_to_active": true,
    "recovery_payment_created_with_success_status": $RECOVERY_PAYMENT_CORRECT,
    "grace_period_fields_cleared": $GRACE_CLEARED
  }
}
EOF

if [[ "$TEST_STATUS" == "pass" ]]; then
    echo -e "${GREEN}✓ SUB-04 Test PASSED${NC}"
else
    echo -e "${RED}✗ SUB-04 Test FAILED${NC}"
fi
cat sub-04-report.json
echo ""

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi

exit 0
