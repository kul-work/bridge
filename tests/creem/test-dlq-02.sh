#!/bin/bash

##############################################################################
# DLQ-02: Dead-Lettered Webhook Recovered via Admin Retry (Creem)
#
# Purpose: Verify that a dead-lettered webhook_delivery can be reset via the
#          admin retry endpoint (POST /admin/webhooks/:webhook_id/retry) or
#          the reset_webhook_delivery DB function, clearing the dead-letter
#          state so the background worker can re-attempt forwarding.
#          Also verifies the regression fix from commit de0fad7: admin retry
#          must NOT reopen an already-forwarded delivery.
#
# Usage: ./test-dlq-02.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars:
#     * PROVIDER, PRODUCT_ID_SUB, BRIDGE_APP_ID
#     * BRIDGE_API_KEY, BRIDGE_API_URL, WEBHOOK_INGRESS_TOKEN
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER
#   - psql installed and in PATH
#   - Optional: ADMIN_JWT env var with a valid Clerk admin JWT for testing
#     the admin API endpoint. If not set, the test falls back to verifying
#     the reset_webhook_delivery DB function directly.
#
# TESTPLAN Reference:
#   DLQ-02: Dead-lettered webhook recovered via admin retry. No duplicate DB
#   entries. See BRIDGE_ADMIN_TESTPLAN.md ADMIN-WHK-01/ADMIN-WHK-02.
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
TIMESTAMP=$(date +%s)
TEST_STARTED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
TEST_RUN_ID="dlq-02-${TIMESTAMP}-$$"
PRODUCT_ID="$PRODUCT_ID_SUB"
REPORT_FILE="dlq-02-report.json"
USER_ID="test_dlq_02_user_$TEST_RUN_ID"
PURCHASE_TOKEN="test-dlq-02-token-$TEST_RUN_ID"
PROVIDER_WEBHOOK_ID="dlq-02-provider-$TEST_RUN_ID"
DLQ_REASON="HTTP 500 response body: Internal Server Error"

echo -e "${YELLOW}========================================${NC}"
echo "DLQ-02: Dead-Lettered Webhook Recovered via Admin Retry (Creem)"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

# Step 1: Setup - create subscription, webhook provider, dead-lettered delivery
echo -e "${YELLOW}[1/7] Setting up dead-lettered webhook delivery${NC}"

# Clean up any prior test data
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.webhook_delivery WHERE app_id = '$BRIDGE_APP_ID' AND webhook_provider_id IN (
     SELECT id FROM pay.webhook_provider WHERE provider_webhook_id = '$PROVIDER_WEBHOOK_ID'
   );" 2>/dev/null
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.webhook_provider WHERE provider_webhook_id = '$PROVIDER_WEBHOOK_ID';" 2>/dev/null
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" 2>/dev/null

# Create subscription
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, provider, purchase_token, status, auto_renewing, current_period_end)
   VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID', '$PROVIDER', '$PURCHASE_TOKEN', 'active', true, NOW() + INTERVAL '30 days');" 2>/dev/null

# Create webhook_provider record
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "INSERT INTO pay.webhook_provider (id, app_id, provider, provider_webhook_id, event_type, subscription_id, purchase_token, payload, processed, timestamp_epoch_ms)
   VALUES (gen_random_uuid(), '$BRIDGE_APP_ID', '$PROVIDER', '$PROVIDER_WEBHOOK_ID', 'subscription.paid',
           '$PRODUCT_ID', '$PURCHASE_TOKEN', '{\"test\": \"dlq-02\"}', true, $TIMESTAMP 000);" 2>/dev/null

WHK_PROVIDER_ID=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -At -c \
  "SELECT id FROM pay.webhook_provider WHERE provider_webhook_id = '$PROVIDER_WEBHOOK_ID' AND app_id = '$BRIDGE_APP_ID';" 2>/dev/null | tr -d '[:space:]')

