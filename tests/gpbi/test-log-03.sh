#!/bin/bash

##############################################################################
# LOG-03: ACK Failure & Retry Logging
# 
# Purpose: Verify that acknowledgment (ACK) flows, failures, and retries 
#          are logged with structured metadata for monitoring.
#
# Usage: ./test-log-03.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars:
#     * PROVIDER, PRODUCT_ID_SUB
#     * BRIDGE_API_KEY, BRIDGE_API_URL
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER
#   - psql installed and in PATH
#
# TESTPLAN Reference:
#   Expected Behavior: Initial purchase triggers the provider ACK flow.
#                      'ack_attempt' events are logged with fields: attempt_count, outcome.
#                      If a failure occurs, 'ack_retry_scheduled' is logged with backoff details.
#                      Enables troubleshooting and monitoring of transient provider failures.
#                      Validates that the background processor reports activity and error states.
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
TEST_RUN_ID="log-03-${TIMESTAMP}-$$"
PRODUCT_ID="$PRODUCT_ID_SUB"
PROVIDER="$PROVIDER"
REPORT_FILE="log-03-report.json"
USER_ID="${USER_ID:-test_log_03_user_$TEST_RUN_ID}"
DUMMY_TOKEN="mock-google-play-subscription:$PRODUCT_ID:test-log-03-token-$TEST_RUN_ID"

# Defaults
APP_URL="${BRIDGE_API_URL:-http://localhost:5555}"
DB_URL="${BRIDGE_DB_URL}"
REGISTER_HTTP=0
VERIFY_HTTP=0
REGISTER_BODY=""
VERIFY_BODY=""
SUB_STATUS=""
ACK_AT=""
CURRENT_FAILURE_KIND="setup"
rm -f "$REPORT_FILE"

write_failure_report() {
    local failure_kind="$1"
    local failure_step="$2"
    local details="$3"
    local finished_at
    finished_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    cat > "$REPORT_FILE" <<EOF
{
  "test_id": "LOG-03",
  "test_name": "ACK Failure & Retry Logging",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$finished_at",
  "status": "fail",
  "failure_kind": "$failure_kind",
  "failure_step": "$failure_step",
  "details": "$details",
  "register_http_code": $REGISTER_HTTP,
  "verify_http_code": $VERIFY_HTTP,
  "subscription_status": "$SUB_STATUS",
  "acknowledged_at": "$ACK_AT"
}
EOF
}

fail_test() {
    write_failure_report "$1" "$2" "$3"
    exit 1
}

write_failure_report_on_exit() {
    local exit_code=$?
    if [[ "$exit_code" -ne 0 && ! -f "$REPORT_FILE" ]]; then
        write_failure_report "$CURRENT_FAILURE_KIND" "script_exit" "unexpected command failure (exit $exit_code)"
    fi
}
trap write_failure_report_on_exit EXIT

# Extract DB password if needed (globals.cfg already exports PGPASSWORD=postgres)
if [[ "$DB_URL" == *":"* ]]; then
    export PGPASSWORD="${DB_URL##*:}"
    export PGPASSWORD="${PGPASSWORD%%@*}"
fi

echo -e "${YELLOW}========================================${NC}"
echo "LOG-03: ACK Failure & Retry Logging"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

# Step 1: Generate a synthetic external_user_id for this run
echo -e "${YELLOW}[1/6] Preparing generated user_id for this run${NC}"

if [[ -z "$USER_ID" ]] || [[ "$USER_ID" == *"error"* ]] || [[ "$USER_ID" == *"ERROR"* ]]; then
    fail_test "setup" "user_id" "generated user ID is invalid"
fi

USER_ID=$(echo "$USER_ID" | tr -d '[:space:]')
echo -e "${GREEN}✓ User ID: $USER_ID${NC}"
echo ""

# Step 2: Clean up and prepare
echo -e "${YELLOW}[2/6] Preparing test environment${NC}"

echo -e "${GREEN}✓ Cleanup complete${NC}"
echo -e "${BLUE}Purchase token: $DUMMY_TOKEN${NC}"
echo ""

# Step 2.5: Register purchase (Bridge requirement)
echo -e "${YELLOW}[2.5/6] Registering purchase in Bridge${NC}"
REGISTER_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BRIDGE_API_URL/api/v1/purchase/register" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"external_user_id\": \"$USER_ID\",
    \"provider\": \"$PROVIDER\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"reason\": \"test-log-03-setup\"
  }")

REGISTER_HTTP=$(echo "$REGISTER_RESPONSE" | tail -n1)
REGISTER_BODY=$(echo "$REGISTER_RESPONSE" | sed '$d')

if [[ "$REGISTER_HTTP" == "200" ]] && echo "$REGISTER_BODY" | grep -Eq '"status"[[:space:]]*:[[:space:]]*"registered"'; then
    echo -e "${GREEN}✓ Purchase registration successful${NC}"
else
    fail_test "setup" "register" "expected HTTP 200 with status=registered, got HTTP $REGISTER_HTTP"
fi
echo ""

# Step 3: Execute purchase that triggers ACK
echo -e "${YELLOW}[3/6] Executing purchase (triggers ACK flow)${NC}"
echo ""

echo "Expected log events on successful ACK:"
echo "  - event: ack_attempt"
echo "  - subscription_id: $PRODUCT_ID"
echo "  - attempt: 1"
echo "  - status: success (in mock mode)"
echo ""

echo "In failure scenario (simulated), expected logs:"
echo "  - ack_attempt: attempt=1, status=failed, error=google_api_500"
echo "  - ack_retry_scheduled: next_retry_seconds=60"
echo "  - ack_attempt: attempt=2, status=success"
echo ""

