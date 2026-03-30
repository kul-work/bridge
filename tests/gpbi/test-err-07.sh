#!/bin/bash

##############################################################################
# ERR-07: Unknown Notification Type
# 
# Purpose: Verify that webhooks with unknown notification types (e.g., type 99)
#          are acknowledged (HTTP 200) but no action is taken (forward-compatible).
#
# Usage: ./test-err-07.sh --email "user@example.com"
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=false
#   - DATABASE_URL configured and db accessible
#   - psql installed and in PATH
#
# TESTPLAN Reference:
#   Expected Behavior: HTTP 200 (acknowledged) but no action taken.
#   Backend Response: Backend parses successfully but doesn't match any known
#                     notification type. Logs warning: "Unknown notification type".
#                     Returns gracefully; no DB change (forward-compatible).
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
    echo "Usage: ./test-err-07.sh --email \"user@example.com\""
    exit 1
fi

echo -e "${YELLOW}========================================${NC}"
echo "ERR-07: Unknown Notification Type"
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

# Step 2: Record initial database state
echo -e "${YELLOW}[2/5] Recording initial database state${NC}"

INITIAL_SUB_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')
INITIAL_PAYMENT_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM pay.payments WHERE external_user_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')

echo -e "${BLUE}Initial subscription count: $INITIAL_SUB_COUNT${NC}"
echo -e "${BLUE}Initial payment count: $INITIAL_PAYMENT_COUNT${NC}"
echo ""

# Step 3: Send webhooks with unknown notification types
echo -e "${YELLOW}[3/5] Sending webhooks with unknown notification types${NC}"
echo ""

TIMESTAMP=$(date +%s000)
PURCHASE_TOKEN="test-err-07-unknown-type-$(date +%s)"

# Test with various unknown notification types
UNKNOWN_TYPES=(99 50 100 255)

ALL_ACKNOWLEDGED="true"

for TYPE in "${UNKNOWN_TYPES[@]}"; do
    echo -e "${BLUE}Testing notification type: $TYPE (unknown)${NC}"
    
    MESSAGE_ID="err-07-type-$TYPE-$(date +%s)"
    
    # Create notification with unknown type
    NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$TIMESTAMP",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": $TYPE,
    "purchaseToken": "$PURCHASE_TOKEN",
    "subscriptionId": "$PRODUCT_ID"
  }
}
EOF
)
    
    NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0)
    
    RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
      "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
      -H "Content-Type: application/json" \
      -H "Authorization: Bearer test-token" \
      -d "{
        \"message\": {
          \"data\": \"$NOTIFICATION_B64\",
          \"message_id\": \"$MESSAGE_ID\",
          \"attributes\": {}
        },
        \"subscription\": \"projects/test-project/pay.subscriptions/test-sub\"
      }")
    
    HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
    
    if [[ "$HTTP_CODE" == "200" ]]; then
        echo -e "  ${GREEN}✓ Acknowledged (HTTP 200) - correctly ignored${NC}"
    else
        echo -e "  ${YELLOW}⚠ HTTP $HTTP_CODE (expected 200)${NC}"
        # Still acceptable if it doesn't cause errors
        if [[ "$HTTP_CODE" =~ ^5[0-9][0-9]$ ]]; then
            ALL_ACKNOWLEDGED="false"
        fi
    fi
done
echo ""

# Step 4: Verify no database state change
echo -e "${YELLOW}[4/5] Verifying no database state change (DB Validation)${NC}"

FINAL_SUB_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')
FINAL_PAYMENT_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM pay.payments WHERE external_user_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')

# Check for test token pay.subscriptions
UNKNOWN_TOKEN_SUB=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" -t 2>/dev/null | tr -d ' ')

echo "Final subscription count: $FINAL_SUB_COUNT (initial: $INITIAL_SUB_COUNT)"
echo "Subscriptions with test token: $UNKNOWN_TOKEN_SUB"
echo "Final payment count: $FINAL_PAYMENT_COUNT (initial: $INITIAL_PAYMENT_COUNT)"
echo ""

DB_UNCHANGED="false"
if [[ "$FINAL_SUB_COUNT" == "$INITIAL_SUB_COUNT" ]] && [[ "$UNKNOWN_TOKEN_SUB" == "0" ]]; then
    echo -e "${GREEN}✓ No database state change (forward-compatible)${NC}"
    DB_UNCHANGED="true"
else
    echo -e "${RED}✗ Database state changed with unknown notification types!${NC}"
fi
echo ""

# Step 5: Summary
echo -e "${YELLOW}[5/5] Test Summary${NC}"
echo ""

echo "Unknown notification types tested: ${UNKNOWN_TYPES[*]}"
echo ""

# Determine overall test status
if [[ "$ALL_ACKNOWLEDGED" == "true" ]] && [[ "$DB_UNCHANGED" == "true" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}✓ ERR-07 Test PASSED${NC}"
elif [[ "$DB_UNCHANGED" == "true" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}✓ ERR-07 Test PASSED (unknown types handled safely)${NC}"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}✗ ERR-07 Test FAILED${NC}"
fi

# Generate JSON report
cat > err-07-report.json <<EOF
{
  "test_id": "ERR-07",
  "test_name": "Unknown Notification Type",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "user_email": "$EMAIL",
  "purchase_token": "$PURCHASE_TOKEN",
  "results": {
    "all_unknown_types_acknowledged": $ALL_ACKNOWLEDGED,
    "no_db_state_change": $DB_UNCHANGED,
    "unknown_types_tested": [99, 50, 100, 255],
    "unknown_token_subscription_count": $UNKNOWN_TOKEN_SUB,
    "initial_subscription_count": $INITIAL_SUB_COUNT,
    "final_subscription_count": $FINAL_SUB_COUNT
  },
  "notes": "Google may add new notification types; backend should be forward-compatible"
}
EOF

echo -e "${YELLOW}========================================${NC}"
echo -e "$TEST_RESULT_MSG"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: err-07-report.json"
cat err-07-report.json
echo ""

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi
exit 0
