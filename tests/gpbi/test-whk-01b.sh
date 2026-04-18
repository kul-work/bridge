#!/bin/bash

##############################################################################
# WHK-01B: Audience Claim Mismatch Rejection
# 
# Purpose: Verify that webhooks with JWT audience claim mismatch are rejected
#          when GOOGLE_VERIFY_AUDIENCE=true is set.
#
# Usage: ./test-whk-01b.sh
#
# Prerequisites:
#   - Backend running and listening on $APP_URL (default: http://localhost:3000)
#   - Backend configured with: MOCK_EXTERNAL_APIS=true
#   - GOOGLE_PUB_SUB_AUDIENCE set (e.g., https://api.yourdomain.com)
#   - DATABASE_URL configured and db accessible
#   - psql installed and in PATH
#   - Test uses header: X-Webhook-Verification-Mode: strict
#     (Forces signature verification regardless of GOOGLE_VERIFY_WEBHOOK_SIGNATURE setting)
#
# TESTPLAN Reference:
#   Backend Behavior: Code logs "JWT audience mismatch: got 'https://different-domain.com',
#                     expected 'https://api.yourdomain.com'",
#                     Error response: WebhookVerificationFailed,
#                     Database state unchanged.
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
RUN_ID="$(date +%s)-$RANDOM"
USER_ID="${USER_ID:-test_whk_01b_user_$RUN_ID}"

# Defaults
APP_URL="$APP_URL"
DB_URL="$DATABASE_URL"

# Extract DB password once
export PGPASSWORD="${DB_URL##*:}"
export PGPASSWORD="${PGPASSWORD%%@*}"

echo -e "${YELLOW}========================================${NC}"
echo "WHK-01B: Audience Claim Mismatch Rejection"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: Generate a synthetic external_user_id for this run
echo -e "${YELLOW}[1/5] Preparing generated user_id for this run${NC}"

if [[ -z "$USER_ID" ]] || [[ "$USER_ID" == *"error"* ]] || [[ "$USER_ID" == *"ERROR"* ]]; then
    echo -e "${RED}✗ Failed to fetch user_id from database${NC}"
    echo "Error: $USER_ID"
    exit 1
fi

USER_ID=$(echo "$USER_ID" | tr -d '[:space:]')
echo -e "${GREEN}✓ User ID: $USER_ID${NC}"
echo ""

# Step 2: Record initial database state (for comparison after test)
echo -e "${YELLOW}[2/5] Recording initial database state${NC}"

INITIAL_SUB_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')
INITIAL_PAYMENT_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM pay.payments WHERE external_user_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')

echo -e "${BLUE}Initial subscription count: $INITIAL_SUB_COUNT${NC}"
echo -e "${BLUE}Initial payment count: $INITIAL_PAYMENT_COUNT${NC}"
echo ""

# Step 3: Send webhook with WRONG audience claim in JWT
echo -e "${YELLOW}[3/5] Sending webhook with WRONG audience claim${NC}"
echo ""

TIMESTAMP=$(date +%s000)
MESSAGE_ID="whk-01b-audience-mismatch-$(date +%s)"
PURCHASE_TOKEN="test-whk-01b-audience-token"

# Create a mock JWT with wrong audience (simplified - real JWT would be more complex)
# The backend should reject this because aud != GOOGLE_PUB_SUB_AUDIENCE
WRONG_AUDIENCE="https://different-domain.com"

echo "Webhook details:"
echo "  Message ID: $MESSAGE_ID"
echo "  JWT Audience (wrong): $WRONG_AUDIENCE"
echo "  Expected Audience: (configured GOOGLE_PUB_SUB_AUDIENCE)"
echo "  Product ID: $PRODUCT_ID"
echo "  Expected: HTTP 400/403 (audience claim mismatch)"
echo ""

# Create DeveloperNotification JSON
NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$TIMESTAMP",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 4,
    "purchaseToken": "$PURCHASE_TOKEN",
    "subscriptionId": "$PRODUCT_ID"
  }
}
EOF
)

# Base64 encode the notification
NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0)

# Create a fake JWT header with wrong audience
# Format: header.payload.signature (all base64url encoded)
# This simulates a JWT with aud="https://different-domain.com"
JWT_HEADER=$(echo -n '{"alg":"RS256","typ":"JWT"}' | base64 -w 0 | tr '+/' '-_' | tr -d '=')
JWT_PAYLOAD=$(echo -n "{\"aud\":\"$WRONG_AUDIENCE\",\"iss\":\"accounts.google.com\",\"exp\":$(($(date +%s) + 3600))}" | base64 -w 0 | tr '+/' '-_' | tr -d '=')
FAKE_JWT="$JWT_HEADER.$JWT_PAYLOAD.fake-signature-for-testing"

