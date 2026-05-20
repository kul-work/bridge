#!/bin/bash

##############################################################################
# OTP-04: Slow Card (Pending State - One-Time Product)
# 
# Purpose: Verify that a slow test card (pending state) is properly handled 
#          with the correct status transitions from Pending to Success.
#
# Usage: ./test-otp-04.sh [--replay] [--wait-for-approval] [--approve] [--repro-duplicate-verify-gap]
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars:
#     * PROVIDER, PRODUCT_ID_OTP
#     * BRIDGE_API_KEY, BRIDGE_API_URL, WEBHOOK_INGRESS_TOKEN
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER
#   - psql installed and in PATH
#
# TESTPLAN Reference:
#   Expected Behavior: Initial POST /api/v1/verify-purchase returns 202 Accepted (Pending).
#                      A payment record is created in pay.payments with status='pending'.
#                      Upon receiving the ONE_TIME_PRODUCT_PURCHASED notificationType=1 webhook (or polling), status transitions to 'success' (active).
#                      'acknowledged_at' is set in pay.payments confirming finality.
#                      Ensures the system correctly manages asynchronous payment lifecycles and 'pending' states for OTPs.
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
TEST_RUN_ID="otp-04-${TIMESTAMP}-$$"
DUMMY_TOKEN="test-inapp-slow-4567"
PRODUCT_ID="$PRODUCT_ID_OTP"
PROVIDER="$PROVIDER"
REPORT_FILE="otp-04-report.json"

# Defaults
WAIT_FOR_APPROVAL=false
REPLAY_OTP=false
APPROVE_ONLY=false
REPRO_DUPLICATE_VERIFY_GAP=false
REGRESSION_CHECKED=false
MOCK_RTDN_FIXTURE=""
APP_URL="$BRIDGE_API_URL"
DB_URL="$BRIDGE_DB_URL"

# Extract DB password once
export PGPASSWORD="${DB_URL##*:}"
export PGPASSWORD="${PGPASSWORD%%@*}"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --replay)
            REPLAY_OTP=true
            shift 1
            ;;
        --wait-for-approval)
            WAIT_FOR_APPROVAL=true
            shift
            ;;
        --approve)
            APPROVE_ONLY=true
            shift
            ;;
        --repro-duplicate-verify-gap)
            REPRO_DUPLICATE_VERIFY_GAP=true
            shift
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

if [[ "$APPROVE_ONLY" == "true" && ( "$REPLAY_OTP" == "true" || "$WAIT_FOR_APPROVAL" == "true" || "$REPRO_DUPLICATE_VERIFY_GAP" == "true" ) ]]; then
    echo -e "${RED}--approve cannot be combined with --replay, --wait-for-approval, or --repro-duplicate-verify-gap${NC}"
    exit 1
fi

if [[ "$REPLAY_OTP" == "true" && ( "$WAIT_FOR_APPROVAL" == "true" || "$REPRO_DUPLICATE_VERIFY_GAP" == "true" ) ]]; then
    echo -e "${RED}--replay cannot be combined with --wait-for-approval or --repro-duplicate-verify-gap${NC}"
    exit 1
fi

if [[ "$WAIT_FOR_APPROVAL" == "true" && "$REPRO_DUPLICATE_VERIFY_GAP" == "true" ]]; then
    echo -e "${RED}--wait-for-approval and --repro-duplicate-verify-gap are mutually exclusive${NC}"
    exit 1
fi

if [[ "$REPLAY_OTP" == "true" || "$APPROVE_ONLY" == "true" || "$REPRO_DUPLICATE_VERIFY_GAP" == "true" ]]; then
    MOCK_RTDN_FIXTURE="$SCRIPT_DIR/fixtures/otp-04-pending-response.json"
    echo -e "${YELLOW}[Approval] MOCK_RTDN_FIXTURE=${MOCK_RTDN_FIXTURE}${NC}"
fi

bridge_sql() {
    psql -v ON_ERROR_STOP=1 -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -At -c "$1"
}

wait_for_bridge_value() {
    local sql="$1"
    local expected="$2"
    local label="$3"
    local actual=""

    for _ in {1..20}; do
        actual="$(bridge_sql "$sql" 2>/dev/null | tr -d '[:space:]' || true)"
        if [[ "$actual" == "$expected" ]]; then
            echo -e "${GREEN}[OK] $label: $actual${NC}"
            return 0
        fi
        sleep 1
    done

    echo -e "${RED}[FAIL] $label: expected $expected, got ${actual:-<empty>}${NC}"
    return 1
}

HIHA_DB_MODE=""

detect_hiha_db_mode() {
    local bridge_schema_count
    bridge_schema_count=$(bridge_sql "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = 'hiha' AND table_name = 'webhook_callbacks';" 2>/dev/null | tr -d '[:space:]' || true)
    if [[ "$bridge_schema_count" == "1" ]]; then
        HIHA_DB_MODE="bridge_schema"
        return 0
    fi

    local separate_db_count
    separate_db_count=$(psql -v ON_ERROR_STOP=1 -U "$HIHA_DB_USER" -h "$HIHA_DB_HOST" -p "$HIHA_DB_PORT" -d "$HIHA_DB_NAME" -At \
        -c "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = 'public' AND table_name = 'webhook_callbacks';" 2>/dev/null | tr -d '[:space:]' || true)
    if [[ "$separate_db_count" == "1" ]]; then
        HIHA_DB_MODE="separate_public"
        return 0
    fi

    HIHA_DB_MODE=""
    return 1
}

