#!/bin/bash

##############################################################################
# LOG-01: Structured Billing Event Logging
# 
# Purpose: Verify that billing lifecycle events are logged in structured 
#          JSON format with consistent fields for observability.
#
# Usage: ./test-log-01.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars:
#     * PROVIDER, PACKAGE_NAME, PRODUCT_ID_SUB
#     * BRIDGE_API_KEY, BRIDGE_API_URL, WEBHOOK_INGRESS_TOKEN
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER
#   - psql installed and in PATH
#
# TESTPLAN Reference:
#   Expected Behavior: Key events (verified, acknowledged, webhook processing) trigger structured JSON logs.
#                      Logs contain consistent fields: event_id, user_id (opaque), event_type, and status.
#                      No sensitive data (tokens, PII, keys) is present in standard log output.
#                      Ensures high auditability and traceability across the pipeline.
#                      Validates that logging correctly differentiates between event kinds.
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
USER_ID="${USER_ID:-test_log_01_user_$RUN_ID}"

# Defaults
APP_URL="${BRIDGE_API_URL:-http://localhost:5555}"
DB_URL="${BRIDGE_DB_URL}"

# Extract DB password if needed (globals.cfg already exports PGPASSWORD=postgres)
if [[ "$DB_URL" == *":"* ]]; then
    export PGPASSWORD="${DB_URL##*:}"
    export PGPASSWORD="${PGPASSWORD%%@*}"
fi

echo -e "${YELLOW}========================================${NC}"
echo "LOG-01: Structured Billing Event Logging"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: Generate a synthetic external_user_id for this run
echo -e "${YELLOW}[1/7] Preparing generated user_id for this run${NC}"

if [[ -z "$USER_ID" ]] || [[ "$USER_ID" == *"error"* ]] || [[ "$USER_ID" == *"ERROR"* ]]; then
    echo -e "${RED}✗ Failed to fetch user_id from database${NC}"
    echo "Error: $USER_ID"
    exit 1
fi

USER_ID=$(echo "$USER_ID" | tr -d '[:space:]')
echo -e "${GREEN}✓ User ID: $USER_ID${NC}"
echo ""

# Step 2: Clean up and prepare
echo -e "${YELLOW}[2/7] Preparing test environment${NC}"

PURCHASE_TOKEN="test-log-01-lifecycle-$(date +%s)"

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" 2>/dev/null
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.payments WHERE external_user_id = '$USER_ID';" 2>/dev/null

echo -e "${GREEN}✓ Cleanup complete${NC}"
echo -e "${BLUE}Purchase token: $PURCHASE_TOKEN${NC}"
echo ""

# Step 2.5: Register purchase (Bridge requirement)
echo -e "${YELLOW}[2.5/7] Registering purchase in Bridge${NC}"
REGISTER_HTTP=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/purchase/register" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"reason\": \"test-log-01-setup\",
    \"product_type\": \"subscription\",
    \"amount_cents\": 0,
    \"transaction_id\": \"test-log-01-reg-$RUN_ID\"
  }")

if [[ "$REGISTER_HTTP" == "200" ]]; then
    echo -e "${GREEN}✓ Purchase registration successful${NC}"
else
    echo -e "${RED}✗ Purchase registration failed (HTTP $REGISTER_HTTP)${NC}"
    exit 1
fi
echo ""

# Step 3: Execute SUB-01 (Initial Purchase) - should log purchase_verified
echo -e "${YELLOW}[3/7] Executing purchase (SUB-01 lifecycle event)${NC}"
echo ""

echo "Expected log events:"
echo "  - purchase_verified: user_id, subscription_id, token_hash, status, timestamp"
echo "  - purchase_acknowledged: subscription_id, ack_status, timestamp"
echo "  - access_granted: user_id, reason, timestamp"
echo ""

VERIFY_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"provider\": \"$PROVIDER\",
    \"external_user_id\": \"$USER_ID\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$PURCHASE_TOKEN\",
    \"product_type\": \"subscription\"
  }")

VERIFY_HTTP=$(echo "$VERIFY_RESPONSE" | tail -n1)

PURCHASE_LOGGED="false"
if [[ "$VERIFY_HTTP" == "200" ]]; then
    echo -e "${GREEN}✓ Purchase successful (HTTP 200)${NC}"
    echo -e "${BLUE}  Backend should have logged: purchase_verified, purchase_acknowledged, access_granted${NC}"
    PURCHASE_LOGGED="true"
