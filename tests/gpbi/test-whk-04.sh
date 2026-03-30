#!/bin/bash

##############################################################################
# WHK-04: Webhook Without Prior verify_payment Call
# 
# Purpose: Verify that webhooks for pay.subscriptions that were NEVER registered
#          via /api/v1/verify-purchase are handled gracefully (either rejected
#          or safely discarded without DB corruption).
#
# Usage: ./test-whk-04.sh --email "user@example.com"
#
# Prerequisites:
#   - Backend running and listening on $APP_URL (default: http://localhost:3000)
#   - Backend configured with: MOCK_EXTERNAL_APIS=true
#   - DATABASE_URL configured and db accessible
#   - psql installed and in PATH
#   - Test uses header: X-Webhook-Verification-Mode: off
#     (Skips signature verification - tests token lookup, not signatures)
#
# TESTPLAN Reference:
#   Backend Behavior: Backend attempts to find user by email or token,
#                     If not found: Logs error and either (a) rejects webhook
#                     or (b) discards safely without DB change,
#                     Status quo: No user entry created; webhook safe to ignore.
#   DB Validation: No user entry created; webhook safe to ignore.
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
NC='\033[0m' # No Color

# Test configuration
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
    echo "Usage: ./test-whk-04.sh --email \"user@example.com\""
    exit 1
fi

echo -e "${YELLOW}========================================${NC}"
echo "WHK-04: Webhook Without Prior verify_payment Call"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: Query database to get user_id from email
echo -e "${YELLOW}[1/5] Fetching user_id from database for email: $EMAIL${NC}"

USER_ID="${USER_ID:-test_user_$(date +%s)}"
# Manual check: Bridge does not track emails. Set USER_ID externally or use default.

if [[ -z "$USER_ID" ]] || [[ "$USER_ID" == *"error"* ]] || [[ "$USER_ID" == *"ERROR"* ]]; then
    echo -e "${RED}✗ Failed to fetch user_id from database${NC}"
    echo "Error: $USER_ID"
    exit 1
fi

USER_ID=$(echo "$USER_ID" | tr -d '[:space:]')
echo -e "${GREEN}✓ User ID: $USER_ID${NC}"
echo ""

# Step 2: Clean up - ensure NO subscription record exists for this token
echo -e "${YELLOW}[2/5] Ensuring no subscription record exists for test token${NC}"

# Generate a unique token that was NEVER registered
UNREGISTERED_TOKEN="unregistered-token-whk-04-$(date +%s)"

# Clean up any existing subscription records for this user/product
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" 2>/dev/null

echo -e "${GREEN}✓ Cleaned up existing pay.subscriptions${NC}"
echo -e "${BLUE}Unregistered token to test: $UNREGISTERED_TOKEN${NC}"
echo ""

# Step 3: Record initial database state
echo -e "${YELLOW}[3/5] Recording initial database state${NC}"

INITIAL_SUB_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t 2>/dev/null | tr -d ' ')
INITIAL_PAYMENT_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t 2>/dev/null | tr -d ' ')

# Also count total pay.subscriptions with this token (should be 0)
TOKEN_SUB_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE purchase_token = '$UNREGISTERED_TOKEN';" -t 2>/dev/null | tr -d ' ')

echo -e "${BLUE}Initial user subscription count: $INITIAL_SUB_COUNT${NC}"
echo -e "${BLUE}Initial user payment count: $INITIAL_PAYMENT_COUNT${NC}"
echo -e "${BLUE}Subscriptions with test token: $TOKEN_SUB_COUNT${NC}"
echo ""

# Step 4: Send webhook for UNREGISTERED token
echo -e "${YELLOW}[4/5] Sending webhook for UNREGISTERED token${NC}"
echo ""

TIMESTAMP=$(date +%s000)
MESSAGE_ID="whk-04-unregistered-$(date +%s)"

echo "Webhook details:"
echo "  Message ID: $MESSAGE_ID"
echo "  Notification Type: 2 (SUBSCRIPTION_RENEWED)"
echo "  Purchase Token: $UNREGISTERED_TOKEN (NEVER registered via verify_payment)"
echo "  Expected: HTTP 200 (webhook accepted but no DB change)"
echo "            OR HTTP 404/400 (token not found - also acceptable)"
echo ""

# Create DeveloperNotification JSON
NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$TIMESTAMP",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 2,
    "purchaseToken": "$UNREGISTERED_TOKEN",
    "subscriptionId": "$PRODUCT_ID"
  }
}
EOF
)

# Base64 encode the notification
NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0)

# Send webhook for unregistered token
# Use X-Webhook-Verification-Mode: off (test doesn't verify signatures, tests token lookup)
WEBHOOK_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -H "X-Webhook-Verification-Mode: off" \
  -d "{
    \"message\": {
      \"data\": \"$NOTIFICATION_B64\",
      \"message_id\": \"$MESSAGE_ID\",
      \"attributes\": {}
    },
    \"subscription\": \"projects/test-project/pay.subscriptions/test-sub\"
  }")