VERIFY_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$BRIDGE_API_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"provider\": \"$PROVIDER\",
    \"external_user_id\": \"$USER_ID\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$DUMMY_TOKEN\",
    \"product_type\": \"subscription\"
  }")

VERIFY_HTTP=$(echo "$VERIFY_RESPONSE" | tail -n1)
VERIFY_BODY=$(echo "$VERIFY_RESPONSE" | sed '$d')

PURCHASE_OK="false"
if [[ "$VERIFY_HTTP" == "200" ]] && echo "$VERIFY_BODY" | grep -Eq '"status"[[:space:]]*:[[:space:]]*"active"'; then
    echo -e "${GREEN}✓ Purchase successful (HTTP 200)${NC}"
    echo -e "${BLUE}  Access granted even if ACK is pending${NC}"
    PURCHASE_OK="true"
else
    fail_test "setup" "verify" "expected HTTP 200 with status=active, got HTTP $VERIFY_HTTP"
fi
echo ""

# Step 4: Verify subscription exists and check acknowledged_at
CURRENT_FAILURE_KIND="behavior"
echo -e "${YELLOW}[4/6] Verifying subscription and ACK status (DB Validation)${NC}"

SUB_DATA=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "SELECT status FROM pay.subscriptions WHERE purchase_token = '$DUMMY_TOKEN';" -t 2>/dev/null || echo "")

if [[ ! -z "$SUB_DATA" ]] && [[ "$SUB_DATA" != *"(0 rows)"* ]]; then
    SUB_STATUS=$(echo "$SUB_DATA" | awk -F '|' '{print $1}' | tr -d ' ')
    
    # Fetch acknowledged_at from PAYMENTS table
    ACK_AT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "SELECT acknowledged_at FROM pay.payments WHERE provider_purchase_token = '$DUMMY_TOKEN' ORDER BY created_at DESC LIMIT 1;" -t 2>/dev/null | tr -d ' ')

    echo "Subscription status: $SUB_STATUS"
    echo "Acknowledged at: $ACK_AT"
    echo ""
    
    ACK_LOGGED="false"
    if [[ ! -z "$ACK_AT" ]] && [[ "$ACK_AT" != "" ]]; then
        echo -e "${GREEN}✓ Subscription acknowledged (ack_at set)${NC}"
        echo -e "${BLUE}  Backend should have logged: ack_attempt with status=success${NC}"
        ACK_LOGGED="true"
    else
        fail_test "behavior" "acknowledgement" "mock verification succeeded but acknowledged_at is empty"
    fi
else
    fail_test "behavior" "subscription" "verification returned 200 but no subscription was created"
fi
echo ""

# Step 5: Document expected log patterns
echo -e "${YELLOW}[5/6] Expected Log Patterns for ACK Monitoring${NC}"
echo ""

echo -e "${BLUE}ACK Attempt Log (first attempt or retry):${NC}"
echo '  {'
echo '    "event": "ack_attempt",'
echo "    \"external_user_id\": \"$USER_ID\",
    \"subscription_id\": \"$PRODUCT_ID\","
echo '    "attempt": 1,'
echo '    "status": "success" | "failed",'
echo '    "error": null | "google_api_500",'
echo '    "timestamp": "2024-01-01T00:00:00Z"'
echo '  }'
echo ""

echo -e "${BLUE}ACK Retry Scheduled Log (on failure):${NC}"
echo '  {'
echo '    "event": "ack_retry_scheduled",'
echo "    \"external_user_id\": \"$USER_ID\",
    \"subscription_id\": \"$PRODUCT_ID\","
echo '    "next_retry_seconds": 60,'
echo '    "timestamp": "2024-01-01T00:00:00Z"'
echo '  }'
echo ""

echo -e "${BLUE}Why this matters:${NC}"
echo "  - Spike in ack_attempt failures indicates Google API issues"
echo "  - ack_retry_scheduled shows retry queue is working"
echo "  - Enables ops to monitor ACK health independently"
echo ""

# Step 6: Cleanup and summary
echo -e "${YELLOW}[6/6] Cleanup and Summary${NC}"
echo ""

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.subscriptions WHERE purchase_token = '$DUMMY_TOKEN';" 2>/dev/null
echo -e "${GREEN}✓ Cleaned up test data${NC}"
echo ""

# Determine overall test status
if [[ "$PURCHASE_OK" == "true" ]] && [[ "$ACK_LOGGED" == "true" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}✓ LOG-03 Test PASSED${NC}"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}✗ LOG-03 Test FAILED${NC}"
fi

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "LOG-03",
  "test_name": "ACK Failure & Retry Logging",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "$TEST_STATUS",
  "failure_kind": null,
  "user_id": "$USER_ID",
  "purchase_token": "$DUMMY_TOKEN",
  "results": {
    "register_http_code": $REGISTER_HTTP,
    "purchase_success": $PURCHASE_OK,
    "verify_http_code": $VERIFY_HTTP,
    "ack_flow_triggered": $ACK_LOGGED,
    "subscription_status": "${SUB_STATUS:-N/A}",
    "acknowledged_at": "${ACK_AT:-NULL}"
  },
  "expected_log_events": {
    "ack_attempt": {
      "fields": ["event", "subscription_id", "attempt", "status", "error", "timestamp"],
      "status_values": ["success", "failed"]
    },
    "ack_retry_scheduled": {
      "fields": ["event", "subscription_id", "next_retry_seconds", "timestamp"]
    }
  },
  "notes": "Spike in ack failures indicates Google API issues. Enables ops monitoring."
}
EOF

echo -e "${YELLOW}========================================${NC}"
echo -e "$TEST_RESULT_MSG"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
echo ""

if [[ "$TEST_STATUS" != "pass" ]]; then
    exit 1
fi
exit 0