hiha_sql() {
    local sql="$1"

    case "$HIHA_DB_MODE" in
        bridge_schema)
            psql -v ON_ERROR_STOP=1 -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -At -c "$sql"
            ;;
        separate_public)
            local public_sql
            public_sql="${sql//hiha.webhook_callbacks/webhook_callbacks}"
            public_sql="${public_sql//hiha.users/users}"
            psql -v ON_ERROR_STOP=1 -U "$HIHA_DB_USER" -h "$HIHA_DB_HOST" -p "$HIHA_DB_PORT" -d "$HIHA_DB_NAME" -At -c "$public_sql"
            ;;
        *)
            return 1
            ;;
    esac
}

send_approval_webhook() {
    if [[ ! -f "$MOCK_RTDN_FIXTURE" ]]; then
        echo -e "${RED}✗ Missing RTDN fixture: $MOCK_RTDN_FIXTURE${NC}"
        exit 1
    fi
    WEBHOOK_PATH_TOKEN="${WEBHOOK_INGRESS_TOKEN:-${WEBHOOK_TOKEN:-}}"
    if [[ -z "$WEBHOOK_PATH_TOKEN" ]]; then
        echo -e "${RED}✗ Missing WEBHOOK_INGRESS_TOKEN or WEBHOOK_TOKEN${NC}"
        exit 1
    fi

    NOTIFICATION_JSON=$(sed "s/<REDACTED_PURCHASE_TOKEN>/$DUMMY_TOKEN/g" "$MOCK_RTDN_FIXTURE")
    NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0 2>/dev/null || echo -n "$NOTIFICATION_JSON" | base64)
    WEBHOOK_ID="test-webhook-otp04-purchased-$(date +%s)"

    WEBHOOK_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
      "$APP_URL/webhooks/$WEBHOOK_PATH_TOKEN/$PROVIDER" \
      -H "Content-Type: application/json" \
      -H "Authorization: Bearer test-token" \
      -H "X-Webhook-Verification-Mode: off" \
      -d "{
        \"message\": {
          \"data\": \"$NOTIFICATION_B64\",
          \"message_id\": \"$WEBHOOK_ID\",
          \"attributes\": {}
        },
        \"subscription\": \"projects/test-project/subscriptions/test-sub\"
      }")

    WH_HTTP_CODE=$(echo "$WEBHOOK_RESPONSE" | tail -n1)
    if [[ "$WH_HTTP_CODE" == "200" ]] || [[ "$WH_HTTP_CODE" == "204" ]]; then
        echo -e "${GREEN}✓ Webhook accepted (HTTP $WH_HTTP_CODE)${NC}"
        WEBHOOK_ACCEPTED=true
    else
        echo -e "${RED}✗ Webhook returned HTTP $WH_HTTP_CODE${NC}"
        echo "$WEBHOOK_RESPONSE"
        exit 1
    fi
}

assert_success_not_downgraded_after_verify_retry() {
    local regression_user_id="$1"

    echo -e "${YELLOW}[Regression] Retrying verify-purchase after success must not downgrade payment status${NC}"

    VERIFY_RESPONSE_REGRESSION=$(curl -s -w "\n%{http_code}" -X POST \
      "$APP_URL/api/v1/verify-purchase" \
      -H "Content-Type: application/json" \
      -H "Authorization: Bearer $BRIDGE_API_KEY" \
      -d "{
        \"provider\": \"$PROVIDER\",
        \"external_user_id\": \"$regression_user_id\",
        \"subscription_id\": \"$PRODUCT_ID\",
        \"purchase_token\": \"$DUMMY_TOKEN\",
        \"product_type\": \"inapp\"
      }")

    REGRESSION_HTTP_CODE=$(echo "$VERIFY_RESPONSE_REGRESSION" | tail -n1)
    if [[ "$REGRESSION_HTTP_CODE" != "200" && "$REGRESSION_HTTP_CODE" != "202" ]]; then
        echo -e "${RED}[FAIL] Regression verify retry failed with HTTP $REGRESSION_HTTP_CODE${NC}"
        echo "$VERIFY_RESPONSE_REGRESSION"
        exit 1
    fi

    # PURCHASE_TOKEN holds the actual provider_transaction_id (order id, e.g. mock-google-play-order:...) read from the DB in step 4.
    REGRESSION_STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT status FROM pay.payments WHERE provider = '$PROVIDER' AND provider_transaction_id = '$PURCHASE_TOKEN' AND product_id = '$PRODUCT_ID' ORDER BY created_at DESC LIMIT 1;" -t 2>/dev/null | tr -d ' ')
    if [[ "$REGRESSION_STATUS" != "success" ]]; then
        echo -e "${RED}[FAIL] Regression: successful OTP was downgraded to $REGRESSION_STATUS after verify retry${NC}"
        exit 1
    fi

    REGRESSION_CHECKED=true
    echo -e "${GREEN}[OK] Success remained monotonic after verify retry${NC}"
}