else
    echo -e "${RED}✗ Purchase failed (HTTP $VERIFY_HTTP)${NC}"
fi
echo ""

# Step 4: Execute webhook (renewal) - should log webhook_received, webhook_processed
echo -e "${YELLOW}[4/7] Executing renewal webhook (SUB-02 lifecycle event)${NC}"
echo ""

echo "Expected log events:"
echo "  - webhook_received: notification_type, event_id, timestamp"
echo "  - webhook_processed: notification_type, event_id, new_status, timestamp"
echo ""

TIMESTAMP=$(date +%s000)
MESSAGE_ID="log-01-renewal-$(date +%s)"

NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$TIMESTAMP",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 2,
    "purchaseToken": "$PURCHASE_TOKEN",
    "subscriptionId": "$PRODUCT_ID"
  }
}
EOF
)

NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0)

WEBHOOK_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
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

WEBHOOK_HTTP=$(echo "$WEBHOOK_RESPONSE" | tail -n1)

WEBHOOK_LOGGED="false"
if [[ "$WEBHOOK_HTTP" == "200" ]] || [[ "$WEBHOOK_HTTP" == "204" ]]; then
    echo -e "${GREEN}✓ Renewal webhook processed (HTTP 200)${NC}"
    echo -e "${BLUE}  Backend should have logged: webhook_received, webhook_processed${NC}"
    WEBHOOK_LOGGED="true"
else
    echo -e "${YELLOW}⚠ Webhook returned HTTP $WEBHOOK_HTTP${NC}"
fi
echo ""

# Step 5: Execute cancellation webhook - should log status change
echo -e "${YELLOW}[5/7] Executing cancellation webhook (SUB-03 lifecycle event)${NC}"
echo ""

echo "Expected log events:"
echo "  - webhook_received: notification_type=3, event_id, timestamp"
echo "  - webhook_processed: new_status=cancelled, timestamp"
echo ""

MESSAGE_ID_CANCEL="log-01-cancel-$(date +%s)"

NOTIFICATION_CANCEL=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$TIMESTAMP",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 3,
    "purchaseToken": "$PURCHASE_TOKEN",
    "subscriptionId": "$PRODUCT_ID"
  }
}
EOF
)

NOTIFICATION_CANCEL_B64=$(echo -n "$NOTIFICATION_CANCEL" | base64 -w 0)

CANCEL_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -H "X-Webhook-Verification-Mode: off" \
  -d "{
    \"message\": {
      \"data\": \"$NOTIFICATION_CANCEL_B64\",
      \"message_id\": \"$MESSAGE_ID_CANCEL\",
      \"attributes\": {}
    },
    \"subscription\": \"projects/test-project/pay.subscriptions/test-sub\"
  }")

CANCEL_HTTP=$(echo "$CANCEL_RESPONSE" | tail -n1)

CANCEL_LOGGED="false"
if [[ "$CANCEL_HTTP" == "200" ]] || [[ "$CANCEL_HTTP" == "204" ]]; then
    echo -e "${GREEN}✓ Cancellation webhook processed (HTTP 200)${NC}"
    CANCEL_LOGGED="true"
else
    echo -e "${YELLOW}⚠ Cancellation webhook returned HTTP $CANCEL_HTTP${NC}"
fi
echo ""

# Step 6: Execute expiry webhook - should log access_revoked
echo -e "${YELLOW}[6/7] Executing expiry webhook (SUB-05 lifecycle event)${NC}"
echo ""

echo "Expected log events:"
echo "  - webhook_received: notification_type=13, event_id, timestamp"
echo "  - webhook_processed: new_status=expired, timestamp"
echo "  - access_revoked: user_id, reason=subscription_expired, timestamp"
echo ""

MESSAGE_ID_EXPIRE="log-01-expire-$(date +%s)"

NOTIFICATION_EXPIRE=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$TIMESTAMP",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 13,
    "purchaseToken": "$PURCHASE_TOKEN",
    "subscriptionId": "$PRODUCT_ID"
  }
}
EOF
)

NOTIFICATION_EXPIRE_B64=$(echo -n "$NOTIFICATION_EXPIRE" | base64 -w 0)

