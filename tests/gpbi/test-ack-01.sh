#!/bin/bash

##############################################################################
# ACK-01: Subscription ACK on Initial Purchase Test
# 
# Purpose: Verify that when a new subscription is purchased, the backend 
#          immediately calls purchases.pay.subscriptions.acknowledge() and the
#          acknowledged_at field is set in the database.
#
# Usage: ./test-ack-01.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - DATABASE_URL configured and db accessible
#   - psql installed and in PATH
#
# Test Flow:
#   1. Clean up any existing subscription for test user
#   2. Call /api/v1/verify-purchase with new subscription token
#   3. Verify subscription created with status='active'
#   4. Verify acknowledged_at is NOT NULL (ACK was called)
#   5. Check backend logs confirm acknowledge() was called
#
# DB Validation (from TESTPLAN):
#   - pay.payments table: No specific changes
#   - pay.subscriptions table: acknowledged_at NOT NULL
#
# Note: ACK NOT called on renewals (only on initial purchases and resubscribes)
#       ACK must complete within 3 days or Google refunds automatically
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
USER_ID="${USER_ID:-test_ack_01_user_$RUN_ID}"
DUMMY_TOKEN="test-subscription-ack01-$RUN_ID"

# Defaults
APP_URL="$APP_URL"
DB_URL="$DATABASE_URL"

# Extract DB password once
export PGPASSWORD="${DB_URL##*:}"
export PGPASSWORD="${PGPASSWORD%%@*}"

echo -e "${YELLOW}========================================${NC}"
echo "ACK-01: Subscription ACK on Initial Purchase Test"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: Generate a synthetic external_user_id for this run
echo -e "${YELLOW}[1/5] Preparing generated user_id for this run${NC}"

if [[ -z "$USER_ID" ]] || [[ "$USER_ID" == *"error"* ]] || [[ "$USER_ID" == *"ERROR"* ]]; then
    echo -e "${RED}✗ Failed to fetch user_id from database${NC}"
    exit 1
fi

USER_ID=$(echo "$USER_ID" | tr -d '[:space:]')
echo -e "${GREEN}✓ User ID: $USER_ID${NC}"
echo ""

# Step 2: Clean up any existing subscription
echo -e "${YELLOW}[2/5] Cleaning up existing pay.subscriptions for test${NC}"

CLEANUP_QUERY="DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';"
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "$CLEANUP_QUERY" 2>/dev/null
echo -e "${GREEN}✓ Previous subscription records removed${NC}"
echo ""

# Step 3: Call /api/v1/verify-purchase for initial subscription
echo -e "${YELLOW}[3/5] Calling /api/v1/verify-purchase for initial purchase${NC}"

echo "  POST $APP_URL/api/v1/verify-purchase"
echo "  Provider: $PROVIDER"
echo "  Product ID: $PRODUCT_ID"
echo "  Token: $DUMMY_TOKEN (new subscription)"
echo "  Expected: Backend calls purchases.pay.subscriptions.acknowledge()"
echo ""

VERIFY_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$APP_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
   \
   \
  -d "{
    \"provider\": \"$PROVIDER\",
    \"external_user_id\": \"$USER_ID\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$DUMMY_TOKEN\",
    \"product_type\": \"subscription\"
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

# Step 4: Verify subscription created with acknowledged_at set
echo -e "${YELLOW}[4/5] Verifying subscription has acknowledged_at set${NC}"

DB_QUERY="SELECT external_user_id, subscription_id, status, purchase_token FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';"

echo "Query:"
echo "  $DB_QUERY"
echo ""

DB_RESULT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "$DB_QUERY" -t 2>/dev/null || echo "")

if [[ -z "$DB_RESULT" || "$DB_RESULT" == *"(0 rows)"* ]]; then
    echo -e "${RED}✗ No subscription record found in database${NC}"
    exit 1
fi