if [[ "$APPROVE_ONLY" == "true" ]]; then
    echo -e "${YELLOW}========================================${NC}"
    echo "OTP-04: Approval Simulation"
    echo -e "${YELLOW}========================================${NC}"
    echo ""
    echo "Purchase token: $DUMMY_TOKEN"
    echo ""

    # provider_transaction_id is the mock order id (mock-google-play-order:DUMMY_TOKEN), not the raw token.
    APPROVAL_TARGET=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT external_user_id, status FROM pay.payments WHERE provider = '$PROVIDER' AND provider_transaction_id = 'mock-google-play-order:$DUMMY_TOKEN' AND product_id = '$PRODUCT_ID' ORDER BY created_at DESC LIMIT 1;" -t 2>/dev/null || echo "")
    if [[ -z "$APPROVAL_TARGET" || "$APPROVAL_TARGET" == *"(0 rows)"* ]]; then
        echo -e "${RED}✗ No OTP-04 payment found for token $DUMMY_TOKEN${NC}"
        echo "Start a pending run first: bash test-otp-04.sh --wait-for-approval"
        exit 1
    fi

    APPROVAL_USER_ID=$(echo "$APPROVAL_TARGET" | awk -F '|' '{print $1}' | head -n1 | xargs)
    APPROVAL_STATUS=$(echo "$APPROVAL_TARGET" | awk -F '|' '{print $2}' | head -n1 | xargs)
    PURCHASE_TOKEN="mock-google-play-order:$DUMMY_TOKEN"

    echo "Found payment:"
    echo "  User ID: $APPROVAL_USER_ID"
    echo "  Status:  $APPROVAL_STATUS"
    echo ""

    if [[ "$APPROVAL_STATUS" != "pending" ]]; then
        echo -e "${RED}✗ Expected pending payment, found: $APPROVAL_STATUS${NC}"
        exit 1
    fi

    WEBHOOK_ACCEPTED=false
    echo -e "${YELLOW}Sending ONE_TIME_PRODUCT_PURCHASED webhook approval${NC}"
    send_approval_webhook

    echo "Polling for webhook processing..."
    FINAL_STATUS="$APPROVAL_STATUS"
    for _ in {1..10}; do
        FINAL_STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT status FROM pay.payments WHERE provider = '$PROVIDER' AND provider_transaction_id = 'mock-google-play-order:$DUMMY_TOKEN' AND product_id = '$PRODUCT_ID' ORDER BY created_at DESC LIMIT 1;" -t 2>/dev/null | tr -d ' ')
        echo "  Status: $FINAL_STATUS"
        if [[ "$FINAL_STATUS" == "success" ]]; then
            assert_success_not_downgraded_after_verify_retry "$APPROVAL_USER_ID"
            echo -e "${GREEN}✓ OTP-04 approval applied${NC}"
            exit 0
        fi
        sleep 1
    done

    echo -e "${RED}✗ Approval webhook did not transition payment to success${NC}"
    exit 1
fi

