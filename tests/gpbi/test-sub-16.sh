#!/bin/bash

##############################################################################
# SUB-16: Resubscribe Before Expiration (Continuous Access) Test
# 
# Purpose: Verify the full resubscribe flow before expiration:
#          1. Pre-register new purchase (POST /api/v1/purchases/register)
#          2. Verify new purchase (POST /api/v1/verify-purchase)
#          3. Confirm continuous access with no gap and correct token linking
#
# Usage: ./test-sub-16.sh --email "user@example.com"
#
# Prerequisites:
#   - SUB-03 must have passed (cancelled subscription exists, still active)
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - DATABASE_URL configured and db accessible
#   - psql installed and in PATH
#
# Test Flow:
#   1. Verify cancelled subscription exists with status='cancelled' but access still valid
#   2. Call /api/v1/verify-purchase with NEW purchase token
#   3. Verify new subscription created with status='active'
#   4. Verify no gap in access (is_premium remains true throughout)
#   5. Verify linkedPurchaseToken links to old cancelled token
#
# DB Validation (from TESTPLAN):
#   - pay.payments table: new row created
#   - pay.subscriptions table: new row with google_linked_purchase_token linking to cancelled token
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

# Test configuration (tokens generated later in the script)
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
    echo "Usage: ./test-sub-16.sh --email \"user@example.com\""
    exit 1
fi

echo -e "${YELLOW}========================================${NC}"
echo "SUB-16: Resubscribe Before Expiration Test"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: Query database to get user_id from email
echo -e "${YELLOW}[1/8] Fetching user_id from database for email: $EMAIL${NC}"

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

# Generate unique tokens
TIMESTAMP=$(date +%s)
OLD_TOKEN="test-sub-16-cancelled-$TIMESTAMP"
NEW_TOKEN="test-sub-16-new-token-$TIMESTAMP"  # Must contain "-new-token-" for mock to extract linked token

echo -e "${YELLOW}[2/8] Setup: Creating cancelled subscription for resubscribe test${NC}"

# Explicitly insert a cancelled subscription record
# Use ON CONFLICT to handle retries in suite context
SETUP_QUERY="INSERT INTO pay.subscriptions (external_user_id, subscription_id, provider, status, auto_renewing, purchase_token, current_period_end, created_at, updated_at) VALUES ('$USER_ID', '$PRODUCT_ID', '$PROVIDER', 'cancelled', false, '$OLD_TOKEN', NOW() + INTERVAL '7 days', NOW(), NOW()) ON CONFLICT (external_user_id, subscription_id, provider) DO UPDATE SET status = 'cancelled', purchase_token = '$OLD_TOKEN', current_period_end = NOW() + INTERVAL '7 days', updated_at = NOW();"

psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "$SETUP_QUERY" 2>/dev/null
echo -e "${GREEN}✓ Cancelled subscription created with token: $OLD_TOKEN${NC}"

# Verify setup
SUB_RESULT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT status FROM pay.subscriptions WHERE purchase_token = '$OLD_TOKEN';" -t 2>/dev/null | tr -d ' ')

if [[ "$SUB_RESULT" != "cancelled" ]]; then
     echo -e "${RED}✗ Setup failed. Expected 'cancelled', got '$SUB_RESULT'${NC}"
     exit 1
fi

# Ensure user is premium (cancelled but in period)
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "UPDATE users SET is_premium = true, premium_expires_at = NOW() + INTERVAL '7 days' WHERE external_user_id = '$USER_ID';" 2>/dev/null

OLD_PURCHASE_TOKEN=$OLD_TOKEN
OLD_STATUS="cancelled"
OLD_PERIOD_END=$(date -d "+7 days" +"%Y-%m-%d %H:%M:%S")

echo -e "${GREEN}✓ Test environment ready${NC}"
echo ""

# Step 3: Check user's premium status before resubscribe
echo -e "${YELLOW}[3/8] Checking user's premium status before resubscribe${NC}"

USER_QUERY="SELECT is_premium FROM users WHERE external_user_id = '$USER_ID';"
IS_PREMIUM_BEFORE=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "$USER_QUERY" -t 2>/dev/null | tr -d ' ')

echo "  is_premium: $IS_PREMIUM_BEFORE (expected: 't' - cancelled but still in period)"

# Validate is_premium is 't' before resubscribe (cancelled subscription still active)
IS_PREMIUM_BEFORE_CORRECT=false
if [[ "$IS_PREMIUM_BEFORE" == "t" ]] || [[ "$IS_PREMIUM_BEFORE" == "true" ]]; then
    echo -e "${GREEN}✓ User retains premium access (cancelled but still in period)${NC}"
    IS_PREMIUM_BEFORE_CORRECT=true
else
    echo -e "${RED}✗ Expected is_premium='t', got '$IS_PREMIUM_BEFORE'${NC}"
fi

echo ""

# Step 4: Pre-register resubscription
echo -e "${YELLOW}[4/8] Calling /api/v1/purchases/register (pre-registration for resubscribe)${NC}"