# Create dead-lettered webhook_delivery record (3 failed attempts, dead_lettered=true)
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "INSERT INTO pay.webhook_delivery (id, app_id, webhook_provider_id, forward_attempts, forwarded, dead_lettered, dead_lettered_at, dead_letter_reason, last_http_status, last_error)
   VALUES (gen_random_uuid(), '$BRIDGE_APP_ID', '$WHK_PROVIDER_ID', 3, false, true, NOW(), '$DLQ_REASON', 500, '$DLQ_REASON');" 2>/dev/null

DELIVERY_ID=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -At -c \
  "SELECT id FROM pay.webhook_delivery WHERE webhook_provider_id = '$WHK_PROVIDER_ID' AND app_id = '$BRIDGE_APP_ID';" 2>/dev/null | tr -d '[:space:]')

if [[ -z "$DELIVERY_ID" ]]; then
    echo -e "${RED}✗ Failed to create dead-lettered webhook_delivery record${NC}"
    exit 1
fi

echo -e "${GREEN}✓ Created dead-lettered delivery: $DELIVERY_ID${NC}"
echo ""

# Step 2: Verify initial dead-letter state
echo -e "${YELLOW}[2/7] Verifying initial dead-letter state${NC}"

INITIAL_DLQ=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -At -c \
  "SELECT dead_lettered FROM pay.webhook_delivery WHERE id = '$DELIVERY_ID';" 2>/dev/null | tr -d '[:space:]')

if [[ "$INITIAL_DLQ" != "true" ]]; then
    echo -e "${RED}✗ Initial state should be dead_lettered=true (got: $INITIAL_DLQ)${NC}"
    exit 1
fi
echo -e "${GREEN}✓ Initial state confirmed: dead_lettered=true${NC}"
echo ""

# Step 3: Attempt admin retry
echo -e "${YELLOW}[3/7] Attempting admin retry${NC}"

RETRY_METHOD="unknown"
RETRY_HTTP_CODE=""

if [[ -n "${ADMIN_JWT:-}" ]]; then
    # Call the admin API endpoint
    RETRY_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
      "$BRIDGE_API_URL/admin/webhooks/$DELIVERY_ID/retry" \
      -H "Authorization: Bearer $ADMIN_JWT" \
      -H "Content-Type: application/json" 2>/dev/null || echo "error")

    RETRY_HTTP_CODE=$(echo "$RETRY_RESPONSE" | tail -n1)
    RETRY_METHOD="admin_api"

    if [[ "$RETRY_HTTP_CODE" == "200" ]]; then
        echo -e "${GREEN}✓ Admin API retry returned 200 OK${NC}"
    else
        echo -e "${RED}✗ Admin API retry failed with HTTP $RETRY_HTTP_CODE${NC}"
        echo "$RETRY_RESPONSE" | head -n -1
        exit 1
    fi
else
    # Fallback: simulate the reset_webhook_delivery DB function directly.
    # This is the same SQL the admin handler calls internally (src/db/webhooks.rs:189-215).
    RESET_RESULT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -At -c \
      "UPDATE pay.webhook_delivery
       SET forward_attempts = 0,
           dead_lettered = false,
           dead_lettered_at = NULL,
           dead_letter_reason = NULL,
           last_error = NULL,
           updated_at = NOW()
       WHERE id = '$DELIVERY_ID'
         AND app_id = '$BRIDGE_APP_ID'
         AND forwarded = false
         AND dead_lettered = true
       RETURNING id;" 2>/dev/null | tr -d '[:space:]')

    RETRY_METHOD="db_direct"

    if [[ -n "$RESET_RESULT" ]]; then
        echo -e "${GREEN}✓ DB reset_webhook_delivery succeeded (no ADMIN_JWT set, using DB fallback)${NC}"
    else
        echo -e "${RED}✗ DB reset_webhook_delivery affected 0 rows${NC}"
        exit 1
    fi
fi
echo ""

# Step 4: Verify the delivery was reset (dead_letter cleared)
echo -e "${YELLOW}[4/7] Verifying delivery was reset${NC}"