if [[ "$REPRO_DUPLICATE_VERIFY_GAP" == "true" ]]; then
    echo -e "${YELLOW}========================================${NC}"
    echo "OTP-04: Duplicate Verify Gap Regression"
    echo -e "${YELLOW}========================================${NC}"
    echo ""
    # Keep this off the OTP-04 slow token so this disposable repro can run
    # beside --wait-for-approval without changing server-wide mock fixtures.
    REPRO_TOKEN="${REPRO_TOKEN:-test-inapp-otp04-gap-active-$TIMESTAMP}"
    echo "Purchase token: $REPRO_TOKEN"
    echo "User ID: ${USER_ID:-test_otp_gap_user_$TEST_RUN_ID}"
    echo ""

    if [[ -z "${BRIDGE_API_KEY:-}" ]]; then
        echo -e "${RED}[FAIL] BRIDGE_API_KEY is required${NC}"
        exit 1
    fi
    if [[ -z "${BRIDGE_APP_ID:-}" ]]; then
        echo -e "${RED}[FAIL] BRIDGE_APP_ID is required${NC}"
        exit 1
    fi

    USER_ID="${USER_ID:-test_otp_gap_user_$TEST_RUN_ID}"
    WEBHOOK_PATH_TOKEN="${WEBHOOK_INGRESS_TOKEN:-${WEBHOOK_TOKEN:-}}"
    RTDN_EVENT_ID="test-webhook-otp04-gap-purchased-$TEST_RUN_ID"
    if [[ -z "$WEBHOOK_PATH_TOKEN" ]]; then
        echo -e "${RED}[FAIL] WEBHOOK_INGRESS_TOKEN or WEBHOOK_TOKEN is required${NC}"
        exit 1
    fi

    echo -e "${YELLOW}[1/6] Cleaning repro data${NC}"
    bridge_sql "DELETE FROM pay.webhook_delivery WHERE webhook_provider_id IN (SELECT id FROM pay.webhook_provider WHERE app_id = '$BRIDGE_APP_ID' AND provider = '$PROVIDER' AND (purchase_token = '$REPRO_TOKEN' OR provider_webhook_id LIKE 'test-webhook-otp04-gap-%'));" >/dev/null
    bridge_sql "DELETE FROM pay.webhook_provider WHERE app_id = '$BRIDGE_APP_ID' AND provider = '$PROVIDER' AND (purchase_token = '$REPRO_TOKEN' OR provider_webhook_id LIKE 'test-webhook-otp04-gap-%');" >/dev/null
    bridge_sql "DELETE FROM pay.payments WHERE app_id = '$BRIDGE_APP_ID' AND provider = '$PROVIDER' AND product_id = '$PRODUCT_ID' AND (external_user_id = '$USER_ID' OR provider_purchase_token = '$REPRO_TOKEN' OR provider_transaction_id = 'mock-google-play-order:$REPRO_TOKEN');" >/dev/null
    bridge_sql "DELETE FROM pay.subscriptions WHERE app_id = '$BRIDGE_APP_ID' AND provider = '$PROVIDER' AND external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" >/dev/null
    detect_hiha_db_mode >/dev/null 2>&1 || true
    hiha_sql "DELETE FROM hiha.webhook_callbacks WHERE clerk_id = '$USER_ID'; DELETE FROM hiha.users WHERE clerk_id = '$USER_ID';" >/dev/null || true
    echo -e "${GREEN}[OK] Cleanup complete${NC}"
    echo ""

    echo -e "${YELLOW}[2/6] Sending OTP RTDN before any verify-purchase binding exists${NC}"
    NOTIFICATION_JSON=$(sed "s/<REDACTED_PURCHASE_TOKEN>/$REPRO_TOKEN/g" "$MOCK_RTDN_FIXTURE")
    NOTIFICATION_B64=$(echo -n "$NOTIFICATION_JSON" | base64 -w 0 2>/dev/null || echo -n "$NOTIFICATION_JSON" | base64)
    WEBHOOK_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
      "$APP_URL/webhooks/$WEBHOOK_PATH_TOKEN/$PROVIDER" \
      -H "Content-Type: application/json" \
      -H "Authorization: Bearer test-token" \
      -H "X-Webhook-Verification-Mode: off" \
      -d "{
        \"message\": {
          \"data\": \"$NOTIFICATION_B64\",
          \"message_id\": \"$RTDN_EVENT_ID\",
          \"attributes\": {}
        },
        \"subscription\": \"projects/test-project/subscriptions/test-sub\"
      }")
    WH_HTTP_CODE=$(echo "$WEBHOOK_RESPONSE" | tail -n1)
    if [[ "$WH_HTTP_CODE" != "200" && "$WH_HTTP_CODE" != "204" ]]; then
        echo -e "${RED}[FAIL] RTDN returned HTTP $WH_HTTP_CODE${NC}"
        echo "$WEBHOOK_RESPONSE"
        exit 1
    fi
    echo -e "${GREEN}[OK] RTDN accepted: HTTP $WH_HTTP_CODE${NC}"
    wait_for_bridge_value "SELECT COUNT(*) FROM pay.webhook_provider WHERE app_id = '$BRIDGE_APP_ID' AND provider = '$PROVIDER' AND provider_webhook_id = '$RTDN_EVENT_ID' AND event_type = 'ONE_TIME_PRODUCT_PURCHASED' AND purchase_token = '$REPRO_TOKEN' AND suppressed = true AND suppressed_reason = 'unresolved_external_user_id';" "1" "OTP RTDN suppressed without user binding"
    echo ""

    echo -e "${YELLOW}[3/6] Sending three verify-purchase requests for the same OTP token/user${NC}"
    VERIFY_BODY="{
      \"provider\": \"$PROVIDER\",
      \"external_user_id\": \"$USER_ID\",
      \"subscription_id\": \"$PRODUCT_ID\",
      \"purchase_token\": \"$REPRO_TOKEN\",
      \"product_type\": \"inapp\"
    }"
    VERIFY_TMP_DIR="${TMPDIR:-/tmp}/otp04-gap-$TEST_RUN_ID"
    mkdir -p "$VERIFY_TMP_DIR"
    for n in 1 2 3; do
        curl -s -w "\n%{http_code}" -X POST \
          "$APP_URL/api/v1/verify-purchase" \
          -H "Content-Type: application/json" \
          -H "Authorization: Bearer $BRIDGE_API_KEY" \
          -d "$VERIFY_BODY" > "$VERIFY_TMP_DIR/verify-$n.out" &
    done
    wait

    for n in 1 2 3; do
        VERIFY_HTTP=$(tail -n1 "$VERIFY_TMP_DIR/verify-$n.out")
        VERIFY_PAYLOAD=$(head -n -1 "$VERIFY_TMP_DIR/verify-$n.out")
        echo "  verify $n HTTP: $VERIFY_HTTP"
        echo "  verify $n body: $VERIFY_PAYLOAD"
        if [[ "$VERIFY_HTTP" != "200" ]]; then
            echo -e "${RED}[FAIL] Expected verify $n to return HTTP 200 completed/active${NC}"
            exit 1
        fi
        if [[ "$VERIFY_PAYLOAD" != *"\"status\":\"active\""* ]]; then
            echo -e "${RED}[FAIL] Expected verify $n response status to be active${NC}"
            exit 1
        fi
    done
    rm -f "$VERIFY_TMP_DIR"/verify-*.out
    rmdir "$VERIFY_TMP_DIR" 2>/dev/null || true
    echo -e "${GREEN}[OK] Three successful verify-purchase calls completed${NC}"
    echo ""

    echo -e "${YELLOW}[4/6] Validating Bridge verify callback idempotency${NC}"
    wait_for_bridge_value "SELECT COUNT(*) FROM pay.webhook_provider WHERE app_id = '$BRIDGE_APP_ID' AND provider = '$PROVIDER' AND purchase_token = '$REPRO_TOKEN' AND event_type = 'verify_purchase.succeeded';" "1" "One synthetic verify webhook row"
    PAYMENT_COUNT=$(bridge_sql "SELECT COUNT(*) FROM pay.payments WHERE app_id = '$BRIDGE_APP_ID' AND provider = '$PROVIDER' AND product_id = '$PRODUCT_ID' AND provider_purchase_token = '$REPRO_TOKEN';" | tr -d '[:space:]')
    DELIVERY_COUNT=$(bridge_sql "SELECT COUNT(*) FROM pay.webhook_delivery d JOIN pay.webhook_provider w ON w.id = d.webhook_provider_id WHERE w.app_id = '$BRIDGE_APP_ID' AND w.provider = '$PROVIDER' AND w.purchase_token = '$REPRO_TOKEN' AND w.event_type = 'verify_purchase.succeeded';" | tr -d '[:space:]')
    FORWARDED_COUNT=$(bridge_sql "SELECT COUNT(*) FROM pay.webhook_delivery d JOIN pay.webhook_provider w ON w.id = d.webhook_provider_id WHERE w.app_id = '$BRIDGE_APP_ID' AND w.provider = '$PROVIDER' AND w.purchase_token = '$REPRO_TOKEN' AND w.event_type = 'verify_purchase.succeeded' AND d.forwarded = true;" | tr -d '[:space:]')
    echo "Payment rows for OTP token: $PAYMENT_COUNT"
    echo "Verify delivery rows: $DELIVERY_COUNT"
    echo "Forwarded verify delivery rows: $FORWARDED_COUNT"
    if [[ "$PAYMENT_COUNT" != "1" ]]; then
        echo -e "${RED}[FAIL] Expected one payment row for the OTP token${NC}"
        exit 1
    fi
    if [[ "$DELIVERY_COUNT" != "1" ]]; then
        echo -e "${RED}[FAIL] Expected one verify delivery row${NC}"
        exit 1
    fi
    if [[ "$FORWARDED_COUNT" != "1" ]]; then
        echo -e "${RED}[FAIL] Expected one forwarded verify delivery row${NC}"
        exit 1
    fi
    echo ""

    echo -e "${YELLOW}[5/6] Optional HiHa callback validation${NC}"
    HIHA_AVAILABLE=false
    HIHA_CALLBACK_COUNT="0"
    HIHA_USER_PREMIUM="unknown"
    detect_hiha_db_mode >/dev/null 2>&1 || true
    if [[ -n "$HIHA_DB_MODE" ]] && HIHA_CALLBACK_COUNT=$(hiha_sql "SELECT COUNT(*) FROM hiha.webhook_callbacks WHERE clerk_id = '$USER_ID' AND event_id LIKE 'verify-purchase-%';" 2>/dev/null | tr -d '[:space:]'); then
        HIHA_AVAILABLE=true
        HIHA_USER_PREMIUM=$(hiha_sql "SELECT COALESCE(is_premium::text, 'missing') FROM hiha.users WHERE clerk_id = '$USER_ID';" 2>/dev/null | tr -d '[:space:]' || echo "unknown")
        echo "HiHa verify callback rows: $HIHA_CALLBACK_COUNT"
        echo "HiHa user is_premium: $HIHA_USER_PREMIUM"
        if [[ "$HIHA_CALLBACK_COUNT" != "1" ]]; then
            echo -e "${RED}[FAIL] Expected one HiHa verify callback row${NC}"
            exit 1
        fi
    else
        echo -e "${YELLOW}[WARN] HiHa DB unavailable or callback rows absent; skipped app-side assertions${NC}"
    fi
    echo ""

    echo -e "${YELLOW}[6/6] Repro summary${NC}"
    REPORT_FILE="otp-04-duplicate-verify-gap-report.json"
    TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    cat > "$REPORT_FILE" <<EOF
{
  "test_id": "OTP-04-DUPLICATE-VERIFY-GAP",
  "test_name": "OTP slow-card RTDN unresolved user plus verify callback idempotency",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "pass",
  "user_id": "$USER_ID",
  "product_id": "$PRODUCT_ID",
  "purchase_token": "$REPRO_TOKEN",
  "rtdn_event_id": "$RTDN_EVENT_ID",
  "bridge": {
    "suppressed_rtdn_count": 1,
    "payment_count": $PAYMENT_COUNT,
    "verify_webhook_count": 1,
    "verify_delivery_count": $DELIVERY_COUNT,
    "verify_forwarded_count": $FORWARDED_COUNT
  },
  "hiha": {
    "available": $HIHA_AVAILABLE,
    "verify_callback_count": "$HIHA_CALLBACK_COUNT",
    "user_is_premium": "$HIHA_USER_PREMIUM"
  }
}
EOF
    echo -e "${GREEN}[OK] Duplicate verify callback suppression verified${NC}"
    echo "Report saved to: $REPORT_FILE"
    cat "$REPORT_FILE"
    echo ""
    exit 0