# Send webhook with JWT containing wrong audience
# Use X-Webhook-Verification-Mode: strict to force signature verification
WEBHOOK_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $FAKE_JWT" \
  -H "X-Webhook-Verification-Mode: strict" \
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

# Step 4: Verify webhook was rejected
echo -e "${YELLOW}[4/5] Verifying webhook rejection (audience mismatch)${NC}"

AUDIENCE_REJECTED="false"
if [[ "$WEBHOOK_HTTP_CODE" == "400" ]] || [[ "$WEBHOOK_HTTP_CODE" == "401" ]] || [[ "$WEBHOOK_HTTP_CODE" == "403" ]]; then
    echo -e "${GREEN}✓ Webhook correctly rejected with HTTP $WEBHOOK_HTTP_CODE${NC}"
    AUDIENCE_REJECTED="true"
elif [[ "$WEBHOOK_HTTP_CODE" == "200" ]]; then
    echo -e "${YELLOW}✗ Webhook accepted (HTTP 200) - signature/audience verification DISABLED${NC}"
    echo -e "${YELLOW}  TEST REQUIREMENT: GOOGLE_VERIFY_WEBHOOK_SIGNATURE must be 'true'${NC}"
    echo -e "${YELLOW}  Set GOOGLE_VERIFY_WEBHOOK_SIGNATURE=true in .env and restart backend${NC}"
    AUDIENCE_REJECTED="false"
else
    echo -e "${YELLOW}⚠ Unexpected HTTP code: $WEBHOOK_HTTP_CODE${NC}"
    AUDIENCE_REJECTED="false"
fi
echo ""

# Step 5: Verify database state remains UNCHANGED
echo -e "${YELLOW}[5/5] Verifying database state unchanged (DB Validation)${NC}"

FINAL_SUB_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')
FINAL_PAYMENT_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM pay.payments WHERE external_user_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')

echo "Final subscription count: $FINAL_SUB_COUNT (initial: $INITIAL_SUB_COUNT)"
echo "Final payment count: $FINAL_PAYMENT_COUNT (initial: $INITIAL_PAYMENT_COUNT)"
echo ""

DB_UNCHANGED="false"
if [[ "$FINAL_SUB_COUNT" == "$INITIAL_SUB_COUNT" ]] && [[ "$FINAL_PAYMENT_COUNT" == "$INITIAL_PAYMENT_COUNT" ]]; then
    echo -e "${GREEN}✓ Database state unchanged (no new records created)${NC}"
    DB_UNCHANGED="true"
else
    echo -e "${RED}✗ Database state changed unexpectedly!${NC}"
    echo "  Subscriptions: $INITIAL_SUB_COUNT → $FINAL_SUB_COUNT"
    echo "  Payments: $INITIAL_PAYMENT_COUNT → $FINAL_PAYMENT_COUNT"
    DB_UNCHANGED="false"
fi
echo ""

# FAIL HARD if audience mismatch not rejected
if [[ "$AUDIENCE_REJECTED" == "true" ]] && [[ "$DB_UNCHANGED" == "true" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}✓ WHK-01B Test PASSED - Security works!${NC}"
else
    TEST_STATUS="fail"
    if [[ "$AUDIENCE_REJECTED" == "false" ]]; then
        TEST_RESULT_MSG="${RED}✗ WHK-01B Test FAILED - Audience mismatch was NOT rejected (HTTP $WEBHOOK_HTTP_CODE)${NC}"
    else
        TEST_RESULT_MSG="${RED}✗ WHK-01B Test FAILED - Database was corrupted${NC}"
    fi
fi

# Generate JSON report
cat > whk-01b-report.json <<EOF
{
  "test_id": "WHK-01B",
  "test_name": "Audience Claim Mismatch Rejection",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "message_id": "$MESSAGE_ID",
  "wrong_audience": "$WRONG_AUDIENCE",
  "webhook_http_code": $WEBHOOK_HTTP_CODE,
  "results": {
    "audience_rejected": $AUDIENCE_REJECTED,
    "expected_rejection_codes": ["400", "401", "403"],
    "database_unchanged": $DB_UNCHANGED,
    "initial_subscription_count": $INITIAL_SUB_COUNT,
    "final_subscription_count": $FINAL_SUB_COUNT,
    "initial_payment_count": $INITIAL_PAYMENT_COUNT,
    "final_payment_count": $FINAL_PAYMENT_COUNT
  },
  "notes": "Set GOOGLE_VERIFY_AUDIENCE=true and GOOGLE_PUB_SUB_AUDIENCE in production"
}
EOF

echo -e "${YELLOW}========================================${NC}"
echo -e "$TEST_RESULT_MSG"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: whk-01b-report.json"
cat whk-01b-report.json
echo ""

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi
exit 0