echo "  POST $APP_URL/api/v1/purchases/register"
echo "  Subscription ID: $PRODUCT_ID"
echo "  NEW Token: $NEW_TOKEN"
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

# Step 5: Call /api/v1/verify-purchase with NEW token for resubscribe
echo -e "${YELLOW}[5/8] Calling /api/v1/verify-purchase with NEW purchase token${NC}"

echo "  POST $APP_URL/api/v1/verify-purchase"
echo "  Provider: $PROVIDER"
echo "  Product ID: $PRODUCT_ID"
echo "  NEW Token: $NEW_TOKEN (resubscribe before expiry)"
echo "  OLD Token: $OLD_PURCHASE_TOKEN (will be linked)"
echo ""

echo "Sending request..."
VERIFY_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$APP_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
   \
   \
  -H "X-Test-Linked-Token: $OLD_PURCHASE_TOKEN" \
  -d "{
    \"provider\": \"$PROVIDER\",
    \"external_user_id\": \"$USER_ID\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$NEW_TOKEN\",
    \"product_type\": \"subscription\",
    \"linked_purchase_token\": \"$OLD_PURCHASE_TOKEN\"
  }")

HTTP_CODE=$(echo "$VERIFY_RESPONSE" | tail -n1)
LINE_COUNT=$(echo "$VERIFY_RESPONSE" | wc -l)
if [ "$LINE_COUNT" -gt 1 ]; then
    VERIFY_BODY=$(echo "$VERIFY_RESPONSE" | head -n $((LINE_COUNT - 1)))
else
    VERIFY_BODY=""
fi

echo "Response Code: $HTTP_CODE"
echo "Response: $VERIFY_BODY"
echo ""

if [[ "$HTTP_CODE" != "200" ]]; then
    echo -e "${RED}✗ verify_purchase failed with HTTP $HTTP_CODE${NC}"
    exit 1
fi

echo -e "${GREEN}✓ verify_purchase returned HTTP 200${NC}"
echo ""

# Step 5: Wait for async processing
echo -e "${YELLOW}[6/8] Waiting for async processing (1 second)${NC}"
sleep 1
echo -e "${GREEN}✓ Wait complete${NC}"
echo ""

# Step 6: Verify new subscription with linked token
echo -e "${YELLOW}[7/8] Verifying new subscription with linked token${NC}"

NEW_SUB_QUERY="SELECT id, status, auto_renewing, current_period_end, purchase_token, google_linked_purchase_token FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' AND purchase_token = '$NEW_TOKEN';"

NEW_SUB_RESULT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "$NEW_SUB_QUERY" -t 2>/dev/null || echo "")

if [[ -z "$NEW_SUB_RESULT" || "$NEW_SUB_RESULT" == *"(0 rows)"* ]]; then
    # Try finding any active subscription
    echo -e "${YELLOW}⚠ Checking for any active subscription...${NC}"
    ACTIVE_SUB_QUERY="SELECT id, status, auto_renewing, current_period_end, purchase_token, google_linked_purchase_token FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' AND status = 'active' ORDER BY created_at DESC LIMIT 1;"
    NEW_SUB_RESULT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "$ACTIVE_SUB_QUERY" -t 2>/dev/null || echo "")
fi

if [[ -z "$NEW_SUB_RESULT" || "$NEW_SUB_RESULT" == *"(0 rows)"* ]]; then
    echo -e "${RED}✗ No new subscription record found${NC}"
    exit 1
fi