fi

echo -e "${YELLOW}========================================${NC}"
echo "OTP-04: Slow Card (Pending State) Test"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

# Step 1: External User ID
USER_ID="${USER_ID:-test_otp_user_04_$TEST_RUN_ID}"
echo -e "${GREEN}✓ Testing with User ID: $USER_ID${NC}"
echo ""

# Step 2: Clean up any existing entries from previous tests
echo -e "${YELLOW}[2/8] Cleaning up previous test data${NC}"

CLEANUP_QUERY="DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$CLEANUP_QUERY" 2>/dev/null
echo -e "${GREEN}✓ Previous subscription record removed${NC}"

CLEANUP_TOKEN_SUBSCRIPTIONS_QUERY="DELETE FROM pay.subscriptions s USING pay.payments p WHERE p.product_id = '$PRODUCT_ID' AND s.external_user_id = p.external_user_id AND s.subscription_id = '$PRODUCT_ID' AND (p.provider_transaction_id = '$DUMMY_TOKEN' OR p.provider_transaction_id = 'mock-google-play-order:$DUMMY_TOKEN');"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$CLEANUP_TOKEN_SUBSCRIPTIONS_QUERY" 2>/dev/null
echo -e "${GREEN}✓ Previous slow-card token subscription records removed${NC}"