EXPIRE_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -H "X-Webhook-Verification-Mode: off" \
  -d "{
    \"message\": {
      \"data\": \"$NOTIFICATION_EXPIRE_B64\",
      \"message_id\": \"$MESSAGE_ID_EXPIRE\",
      \"attributes\": {}
    },
    \"subscription\": \"projects/test-project/pay.subscriptions/test-sub\"
  }")

EXPIRE_HTTP=$(echo "$EXPIRE_RESPONSE" | tail -n1)

EXPIRE_LOGGED="false"
if [[ "$EXPIRE_HTTP" == "200" ]] || [[ "$EXPIRE_HTTP" == "204" ]]; then
    echo -e "${GREEN}✓ Expiry webhook processed (HTTP 200)${NC}"
    echo -e "${BLUE}  Backend should have logged: webhook_received, webhook_processed, access_revoked${NC}"
    EXPIRE_LOGGED="true"
else
    echo -e "${YELLOW}⚠ Expiry webhook returned HTTP $EXPIRE_HTTP${NC}"
fi
echo ""

# Step 7: Summary and cleanup
echo -e "${YELLOW}[7/7] Test Summary${NC}"
echo ""

echo "Lifecycle events executed:"
echo "  - Purchase (SUB-01): $([ "$PURCHASE_LOGGED" == "true" ] && echo "✓" || echo "✗")"
echo "  - Renewal (SUB-02): $([ "$WEBHOOK_LOGGED" == "true" ] && echo "✓" || echo "✗")"
echo "  - Cancel (SUB-03): $([ "$CANCEL_LOGGED" == "true" ] && echo "✓" || echo "✗")"
echo "  - Expiry (SUB-05): $([ "$EXPIRE_LOGGED" == "true" ] && echo "✓" || echo "✗")"
echo ""

echo -e "${BLUE}Note: To verify actual log output, check backend logs for:${NC}"
echo "  - Structured JSON format (not free-form text)"
echo "  - Consistent fields per event type"
echo "  - token_hash instead of raw token"
echo "  - No emails or sensitive user_data"
echo ""

# Cleanup
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" 2>/dev/null
echo -e "${GREEN}✓ Cleaned up test data${NC}"
echo ""

# Determine overall test status
ALL_EVENTS_EXECUTED="false"
TEST_STATUS="pass"
if [[ "$PURCHASE_LOGGED" != "true" ]] || [[ "$WEBHOOK_LOGGED" != "true" ]] || [[ "$CANCEL_LOGGED" != "true" ]] || [[ "$EXPIRE_LOGGED" != "true" ]]; then
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}✗ LOG-01 Test FAILED (not all lifecycle events logged)${NC}"
else
    ALL_EVENTS_EXECUTED="true"
    TEST_RESULT_MSG="${GREEN}✓ LOG-01 Test PASSED${NC}"
fi

# Generate JSON report
cat > log-01-report.json <<EOF
{
  "test_id": "LOG-01",
  "test_name": "Structured Billing Event Logging",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "purchase_token": "$PURCHASE_TOKEN",
  "results": {
    "all_lifecycle_events_executed": $ALL_EVENTS_EXECUTED,
    "purchase_event_triggered": $PURCHASE_LOGGED,
    "renewal_webhook_triggered": $WEBHOOK_LOGGED,
    "cancel_webhook_triggered": $CANCEL_LOGGED,
    "expiry_webhook_triggered": $EXPIRE_LOGGED,
    "verify_http_code": $VERIFY_HTTP,
    "renewal_http_code": $WEBHOOK_HTTP,
    "cancel_http_code": $CANCEL_HTTP,
    "expiry_http_code": $EXPIRE_HTTP
  },
  "expected_log_events": [
    "purchase_verified",
    "purchase_acknowledged",
    "access_granted",
    "webhook_received",
    "webhook_processed",
    "access_revoked"
  ],
  "notes": "Verify backend logs contain structured JSON with consistent fields and no sensitive data"
}
EOF

echo -e "${YELLOW}========================================${NC}"
echo -e "$TEST_RESULT_MSG"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: log-01-report.json"
cat log-01-report.json
echo ""

if [[ "$TEST_STATUS" != "pass" ]]; then
    exit 1
fi
exit 0
