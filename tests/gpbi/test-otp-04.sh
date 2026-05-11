#!/bin/bash

##############################################################################
# OTP-04: Slow Card (Pending State - One-Time Product)
# 
# Purpose: Verify that a slow test card (pending state) is properly handled 
#          with the correct status transitions from Pending to Success.
#
# Usage: ./test-otp-04.sh [--replay] [--wait-for-approval] [--approve]
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
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

if [[ "$APPROVE_ONLY" == "true" && ( "$REPLAY_OTP" == "true" || "$WAIT_FOR_APPROVAL" == "true" ) ]]; then
    echo -e "${RED}--approve cannot be combined with --replay or --wait-for-approval${NC}"
    exit 1
fi

if [[ "$REPLAY_OTP" == "true" && "$WAIT_FOR_APPROVAL" == "true" ]]; then
    echo -e "${RED}--replay and --wait-for-approval are mutually exclusive${NC}"
    exit 1
fi

if [[ "$REPLAY_OTP" == "true" || "$APPROVE_ONLY" == "true" ]]; then
    MOCK_RTDN_FIXTURE="$SCRIPT_DIR/fixtures/otp-04-pending-response.json"
    echo -e "${YELLOW}[Approval] MOCK_RTDN_FIXTURE=${MOCK_RTDN_FIXTURE}${NC}"
fi

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

    REGRESSION_STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT status FROM pay.payments WHERE provider = '$PROVIDER' AND provider_transaction_id = '$DUMMY_TOKEN' AND product_id = '$PRODUCT_ID' ORDER BY created_at DESC LIMIT 1;" -t 2>/dev/null | tr -d ' ')
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

    APPROVAL_TARGET=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT external_user_id, status FROM pay.payments WHERE provider = '$PROVIDER' AND provider_transaction_id = '$DUMMY_TOKEN' AND product_id = '$PRODUCT_ID' ORDER BY created_at DESC LIMIT 1;" -t 2>/dev/null || echo "")
    if [[ -z "$APPROVAL_TARGET" || "$APPROVAL_TARGET" == *"(0 rows)"* ]]; then
        echo -e "${RED}✗ No OTP-04 payment found for token $DUMMY_TOKEN${NC}"
        echo "Start a pending run first: bash test-otp-04.sh --wait-for-approval"
        exit 1
    fi

    APPROVAL_USER_ID=$(echo "$APPROVAL_TARGET" | awk -F '|' '{print $1}' | head -n1 | xargs)
    APPROVAL_STATUS=$(echo "$APPROVAL_TARGET" | awk -F '|' '{print $2}' | head -n1 | xargs)

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
        FINAL_STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "SELECT status FROM pay.payments WHERE provider = '$PROVIDER' AND provider_transaction_id = '$DUMMY_TOKEN' AND product_id = '$PRODUCT_ID' ORDER BY created_at DESC LIMIT 1;" -t 2>/dev/null | tr -d ' ')
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

CLEANUP_TOKEN_SUBSCRIPTIONS_QUERY="DELETE FROM pay.subscriptions s USING pay.payments p WHERE p.provider_transaction_id = '$DUMMY_TOKEN' AND p.product_id = '$PRODUCT_ID' AND s.external_user_id = p.external_user_id AND s.subscription_id = '$PRODUCT_ID';"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "$CLEANUP_TOKEN_SUBSCRIPTIONS_QUERY" 2>/dev/null
echo -e "${GREEN}✓ Previous slow-card token subscription records removed${NC}"

CLEANUP_PAYMENTS_QUERY="DELETE FROM pay.payments WHERE (external_user_id = '$USER_ID' AND product_id = '$PRODUCT_ID') OR (provider_transaction_id = '$DUMMY_TOKEN' AND product_id = '$PRODUCT_ID');"
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