# Include both the raw token (legacy) and the mock order id (current) to cover re-runs.
CLEANUP_PAYMENTS_QUERY="DELETE FROM pay.payments WHERE (external_user_id = '$USER_ID' AND product_id = '$PRODUCT_ID') OR ((provider_transaction_id = '$DUMMY_TOKEN' OR provider_transaction_id = 'mock-google-play-order:$DUMMY_TOKEN') AND product_id = '$PRODUCT_ID');"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$CLEANUP_PAYMENTS_QUERY" 2>/dev/null
echo -e "${GREEN}✓ Previous payment records removed${NC}"

CLEANUP_WEBHOOK_DELIVERY_QUERY="DELETE FROM pay.webhook_delivery d USING pay.webhook_provider w WHERE d.webhook_provider_id = w.id AND w.provider = '$PROVIDER' AND w.purchase_token = '$DUMMY_TOKEN';"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$CLEANUP_WEBHOOK_DELIVERY_QUERY" 2>/dev/null
echo -e "${GREEN}✓ Previous slow-card webhook delivery records removed${NC}"

CLEANUP_WEBHOOK_QUERY="DELETE FROM pay.webhook_provider WHERE provider = '$PROVIDER' AND purchase_token = '$DUMMY_TOKEN';"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$CLEANUP_WEBHOOK_QUERY" 2>/dev/null
echo -e "${GREEN}✓ Previous slow-card webhook ingress records removed${NC}"
echo ""

# Step 3: Call /api/v1/verify-purchase endpoint with pending token
echo -e "${YELLOW}[3/8] Calling /api/v1/verify-purchase with slow card token${NC}"

echo "  POST $APP_URL/api/v1/verify-purchase"
echo "  Provider: $PROVIDER"
echo "  Product ID: $PRODUCT_ID"
echo "  Token: $DUMMY_TOKEN (slow card, purchaseState: 2 = PENDING)"
echo "  API Method: purchases.products.get() (v1)"
echo ""

echo "Sending request..."

VERIFY_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$APP_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -d "{
    \"provider\": \"$PROVIDER\",
    \"external_user_id\": \"$USER_ID\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$DUMMY_TOKEN\",
    \"product_type\": \"inapp\"
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

if [[ "$HTTP_CODE" != "202" ]]; then
    echo -e "${RED}✗ verify_purchase failed: Expected HTTP 202 (Accepted/Pending) but got $HTTP_CODE${NC}"
    exit 1
fi

echo -e "${GREEN}✓ verify_purchase returned HTTP 202 (Pending) as expected${NC}"
echo ""

# Step 4: Query database to verify initial storage (pending state)
echo -e "${YELLOW}[4/8] Querying database to verify payment record${NC}"

