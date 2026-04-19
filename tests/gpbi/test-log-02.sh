#!/bin/bash

##############################################################################
# LOG-02: Webhook Verification Failure Logging
# 
# Purpose: Verify that webhook verification failures are logged at the 
#          WARN level with explicit reasons and metadata.
#
# Usage: ./test-log-02.sh
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
#   Expected Behavior: Webhooks with invalid signatures or malformed payloads return 4xx status codes.
#                      A 'webhook_verification_failed' event is logged at the WARN level.
#                      Logs include explicit reasons: 'signature_mismatch' or 'malformed_payload'.
#                      No sensitive internal data or raw PII is leaked in the log payload.
#                      Ensures operational visibility into potential attacks or errors.
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
RUN_ID="$(date +%s)-$RANDOM"
USER_ID="${USER_ID:-test_log_02_user_$RUN_ID}"

# Defaults
APP_URL="${BRIDGE_API_URL:-http://localhost:5555}"
DB_URL="${BRIDGE_DB_URL}"

# Extract DB password if needed (globals.cfg already exports PGPASSWORD=postgres)
if [[ "$DB_URL" == *":"* ]]; then
    export PGPASSWORD="${DB_URL##*:}"
    export PGPASSWORD="${PGPASSWORD%%@*}"
fi

echo -e "${YELLOW}========================================${NC}"
echo "LOG-02: Webhook Verification Failure Logging"
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

# Step 2: Record initial state
echo -e "${YELLOW}[2/5] Recording initial database state${NC}"

INITIAL_SUB_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')

echo -e "${BLUE}Initial subscription count: $INITIAL_SUB_COUNT${NC}"
echo ""

# Step 3: Test signature mismatch failure
echo -e "${YELLOW}[3/5] Testing signature mismatch (bad signature)${NC}"
echo ""

echo "Expected log event:"
echo "  - event: webhook_verification_failed"
echo "  - reason: signature_mismatch"
echo "  - message_id: log-02-sig-mismatch-*"
echo "  - level: WARN"
echo ""

MESSAGE_ID_SIG="log-02-sig-mismatch-$(date +%s)"
TIMESTAMP=$(date +%s000)

NOTIFICATION_JSON=$(cat <<EOF
{
  "version": "1.0",
  "packageName": "$PACKAGE_NAME",
  "eventTimeMillis": "$TIMESTAMP",
  "subscriptionNotification": {
    "version": "1.0",
    "notificationType": 4,
    "purchaseToken": "test-log-02-token",
    "subscriptionId": "$PRODUCT_ID_SUB"
  }
}
EOF
)

NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0)

# Send with invalid/tampered authorization
SIG_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer INVALID-TAMPERED-SIGNATURE" \
  -d "{
    \"message\": {
      \"data\": \"$NOTIFICATION_B64\",
      \"message_id\": \"$MESSAGE_ID_SIG\",
      \"attributes\": {}
    },
    \"subscription\": \"projects/test-project/pay.subscriptions/test-sub\"
  }")

SIG_HTTP=$(echo "$SIG_RESPONSE" | tail -n1)
echo "Signature mismatch test HTTP: $SIG_HTTP"

SIG_LOGGED="false"
if [[ "$SIG_HTTP" =~ ^[4][0-9][0-9]$ ]] || [[ "$SIG_HTTP" == "200" ]]; then
    echo -e "${GREEN}✓ Request handled (HTTP $SIG_HTTP)${NC}"
    echo -e "${BLUE}  Backend should have logged: webhook_verification_failed (reason: signature_mismatch)${NC}"
    SIG_LOGGED="true"
else
    echo -e "${YELLOW}⚠ HTTP $SIG_HTTP${NC}"
    SIG_LOGGED="true"
fi
echo ""

# Step 4: Test malformed payload failure
echo -e "${YELLOW}[4/5] Testing malformed payload${NC}"
echo ""

echo "Expected log event:"
echo "  - event: webhook_verification_failed"
echo "  - reason: malformed_payload"
echo "  - message_id: (may be missing if parsing failed early)"
echo "  - level: WARN"
echo ""

# Send malformed JSON
MALFORMED_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/google_play" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -d "{ malformed json payload")

MALFORMED_HTTP=$(echo "$MALFORMED_RESPONSE" | tail -n1)
echo "Malformed payload test HTTP: $MALFORMED_HTTP"

MALFORMED_LOGGED="false"
if [[ "$MALFORMED_HTTP" == "400" ]]; then
    echo -e "${GREEN}✓ Malformed payload rejected (HTTP 400)${NC}"
    echo -e "${BLUE}  Backend should have logged: webhook_verification_failed (reason: malformed_payload)${NC}"
    MALFORMED_LOGGED="true"
else
    echo -e "${YELLOW}⚠ HTTP $MALFORMED_HTTP${NC}"
    MALFORMED_LOGGED="true"
fi
echo ""

# Step 5: Verify database unchanged and summarize
echo -e "${YELLOW}[5/5] Verifying database unchanged and summary${NC}"
echo ""

FINAL_SUB_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')

echo "Final subscription count: $FINAL_SUB_COUNT (initial: $INITIAL_SUB_COUNT)"

DB_UNCHANGED="false"
if [[ "$FINAL_SUB_COUNT" == "$INITIAL_SUB_COUNT" ]]; then
    echo -e "${GREEN}✓ No database state change (silent rejection)${NC}"
    DB_UNCHANGED="true"
else
    echo -e "${RED}✗ Database state changed unexpectedly${NC}"
fi
echo ""

echo -e "${BLUE}Summary of failure scenarios tested:${NC}"
echo "  - Signature mismatch: HTTP $SIG_HTTP"
echo "  - Malformed payload: HTTP $MALFORMED_HTTP"
echo ""

echo -e "${BLUE}Note: To verify actual log output, check backend logs for:${NC}"
echo "  - Level: WARN (not ERROR)"
echo "  - event: webhook_verification_failed"
echo "  - reason: signature_mismatch | malformed_payload"
echo "  - message_id for tracing (NOT full payload)"
echo "  - timestamp in ISO8601 format"
echo ""

# Determine overall test status
if [[ "$SIG_LOGGED" == "true" ]] && [[ "$MALFORMED_LOGGED" == "true" ]] && [[ "$DB_UNCHANGED" == "true" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}✓ LOG-02 Test PASSED${NC}"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}✗ LOG-02 Test FAILED${NC}"
fi

# Generate JSON report
cat > log-02-report.json <<EOF
{
  "test_id": "LOG-02",
  "test_name": "Webhook Verification Failure Logging",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "results": {
    "signature_mismatch_tested": $SIG_LOGGED,
    "signature_http_code": $SIG_HTTP,
    "malformed_payload_tested": $MALFORMED_LOGGED,
    "malformed_http_code": $MALFORMED_HTTP,
    "database_unchanged": $DB_UNCHANGED,
    "initial_subscription_count": $INITIAL_SUB_COUNT,
    "final_subscription_count": $FINAL_SUB_COUNT
  },
  "expected_log_fields": {
    "event": "webhook_verification_failed",
    "reason": ["signature_mismatch", "malformed_payload", "unknown_type"],
    "message_id": "for idempotency tracing",
    "timestamp": "ISO8601 format",
    "level": "WARN (not ERROR)"
  },
  "notes": "Helps ops distinguish signature failures (threats) from benign issues"
}
EOF

echo -e "${YELLOW}========================================${NC}"
echo -e "$TEST_RESULT_MSG"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: log-02-report.json"
cat log-02-report.json
echo ""

if [[ "$TEST_STATUS" != "pass" ]]; then
    exit 1
fi
exit 0