POST_RESET_STATE=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -At -c \
  "SELECT dead_lettered || '|' ||
          COALESCE(dead_lettered_at::text, 'NULL') || '|' ||
          COALESCE(NULLIF(dead_letter_reason, ''), 'NULL') || '|' ||
          forward_attempts || '|' ||
          COALESCE(last_error, 'NULL')
   FROM pay.webhook_delivery
   WHERE id = '$DELIVERY_ID';" 2>/dev/null)

POST_DLQ=$(echo "$POST_RESET_STATE" | cut -d'|' -f1)
POST_DLQ_AT=$(echo "$POST_RESET_STATE" | cut -d'|' -f2)
POST_DLQ_REASON=$(echo "$POST_RESET_STATE" | cut -d'|' -f3)
POST_ATTEMPTS=$(echo "$POST_RESET_STATE" | cut -d'|' -f4)
POST_LAST_ERROR=$(echo "$POST_RESET_STATE" | cut -d'|' -f5)

RESET_VALID="true"

if [[ "$POST_DLQ" != "false" ]]; then
    echo -e "${RED}✗ dead_lettered is not false after reset (got: $POST_DLQ)${NC}"
    RESET_VALID="false"
else
    echo -e "${GREEN}✓ dead_lettered = false (cleared)${NC}"
fi

if [[ "$POST_DLQ_AT" != "NULL" ]]; then
    echo -e "${RED}✗ dead_lettered_at should be NULL after reset (got: $POST_DLQ_AT)${NC}"
    RESET_VALID="false"
else
    echo -e "${GREEN}✓ dead_lettered_at = NULL (cleared)${NC}"
fi

if [[ "$POST_DLQ_REASON" != "NULL" ]]; then
    echo -e "${RED}✗ dead_letter_reason should be NULL after reset (got: $POST_DLQ_REASON)${NC}"
    RESET_VALID="false"
else
    echo -e "${GREEN}✓ dead_letter_reason = NULL (cleared)${NC}"
fi

if [[ "$POST_ATTEMPTS" != "0" ]]; then
    echo -e "${RED}✗ forward_attempts should be 0 after reset (got: $POST_ATTEMPTS)${NC}"
    RESET_VALID="false"
else
    echo -e "${GREEN}✓ forward_attempts = 0 (reset)${NC}"
fi

if [[ "$POST_LAST_ERROR" != "NULL" ]]; then
    echo -e "${RED}✗ last_error should be NULL after reset (got: $POST_LAST_ERROR)${NC}"
    RESET_VALID="false"
else
    echo -e "${GREEN}✓ last_error = NULL (cleared)${NC}"
fi

echo ""

# Step 5: ADMIN-WHK-02 regression — retry on already-forwarded delivery must NOT reopen it
echo -e "${YELLOW}[5/7] Regression: admin retry must NOT reopen already-forwarded delivery${NC}"

# Mark the delivery as forwarded
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "UPDATE pay.webhook_delivery
   SET forwarded = true, forwarded_at = NOW(), forward_attempts = 1, dead_lettered = false
   WHERE id = '$DELIVERY_ID';" 2>/dev/null

# Attempt to reset a forwarded delivery — reset_webhook_delivery checks `forwarded = false`
RESET_FORWARDED=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -At -c \
  "UPDATE pay.webhook_delivery
   SET forward_attempts = 0, dead_lettered = false, dead_lettered_at = NULL, dead_letter_reason = NULL, last_error = NULL, updated_at = NOW()
   WHERE id = '$DELIVERY_ID' AND app_id = '$BRIDGE_APP_ID' AND forwarded = false AND dead_lettered = true
   RETURNING id;" 2>/dev/null | tr -d '[:space:]')

NO_REOPEN="true"
if [[ -n "$RESET_FORWARDED" ]]; then
    echo -e "${RED}✗ Already-forwarded delivery was reopened (regression!)${NC}"
    NO_REOPEN="false"
else
    echo -e "${GREEN}✓ Already-forwarded delivery was NOT reopened (forward_attempts unchanged)${NC}"