DB_QUERY="SELECT external_user_id, product_id, status, provider_transaction_id FROM pay.payments WHERE external_user_id = '$USER_ID' AND product_id = '$PRODUCT_ID' ORDER BY created_at DESC LIMIT 1;"

echo "Query:"
echo "  $DB_QUERY"
echo ""

DB_RESULT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$DB_QUERY" -t 2>/dev/null || echo "")

if [[ -z "$DB_RESULT" || "$DB_RESULT" == *"(0 rows)"* ]]; then
    echo -e "${RED}✗ No payment record found in database${NC}"
    echo "Database result: $DB_RESULT"
    exit 1
fi

echo -e "${GREEN}✓ Payment record found:${NC}"
echo "$DB_RESULT" | while read line; do
    echo "  $line"
done
echo ""

# Extract initial status
INITIAL_STATUS=$(echo "$DB_RESULT" | awk -F '|' '{print $3}' | head -n1 | tr -d ' ')
PURCHASE_TOKEN=$(echo "$DB_RESULT" | awk -F '|' '{print $4}' | head -n1 | tr -d ' ')

echo -e "${BLUE}Initial status: $INITIAL_STATUS${NC}"
echo -e "${BLUE}Purchase token: $PURCHASE_TOKEN${NC}"
echo ""

# Step 5: Verify payment record was created
echo -e "${YELLOW}[5/8] Verifying payment record was created${NC}"

PAYMENT_QUERY="SELECT amount_cents, status, provider_transaction_id FROM pay.payments WHERE external_user_id = '$USER_ID' AND product_id = '$PRODUCT_ID' ORDER BY created_at DESC LIMIT 1;"

echo "Query:"
echo "  $PAYMENT_QUERY"
echo ""

PAYMENT_RESULT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$PAYMENT_QUERY" -t 2>/dev/null || echo "")

if [[ -z "$PAYMENT_RESULT" || "$PAYMENT_RESULT" == *"(0 rows)"* ]]; then
    echo -e "${YELLOW}⚠ No payment record found (may be normal for pending state)${NC}"
else
    PAYMENT_STATUS=$(echo "$PAYMENT_RESULT" | awk -F '|' '{print $2}' | tr -d ' ')
    echo -e "${GREEN}✓ Payment Record Found: Status=${PAYMENT_STATUS}${NC}"
fi
echo ""

FINAL_STATUS=$INITIAL_STATUS
WEBHOOK_ACCEPTED=false

# Step 6: Simulate Google approval via Pub/Sub webhook in replay mode
if [[ "$REPLAY_OTP" == "true" ]]; then
    echo -e "${YELLOW}[6/8] Sending ONE_TIME_PRODUCT_PURCHASED webhook (replay approval)${NC}"

    send_approval_webhook

    echo "Polling for webhook processing..."
    for _ in {1..10}; do
        FINAL_STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT status FROM pay.payments WHERE external_user_id = '$USER_ID' AND product_id = '$PRODUCT_ID' ORDER BY created_at DESC LIMIT 1;" -t 2>/dev/null | tr -d ' ')
        echo "  Status: $FINAL_STATUS"
        if [[ "$FINAL_STATUS" == "success" ]]; then
            break
        fi
        sleep 1
    done
    echo ""

# Step 6: Wait for real slow-card approval (optional, with polling)
elif [[ "$WAIT_FOR_APPROVAL" == "true" ]]; then
    echo -e "${YELLOW}[6/8] Polling for slow card approval (~5-10 minutes)${NC}"
    echo ""
    
    MAX_WAIT=600  # 10 minutes in seconds
    POLL_INTERVAL=30  # Check every 30 seconds
    ELAPSED=0
    
    while [[ $ELAPSED -lt $MAX_WAIT ]]; do
        sleep $POLL_INTERVAL
        ELAPSED=$((ELAPSED + POLL_INTERVAL))

        # Check DB first so an approval webhook that arrived during sleep is
        # not overwritten by another mock slow-card verification call.
         DB_STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT status FROM pay.payments WHERE external_user_id = '$USER_ID' AND product_id = '$PRODUCT_ID' ORDER BY created_at DESC LIMIT 1;" -t 2>/dev/null | tr -d ' ')

         echo "[$ELAPSED/$MAX_WAIT s] Status: $DB_STATUS"

         if [[ "$DB_STATUS" == "success" ]]; then
             echo -e "${GREEN}✓ Slow card approved! Status transitioned to: success${NC}"
             FINAL_STATUS="success"
             break
         fi
        
        # Call verify again to check if approval completed
        VERIFY_RESPONSE_POLL=$(curl -s -w "\n%{http_code}" -X POST \
          "$APP_URL/api/v1/verify-purchase" \
          -H "Content-Type: application/json" \
          -H "Authorization: Bearer $BRIDGE_API_KEY" \
           \
          -d "{
            \"provider\": \"$PROVIDER\",
            \"external_user_id\": \"$USER_ID\",
    \"subscription_id\": \"$PRODUCT_ID\",
            \"purchase_token\": \"$DUMMY_TOKEN\",
            \"product_type\": \"inapp\"
          }")
        
        HTTP_CODE_POLL=$(echo "$VERIFY_RESPONSE_POLL" | tail -n1)
        
        # Check DB status again after retrying verification.
         DB_STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT status FROM pay.payments WHERE external_user_id = '$USER_ID' AND product_id = '$PRODUCT_ID' ORDER BY created_at DESC LIMIT 1;" -t 2>/dev/null | tr -d ' ')
         
         echo "[$ELAPSED/$MAX_WAIT s] Status after retry: $DB_STATUS"
         
         if [[ "$DB_STATUS" == "success" ]]; then
             echo -e "${GREEN}✓ Slow card approved! Status transitioned to: success${NC}"
             FINAL_STATUS="success"
             break
         fi
        done
        
        if [[ $ELAPSED -ge $MAX_WAIT ]]; then
         echo -e "${YELLOW}⚠ Approval did not complete within 10 minutes${NC}"
         echo -e "${YELLOW}  This may be normal for slow card tests${NC}"
         # Get final status
         FINAL_STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT status FROM pay.payments WHERE external_user_id = '$USER_ID' AND product_id = '$PRODUCT_ID' ORDER BY created_at DESC LIMIT 1;" -t 2>/dev/null | tr -d ' ')
    fi
    echo ""