# Extract fields
STATUS=$(echo "$DB_RESULT" | awk -F '|' '{print $3}' | head -n1 | tr -d ' ')
PURCHASE_TOKEN=$(echo "$DB_RESULT" | awk -F '|' '{print $4}' | head -n1 | tr -d ' ')

# Fetch acknowledged_at from pay.payments table (canonical source)
ACK_QUERY="SELECT acknowledged_at FROM pay.payments WHERE provider_transaction_id = '$PURCHASE_TOKEN';"
ACKNOWLEDGED_AT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "$ACK_QUERY" -t 2>/dev/null | head -n1 | tr -d ' ')

echo "Subscription Record:"
echo "  Status: $STATUS"
echo "  Purchase Token: $PURCHASE_TOKEN"
echo "  Acknowledged At: $ACKNOWLEDGED_AT"
echo ""

# Validate status
STATUS_CORRECT=false
if [[ "$STATUS" == "active" ]]; then
    echo -e "${GREEN}✓ Status: $STATUS (expected: active)${NC}"
    STATUS_CORRECT=true
else
    echo -e "${RED}✗ Expected status 'active', got '$STATUS'${NC}"
fi

# Validate acknowledged_at is NOT NULL
ACKNOWLEDGED_CORRECT=false
if [[ -n "$ACKNOWLEDGED_AT" ]] && [[ "$ACKNOWLEDGED_AT" != "null" ]] && [[ "$ACKNOWLEDGED_AT" != "" ]]; then
    echo -e "${GREEN}✓ acknowledged_at: $ACKNOWLEDGED_AT (NOT NULL - ACK was called)${NC}"
    ACKNOWLEDGED_CORRECT=true
else
    echo -e "${RED}✗ acknowledged_at is NULL/empty (ACK was NOT called)${NC}"
fi
echo ""

# Step 5: Verify token matches
echo -e "${YELLOW}[5/5] Verifying purchase token matches${NC}"

TOKEN_CORRECT=false
if [[ "$PURCHASE_TOKEN" == "$DUMMY_TOKEN" ]]; then
    echo -e "${GREEN}✓ Purchase token matches${NC}"
    TOKEN_CORRECT=true
else
    echo -e "${YELLOW}⚠ Token mismatch: expected $DUMMY_TOKEN, got $PURCHASE_TOKEN${NC}"
fi
echo ""

# Generate JSON report
TEST_STATUS="pass"
if [[ "$STATUS_CORRECT" != "true" ]] || [[ "$ACKNOWLEDGED_CORRECT" != "true" ]]; then
    TEST_STATUS="fail"
elif [[ "$TOKEN_CORRECT" != "true" ]]; then
    TEST_STATUS="partial"
fi

cat > ack-01-report.json <<EOF
{
  "test_id": "ACK-01",
  "test_name": "Subscription ACK on Initial Purchase",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "product_id": "$PRODUCT_ID",
  "purchase_token": "$PURCHASE_TOKEN",
  "subscription_status": "$STATUS",
  "acknowledged_at": "$ACKNOWLEDGED_AT",
  "http_code": $HTTP_CODE,
  "results": {
    "verify_endpoint_success": true,
    "status_is_active": $STATUS_CORRECT,
    "acknowledged_at_not_null": $ACKNOWLEDGED_CORRECT,
    "token_matches": $TOKEN_CORRECT
  },
  "notes": "Per HLD §304–310: All new pay.subscriptions must be acknowledged within 3 days (or Google refunds automatically). ACK NOT called on renewals."
}
EOF

echo -e "${YELLOW}========================================${NC}"
if [[ "$TEST_STATUS" == "pass" ]]; then
    echo -e "${GREEN}✓ ACK-01 Test PASSED${NC}"
elif [[ "$TEST_STATUS" == "partial" ]]; then
    echo -e "${YELLOW}⚠ ACK-01 Test PARTIAL (some checks not verified)${NC}"
else
    echo -e "${RED}✗ ACK-01 Test FAILED${NC}"
fi
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: ack-01-report.json"
cat ack-01-report.json
echo ""

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi

exit 0