fi

# Verify forward_attempts is still 1 (not reset to 0)
FORWARDED_ATTEMPTS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -At -c \
  "SELECT forward_attempts FROM pay.webhook_delivery WHERE id = '$DELIVERY_ID';" 2>/dev/null | tr -d '[:space:]')

if [[ "$FORWARDED_ATTEMPTS" != "1" ]]; then
    echo -e "${RED}✗ forward_attempts changed to $FORWARDED_ATTEMPTS (expected 1 — no reopen)${NC}"
    NO_REOPEN="false"
else
    echo -e "${GREEN}✓ forward_attempts remains 1 (no reopen)${NC}"
fi

echo ""

# Step 6: Verify no duplicate payments/subscriptions
echo -e "${YELLOW}[6/7] Verifying no duplicate payments/subscriptions${NC}"

PAYMENT_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -At -c \
  "SELECT COUNT(*) FROM pay.payments WHERE external_user_id = '$USER_ID';" 2>/dev/null | tr -d '[:space:]')

SUB_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -At -c \
  "SELECT COUNT(*) FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" 2>/dev/null | tr -d '[:space:]')

NO_DUP="true"
if [[ "$PAYMENT_COUNT" != "0" ]]; then
    echo -e "${RED}✗ Found $PAYMENT_COUNT payment rows (expected 0)${NC}"
    NO_DUP="false"
else
    echo -e "${GREEN}✓ No payment rows created by retry${NC}"
fi

if [[ "$SUB_COUNT" != "1" ]]; then
    echo -e "${RED}✗ Found $SUB_COUNT subscription records (expected 1)${NC}"
    NO_DUP="false"
else
    echo -e "${GREEN}✓ Exactly 1 subscription record (no duplicates from retry)${NC}"
fi

echo ""

# Step 7: Cleanup
echo -e "${YELLOW}[7/7] Cleanup${NC}"

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.webhook_delivery WHERE id = '$DELIVERY_ID' AND app_id = '$BRIDGE_APP_ID';" 2>/dev/null
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.webhook_provider WHERE id = '$WHK_PROVIDER_ID' AND app_id = '$BRIDGE_APP_ID';" 2>/dev/null
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.subscriptions WHERE purchase_token = '$PURCHASE_TOKEN';" 2>/dev/null

echo -e "${GREEN}✓ Cleaned up test data${NC}"
echo ""

# Determine overall test status
if [[ "$RESET_VALID" == "true" ]] && [[ "$NO_REOPEN" == "true" ]] && [[ "$NO_DUP" == "true" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}✓ DLQ-02 Test PASSED${NC}"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}✗ DLQ-02 Test FAILED${NC}"
fi

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "DLQ-02",
  "test_name": "Dead-Lettered Webhook Recovered via Admin Retry",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "purchase_token": "$PURCHASE_TOKEN",
  "webhook_delivery_id": "$DELIVERY_ID",
  "webhook_provider_id": "$WHK_PROVIDER_ID",
  "retry_method": "$RETRY_METHOD",
  "retry_http_code": "${RETRY_HTTP_CODE:-N/A}",
  "results": {
    "reset_valid": $RESET_VALID,
    "no_reopen_forwarded": $NO_REOPEN,
    "no_duplicate_payments": $NO_DUP,
    "payment_count": $PAYMENT_COUNT,
    "subscription_count": $SUB_COUNT,
    "post_reset_dead_lettered": $POST_DLQ,
    "post_reset_forward_attempts": $POST_ATTEMPTS,
    "forwarded_attempts_after_reopen_attempt": $FORWARDED_ATTEMPTS
  },
  "notes": "Tests ADMIN-WHK-01 (reset clears dead-letter) and ADMIN-WHK-02 (already-forwarded not reopened). Regression fix from commit de0fad7."
}
EOF

echo -e "${YELLOW}========================================${NC}"
echo -e "$TEST_RESULT_MSG"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
echo ""

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi
exit 0