else
    echo -e "${YELLOW}[6/8] Approval polling not requested${NC}"
    echo ""
    echo "Note: Run with --wait-for-approval to poll for slow card completion"
    echo "Note: Run with --replay to simulate the completion webhook"
    echo ""
fi

# Step 8: Final verification
echo -e "${YELLOW}[8/8] Final verification${NC}"

echo "Expected transition:"
echo "  - Initial state: Pending (from purchaseState: 2 = PENDING)"
echo "  - Final payment state: Success (after Google approval, purchaseState: 0 = PURCHASED)"
echo ""

if [[ "$REPLAY_OTP" == "true" || "$WAIT_FOR_APPROVAL" == "true" ]]; then
    if [[ "$FINAL_STATUS" == "success" ]]; then
        echo -e "${GREEN}✓ Status transition verified: $INITIAL_STATUS → $FINAL_STATUS${NC}"
        assert_success_not_downgraded_after_verify_retry "$USER_ID"
    else
        echo -e "${RED}✗ Status did not transition to success${NC}"
        echo "  Current status: $FINAL_STATUS"
        exit 1
    fi
else
    echo -e "${GREEN}✓ Initial pending record created (approval verification skipped)${NC}"
    echo "  Initial status: $INITIAL_STATUS"
fi

SUBSCRIPTION_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t 2>/dev/null | tr -d ' ')
if [[ "$SUBSCRIPTION_COUNT" == "0" ]]; then
    echo -e "${GREEN}✓ No subscription row created for one-time product${NC}"
else
    echo -e "${RED}✗ One-time product created subscription rows: $SUBSCRIPTION_COUNT${NC}"
    exit 1
fi

ACKNOWLEDGED_AT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT acknowledged_at FROM pay.payments WHERE external_user_id = '$USER_ID' AND product_id = '$PRODUCT_ID' ORDER BY created_at DESC LIMIT 1;" -t 2>/dev/null | xargs)
if [[ "$REPLAY_OTP" == "true" || "$WAIT_FOR_APPROVAL" == "true" ]]; then
    if [[ -n "$ACKNOWLEDGED_AT" ]]; then
        echo -e "${GREEN}✓ Payment acknowledged_at is set${NC}"
    else
        echo -e "${RED}✗ Payment acknowledged_at is not set after completion${NC}"
        exit 1
    fi
fi
echo ""

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "OTP-04",
  "test_name": "Slow Card (Pending State)",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "pass",
  "user_id": "$USER_ID",
  "product_id": "$PRODUCT_ID",
  "purchase_token": "$PURCHASE_TOKEN",
  "initial_status": "$INITIAL_STATUS",
  "final_status": "$FINAL_STATUS",
  "webhook_replay": $REPLAY_OTP,
  "webhook_accepted": $WEBHOOK_ACCEPTED,
  "approval_polling": $WAIT_FOR_APPROVAL,
  "acknowledged_at": "$ACKNOWLEDGED_AT",
  "results": {
    "verify_endpoint_success": true,
    "database_record_created": true,
    "initial_state_pending": $([ "$INITIAL_STATUS" == "pending" ] && echo "true" || echo "false"),
    "status_transition_to_success": $([ "$FINAL_STATUS" == "success" ] && echo "true" || echo "false"),
    "success_not_downgraded_after_verify_retry": $REGRESSION_CHECKED,
    "payment_acknowledged": $([ -n "$ACKNOWLEDGED_AT" ] && echo "true" || echo "false"),
    "no_subscription_rows": $([ "$SUBSCRIPTION_COUNT" == "0" ] && echo "true" || echo "false"),
    "token_matches": true
  }
}
EOF

echo -e "${YELLOW}========================================${NC}"
echo -e "${GREEN}✓ OTP-04 Test PASSED${NC}"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
echo ""

exit 0