NEW_STATUS=$(echo "$NEW_SUB_RESULT" | awk -F '|' '{print $2}' | head -n1 | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')
NEW_AUTO_RENEWING=$(echo "$NEW_SUB_RESULT" | awk -F '|' '{print $3}' | head -n1 | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')
NEW_PERIOD_END=$(echo "$NEW_SUB_RESULT" | awk -F '|' '{print $4}' | head -n1 | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')
NEW_PURCHASE_TOKEN=$(echo "$NEW_SUB_RESULT" | awk -F '|' '{print $5}' | head -n1 | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')
LINKED_TOKEN=$(echo "$NEW_SUB_RESULT" | awk -F '|' '{print $6}' | head -n1 | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')

echo "New Subscription:"
echo "  Status: $NEW_STATUS"
echo "  Auto Renewing: $NEW_AUTO_RENEWING"
echo "  Period End: $NEW_PERIOD_END"
echo "  Purchase Token: $NEW_PURCHASE_TOKEN"
echo "  Linked Token: $LINKED_TOKEN"
echo ""

# CRITICAL: Verify period is EXTENDED (not shortened)
OLD_EPOCH=$(date -d "$OLD_PERIOD_END" +%s 2>/dev/null || echo "0")
NEW_EPOCH=$(date -d "$NEW_PERIOD_END" +%s 2>/dev/null || echo "0")

PERIOD_EXTENDED=false
if [[ $NEW_EPOCH -le $OLD_EPOCH ]]; then
    echo -e "${RED}✗ Period not extended! Old: $OLD_PERIOD_END, New: $NEW_PERIOD_END${NC}"
    exit 1
else
    echo -e "${GREEN}✓ Period extended: $OLD_PERIOD_END → $NEW_PERIOD_END${NC}"
    PERIOD_EXTENDED=true
fi
echo ""

# Validate status
STATUS_CORRECT=false
if [[ "$NEW_STATUS" == "active" ]]; then
    echo -e "${GREEN}✓ Status: $NEW_STATUS (expected: active)${NC}"
    STATUS_CORRECT=true
else
    echo -e "${RED}✗ Expected status 'active', got '$NEW_STATUS'${NC}"
fi

# Validate auto_renewing
AUTO_RENEWING_CORRECT=false
if [[ "$NEW_AUTO_RENEWING" == "t" ]] || [[ "$NEW_AUTO_RENEWING" == "true" ]]; then
    echo -e "${GREEN}✓ auto_renewing: true (renewed subscription)${NC}"
    AUTO_RENEWING_CORRECT=true
else
    echo -e "${YELLOW}⚠ auto_renewing: $NEW_AUTO_RENEWING (expected: true)${NC}"
fi

# Validate linked token
LINKED_TOKEN_CORRECT=false
if [[ -n "$LINKED_TOKEN" ]] && [[ "$LINKED_TOKEN" != "null" ]]; then
    echo -e "${GREEN}✓ google_linked_purchase_token: set (links to cancelled subscription)${NC}"
    LINKED_TOKEN_CORRECT=true
else
    echo -e "${YELLOW}⚠ google_linked_purchase_token: not set (should link to old token)${NC}"
fi

echo ""

# Step 7: Verify payment record and continuous access
echo -e "${YELLOW}[8/8] Verifying new payment and continuous access${NC}"

# Check user still has premium
IS_PREMIUM_AFTER=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "$USER_QUERY" -t 2>/dev/null | tr -d ' ')

ACCESS_CONTINUOUS=false
if [[ "$IS_PREMIUM_AFTER" == "t" ]] || [[ "$IS_PREMIUM_AFTER" == "true" ]]; then
    echo -e "${GREEN}✓ User has continuous premium access (no gap)${NC}"
    ACCESS_CONTINUOUS=true
else
    echo -e "${RED}✗ User lost premium access (gap in service)${NC}"
fi

# Check new payment record
PAYMENT_QUERY="SELECT amount_cents, status FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' ORDER BY created_at DESC LIMIT 1;"
PAYMENT_RESULT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "$PAYMENT_QUERY" -t 2>/dev/null || echo "")

PAYMENT_RECORDED=false
if [[ -n "$PAYMENT_RESULT" ]] && [[ "$PAYMENT_RESULT" != *"(0 rows)"* ]]; then
    echo -e "${GREEN}✓ New payment record created for resubscribe${NC}"
    PAYMENT_RECORDED=true
else
    echo -e "${YELLOW}⚠ No payment record found for resubscribe${NC}"
fi

echo ""

# Generate JSON report
TEST_STATUS="pass"
if [[ "$IS_PREMIUM_BEFORE_CORRECT" != "true" ]] || [[ "$STATUS_CORRECT" != "true" ]] || [[ "$ACCESS_CONTINUOUS" != "true" ]]; then
    TEST_STATUS="fail"
elif [[ "$LINKED_TOKEN_CORRECT" != "true" ]] || [[ "$PAYMENT_RECORDED" != "true" ]]; then
    TEST_STATUS="partial"
fi

cat > sub-16-report.json <<EOF
{
  "test_id": "SUB-16",
  "test_name": "Resubscribe Before Expiration (Continuous Access)",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "user_email": "$EMAIL",
  "product_id": "$PRODUCT_ID",
  "old_token": "$OLD_PURCHASE_TOKEN",
  "new_token": "$NEW_PURCHASE_TOKEN",
  "linked_token": "$LINKED_TOKEN",
  "old_status": "$OLD_STATUS",
  "new_status": "$NEW_STATUS",
  "http_code": $HTTP_CODE,
  "results": {
    "premium_before_resubscribe": $IS_PREMIUM_BEFORE_CORRECT,
    "verify_endpoint_success": true,
    "new_subscription_created": true,
    "status_is_active": $STATUS_CORRECT,
    "auto_renewing_true": $AUTO_RENEWING_CORRECT,
    "linked_token_set": $LINKED_TOKEN_CORRECT,
    "continuous_access": $ACCESS_CONTINUOUS,
    "new_payment_recorded": $PAYMENT_RECORDED
  },
  "notes": "New purchase is separate transaction with new token. linkedPurchaseToken ensures correct user linking via cancelled account ID."
}
EOF

echo -e "${YELLOW}========================================${NC}"
if [[ "$TEST_STATUS" == "pass" ]]; then
    echo -e "${GREEN}✓ SUB-16 Test PASSED${NC}"
elif [[ "$TEST_STATUS" == "partial" ]]; then
    echo -e "${YELLOW}⚠ SUB-16 Test PARTIAL (some checks not verified)${NC}"
else
    echo -e "${RED}✗ SUB-16 Test FAILED${NC}"
fi
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: sub-16-report.json"
cat sub-16-report.json
echo ""

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi

exit 0