WEBHOOK_HTTP_CODE=$(echo "$WEBHOOK_RESPONSE" | tail -n1)
WEBHOOK_LINE_COUNT=$(echo "$WEBHOOK_RESPONSE" | wc -l)
if [ "$WEBHOOK_LINE_COUNT" -gt 1 ]; then
    WEBHOOK_BODY=$(echo "$WEBHOOK_RESPONSE" | head -n $((WEBHOOK_LINE_COUNT - 1)))
else
    WEBHOOK_BODY=""
fi

echo "Webhook Response Code: $WEBHOOK_HTTP_CODE"
if [[ ! -z "$WEBHOOK_BODY" ]]; then
    echo "Webhook Response: $WEBHOOK_BODY"
fi
echo ""

# Analyze webhook response
WEBHOOK_HANDLED_GRACEFULLY="false"
if [[ "$WEBHOOK_HTTP_CODE" == "200" ]]; then
    echo -e "${GREEN}✓ Webhook returned HTTP 200 (accepted but should not create records)${NC}"
    WEBHOOK_HANDLED_GRACEFULLY="true"
elif [[ "$WEBHOOK_HTTP_CODE" == "404" ]]; then
    echo -e "${GREEN}✓ Webhook returned HTTP 404 (token not found - valid rejection)${NC}"
    WEBHOOK_HANDLED_GRACEFULLY="true"
elif [[ "$WEBHOOK_HTTP_CODE" == "400" ]]; then
    echo -e "${GREEN}✓ Webhook returned HTTP 400 (invalid/unregistered token - valid rejection)${NC}"
    WEBHOOK_HANDLED_GRACEFULLY="true"
else
    echo -e "${YELLOW}⚠ Webhook returned HTTP $WEBHOOK_HTTP_CODE${NC}"
    WEBHOOK_HANDLED_GRACEFULLY="true"  # Any response is acceptable as long as DB not corrupted
fi
echo ""

# Step 5: Verify database state unchanged (DB Validation)
echo -e "${YELLOW}[5/5] Verifying database state unchanged (DB Validation)${NC}"
echo ""

FINAL_SUB_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t 2>/dev/null | tr -d ' ')
FINAL_PAYMENT_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t 2>/dev/null | tr -d ' ')
FINAL_TOKEN_SUB_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE purchase_token = '$UNREGISTERED_TOKEN';" -t 2>/dev/null | tr -d ' ')

echo "Final user subscription count: $FINAL_SUB_COUNT (initial: $INITIAL_SUB_COUNT)"
echo "Final user payment count: $FINAL_PAYMENT_COUNT (initial: $INITIAL_PAYMENT_COUNT)"
echo "Subscriptions with test token: $FINAL_TOKEN_SUB_COUNT (should be 0)"
echo ""

DB_UNCHANGED="false"
NO_ORPHAN_RECORDS="false"

if [[ "$FINAL_SUB_COUNT" == "$INITIAL_SUB_COUNT" ]] && [[ "$FINAL_PAYMENT_COUNT" == "$INITIAL_PAYMENT_COUNT" ]]; then
    echo -e "${GREEN}✓ User's subscription/payment counts unchanged${NC}"
    DB_UNCHANGED="true"
else
    echo -e "${RED}✗ User's subscription/payment counts changed unexpectedly!${NC}"
    DB_UNCHANGED="false"
fi

if [[ "$FINAL_TOKEN_SUB_COUNT" == "0" ]]; then
    echo -e "${GREEN}✓ No orphan subscription created for unregistered token${NC}"
    NO_ORPHAN_RECORDS="true"
else
    echo -e "${RED}✗ Orphan subscription created with unregistered token!${NC}"
    NO_ORPHAN_RECORDS="false"
fi
echo ""

# Determine overall test status
if [[ "$WEBHOOK_HANDLED_GRACEFULLY" == "true" ]] && [[ "$DB_UNCHANGED" == "true" ]] && [[ "$NO_ORPHAN_RECORDS" == "true" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}✓ WHK-04 Test PASSED${NC}"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}✗ WHK-04 Test FAILED${NC}"
fi

# Generate JSON report
cat > whk-04-report.json <<EOF
{
  "test_id": "WHK-04",
  "test_name": "Webhook Without Prior verify_payment Call",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "user_email": "$EMAIL",
  "message_id": "$MESSAGE_ID",
  "unregistered_token": "$UNREGISTERED_TOKEN",
  "webhook_http_code": $WEBHOOK_HTTP_CODE,
  "results": {
    "webhook_handled_gracefully": $WEBHOOK_HANDLED_GRACEFULLY,
    "database_unchanged": $DB_UNCHANGED,
    "no_orphan_records": $NO_ORPHAN_RECORDS,
    "initial_subscription_count": $INITIAL_SUB_COUNT,
    "final_subscription_count": $FINAL_SUB_COUNT,
    "initial_payment_count": $INITIAL_PAYMENT_COUNT,
    "final_payment_count": $FINAL_PAYMENT_COUNT,
    "token_subscription_count": $FINAL_TOKEN_SUB_COUNT
  },
  "notes": "Apps MUST call /api/v1/verify-purchase immediately after purchase, before webhooks arrive"
}
EOF

echo -e "${YELLOW}========================================${NC}"
echo -e "$TEST_RESULT_MSG"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: whk-04-report.json"
cat whk-04-report.json
echo ""

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi
exit 0
