#!/bin/bash

##############################################################################
# NET-CREEM-CALLBACK-BODY: Bridge-to-App Creem Callback Body Contract
#
# Purpose: Verify that representative Creem lifecycle webhooks deliver the
#          expected normalized callback JSON fields to the app callback URL.
#
# Usage: ./tests/test-net-creem-callback-body.sh
#
# Prerequisites:
#   - Bridge Backend running with MOCK_EXTERNAL_APIS=true
#   - tests/creem/globals.cfg configured
#   - psql, curl, jq, python, and openssl installed and in PATH
##############################################################################

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/creem/globals.cfg"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

for cmd in psql curl jq python openssl; do
    if ! command -v "$cmd" >/dev/null 2>&1; then
        echo -e "${RED}FAIL: $cmd is required${NC}"
        exit 1
    fi
done

if [[ -z "${WEBHOOK_INGRESS_TOKEN:-}" ]]; then
    echo -e "${RED}FAIL: WEBHOOK_INGRESS_TOKEN is required${NC}"
    exit 1
fi

if [[ -z "${CREEM_WEBHOOK_SECRET:-}" ]]; then
    echo -e "${RED}FAIL: CREEM_WEBHOOK_SECRET is required${NC}"
    exit 1
fi

TIMESTAMP=$(date +%s)
TEST_STARTED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
TEST_RUN_ID="net-creem-callback-body-${TIMESTAMP}-$$"
TMP_DIR="${TMPDIR:-/tmp}/bridge-$TEST_RUN_ID"
CAPTURE_FILE="$TMP_DIR/callbacks.jsonl"
RECEIVER_SCRIPT="$TMP_DIR/callback_receiver.py"
CALLBACK_PORT="${CALLBACK_PORT:-$((19080 + ($$ % 1000)))}"
CALLBACK_URL="http://127.0.0.1:$CALLBACK_PORT/bridge-callback"
RECEIVER_PID=""
ORIGINAL_CALLBACK_URL=""
TEST_STATUS="fail"

mkdir -p "$TMP_DIR"
: > "$CAPTURE_FILE"

cleanup() {
    if [[ -n "$ORIGINAL_CALLBACK_URL" ]]; then
        local escaped_callback_url
        escaped_callback_url=$(printf '%s' "$ORIGINAL_CALLBACK_URL" | sed "s/'/''/g")
        psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
          -c "UPDATE pay.apps SET webhook_callback_url = '$escaped_callback_url' WHERE slug = 'hiha';" >/dev/null 2>&1 || true
    fi

    if [[ -n "$RECEIVER_PID" ]]; then
        kill "$RECEIVER_PID" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT

cat > "$RECEIVER_SCRIPT" <<'PY'
import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

capture_file = sys.argv[1]
port = int(sys.argv[2])

class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b"ok")

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        raw_body = self.rfile.read(length).decode("utf-8")
        try:
            body = json.loads(raw_body) if raw_body else None
        except json.JSONDecodeError:
            body = raw_body

        record = {
            "path": self.path,
            "headers": dict(self.headers),
            "body": body,
        }
        with open(capture_file, "a", encoding="utf-8") as f:
            f.write(json.dumps(record, separators=(",", ":")) + "\n")

        self.send_response(204)
        self.end_headers()

    def log_message(self, format, *args):
        return

server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
server.serve_forever()
PY

python "$RECEIVER_SCRIPT" "$CAPTURE_FILE" "$CALLBACK_PORT" &
RECEIVER_PID=$!

for i in {1..20}; do
    if curl -fsS "http://127.0.0.1:$CALLBACK_PORT/" >/dev/null 2>&1; then
        break
    fi
    if [[ "$i" -eq 20 ]]; then
        echo -e "${RED}FAIL: callback capture server did not start${NC}"
        exit 1
    fi
    sleep 1
done

ORIGINAL_CALLBACK_URL=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "SELECT webhook_callback_url FROM pay.apps WHERE slug = 'hiha' LIMIT 1;" -t | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')

if [[ -z "$ORIGINAL_CALLBACK_URL" ]]; then
    echo -e "${RED}FAIL: could not load original hiha callback URL${NC}"
    exit 1
fi

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "UPDATE pay.apps SET webhook_callback_url = '$CALLBACK_URL' WHERE slug = 'hiha';" >/dev/null

echo -e "${YELLOW}========================================${NC}"
echo "NET-CREEM-CALLBACK-BODY: Bridge-to-App Creem Callback Body Contract"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo "Callback capture URL: $CALLBACK_URL"
echo ""

period_end_30_days() {
    date -u -d "+30 days" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || date -u -v+30d +"%Y-%m-%dT%H:%M:%SZ"
}

send_creem_webhook() {
    local payload="$1"
    local signature
    local http_code

    signature=$(echo -n "$payload" | openssl dgst -sha256 -hmac "$CREEM_WEBHOOK_SECRET" | sed 's/^.* //')
    http_code=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
      "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/creem" \
      -H "Content-Type: application/json" \
      -H "creem-signature: $signature" \
      -d "$payload")

    if [[ "$http_code" != "200" && "$http_code" != "201" && "$http_code" != "204" ]]; then
        echo -e "${RED}FAIL: Creem webhook returned HTTP $http_code${NC}"
        echo "$payload" | jq . || echo "$payload"
        exit 1
    fi
}

wait_for_callback() {
    local user_id="$1"
    local jq_filter="$2"
    local label="$3"

    for i in {1..20}; do
        if jq -s -e --arg user_id "$user_id" \
          "map(select(.body.external_user_id == \$user_id and (.body | $jq_filter))) | length > 0" \
          "$CAPTURE_FILE" >/dev/null 2>&1; then
            echo -e "${GREEN}PASS: $label${NC}"
            return 0
        fi
        sleep 1
    done

    echo -e "${RED}FAIL: callback body not observed for $label${NC}"
    echo -e "${BLUE}Captured callbacks for user $user_id:${NC}"
    jq -s --arg user_id "$user_id" 'map(select(.body.external_user_id == $user_id) | .body)' "$CAPTURE_FILE" || true
    exit 1
}

echo -e "${YELLOW}[1/7] One-time checkout callback body${NC}"
OTP_USER="test_creem_callback_otp_$TEST_RUN_ID"
OTP_EVENT_ID="evt_creem_callback_otp_$TEST_RUN_ID"
OTP_CHECKOUT_ID="checkout_creem_callback_otp_$TEST_RUN_ID"
OTP_PAYLOAD=$(cat <<EOF
{
  "id": "$OTP_EVENT_ID",
  "eventType": "checkout.completed",
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "object": {
    "id": "$OTP_CHECKOUT_ID",
    "checkout_id": "$OTP_CHECKOUT_ID",
    "billing_type": "one_time",
    "customer": {
      "email": "$EMAIL",
      "id": "cust_creem_callback_otp"
    },
    "metadata": {
      "user_id": "$OTP_USER"
    },
    "product_id": "$PRODUCT_ID_OTP",
    "status": "completed",
    "last_transaction": {
      "id": "tx_creem_callback_otp",
      "amount": 2999
    }
  }
}
EOF
)
send_creem_webhook "$OTP_PAYLOAD"
wait_for_callback "$OTP_USER" ".event_type == \"purchase.one_time\" and .provider == \"creem\" and .product_id == \"$PRODUCT_ID_OTP\" and .purchase_token == \"$OTP_CHECKOUT_ID\" and .amount_cents == 2999" "one-time purchase fields delivered"
echo ""

echo -e "${YELLOW}[2/7] Subscription active callback body${NC}"
ACTIVE_USER="test_creem_callback_active_$TEST_RUN_ID"
ACTIVE_EVENT_ID="evt_creem_callback_active_$TEST_RUN_ID"
ACTIVE_SUB_ID="sub_creem_callback_active_$TEST_RUN_ID"
ACTIVE_PERIOD_END=$(period_end_30_days)
ACTIVE_PAYLOAD=$(cat <<EOF
{
  "id": "$ACTIVE_EVENT_ID",
  "eventType": "subscription.active",
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "object": {
    "id": "$ACTIVE_SUB_ID",
    "customer": {
      "email": "$EMAIL",
      "id": "cust_creem_callback_active"
    },
    "metadata": {
      "user_id": "$ACTIVE_USER"
    },
    "status": "active",
    "product_id": "$PRODUCT_ID_SUB",
    "current_period_end_date": "$ACTIVE_PERIOD_END",
    "auto_renewing": true,
    "last_transaction": {
      "amount": 2999
    }
  }
}
EOF
)
send_creem_webhook "$ACTIVE_PAYLOAD"
wait_for_callback "$ACTIVE_USER" ".event_type == \"subscription.activated\" and .provider == \"creem\" and .subscription_id == \"$ACTIVE_SUB_ID\" and .product_id == \"$PRODUCT_ID_SUB\" and .status == \"active\" and .auto_renewing == true and .current_period_end != null" "active subscription fields delivered"
echo ""

echo -e "${YELLOW}[3/7] Trial subscription callback body${NC}"
TRIAL_USER="test_creem_callback_trial_$TEST_RUN_ID"
TRIAL_EVENT_ID="evt_creem_callback_trial_$TEST_RUN_ID"
TRIAL_SUB_ID="sub_creem_callback_trial_$TEST_RUN_ID"
TRIAL_PERIOD_END=$(period_end_30_days)
TRIAL_PAYLOAD=$(cat <<EOF
{
  "id": "$TRIAL_EVENT_ID",
  "eventType": "subscription.trialing",
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "object": {
    "id": "$TRIAL_SUB_ID",
    "customer": {
      "email": "$EMAIL",
      "id": "cust_creem_callback_trial"
    },
    "metadata": {
      "user_id": "$TRIAL_USER"
    },
    "status": "trialing",
    "product_id": "$PRODUCT_ID_SUB",
    "current_period_end_date": "$TRIAL_PERIOD_END",
    "auto_renewing": true
  }
}
EOF
)
send_creem_webhook "$TRIAL_PAYLOAD"
wait_for_callback "$TRIAL_USER" ".event_type == \"subscription.activated\" and .subscription_id == \"$TRIAL_SUB_ID\" and .status == \"trial\" and .current_period_end != null" "trial subscription fields delivered"
echo ""

echo -e "${YELLOW}[4/7] Scheduled cancellation callback body${NC}"
SCHEDULED_USER="test_creem_callback_scheduled_$TEST_RUN_ID"
SCHEDULED_SUB_ID="sub_creem_callback_scheduled_$TEST_RUN_ID"
SCHEDULED_PERIOD_END=$(period_end_30_days)
SCHEDULED_ACTIVE_PAYLOAD=$(cat <<EOF
{
  "id": "evt_creem_callback_scheduled_active_$TEST_RUN_ID",
  "eventType": "subscription.active",
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "object": {
    "id": "$SCHEDULED_SUB_ID",
    "customer": {
      "email": "$EMAIL",
      "id": "cust_creem_callback_scheduled"
    },
    "metadata": {
      "user_id": "$SCHEDULED_USER"
    },
    "status": "active",
    "product_id": "$PRODUCT_ID_SUB",
    "current_period_end_date": "$SCHEDULED_PERIOD_END",
    "auto_renewing": true
  }
}
EOF
)
send_creem_webhook "$SCHEDULED_ACTIVE_PAYLOAD"
wait_for_callback "$SCHEDULED_USER" ".event_type == \"subscription.activated\" and .subscription_id == \"$SCHEDULED_SUB_ID\" and .status == \"active\"" "scheduled-cancel setup activation delivered"
sleep 1
SCHEDULED_PAYLOAD=$(cat <<EOF
{
  "id": "evt_creem_callback_scheduled_cancel_$TEST_RUN_ID",
  "eventType": "subscription.scheduled_cancel",
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "object": {
    "id": "$SCHEDULED_SUB_ID",
    "customer": {
      "email": "$EMAIL",
      "id": "cust_creem_callback_scheduled"
    },
    "metadata": {
      "user_id": "$SCHEDULED_USER"
    },
    "status": "active",
    "product_id": "$PRODUCT_ID_SUB",
    "auto_renewing": false
  }
}
EOF
)
send_creem_webhook "$SCHEDULED_PAYLOAD"
wait_for_callback "$SCHEDULED_USER" ".event_type == \"subscription.cancelled\" and .subscription_id == \"$SCHEDULED_SUB_ID\" and .status == \"active\" and .auto_renewing == false and .cancellation_mode == \"scheduled\"" "scheduled cancellation fields delivered"
echo ""

echo -e "${YELLOW}[5/7] Immediate cancellation callback body${NC}"
CANCEL_USER="test_creem_callback_cancel_$TEST_RUN_ID"
CANCEL_SUB_ID="sub_creem_callback_cancel_$TEST_RUN_ID"
CANCEL_PERIOD_END=$(period_end_30_days)
CANCEL_ACTIVE_PAYLOAD=$(cat <<EOF
{
  "id": "evt_creem_callback_cancel_active_$TEST_RUN_ID",
  "eventType": "subscription.active",
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "object": {
    "id": "$CANCEL_SUB_ID",
    "customer": {
      "email": "$EMAIL",
      "id": "cust_creem_callback_cancel"
    },
    "metadata": {
      "user_id": "$CANCEL_USER"
    },
    "status": "active",
    "product_id": "$PRODUCT_ID_SUB",
    "current_period_end_date": "$CANCEL_PERIOD_END",
    "auto_renewing": true
  }
}
EOF
)
send_creem_webhook "$CANCEL_ACTIVE_PAYLOAD"
CANCEL_PAYLOAD=$(cat <<EOF
{
  "id": "evt_creem_callback_cancelled_$TEST_RUN_ID",
  "eventType": "subscription.canceled",
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "object": {
    "id": "$CANCEL_SUB_ID",
    "customer": {
      "email": "$EMAIL",
      "id": "cust_creem_callback_cancel"
    },
    "metadata": {
      "user_id": "$CANCEL_USER"
    },
    "status": "canceled",
    "product_id": "$PRODUCT_ID_SUB",
    "auto_renewing": false
  }
}
EOF
)
send_creem_webhook "$CANCEL_PAYLOAD"
wait_for_callback "$CANCEL_USER" ".event_type == \"subscription.cancelled\" and .subscription_id == \"$CANCEL_SUB_ID\" and (.status == \"cancelled\" or .status == \"canceled\")" "immediate cancellation fields delivered"
echo ""

echo -e "${YELLOW}[6/7] Paused and expired callback bodies${NC}"
PAUSE_USER="test_creem_callback_pause_$TEST_RUN_ID"
PAUSE_SUB_ID="sub_creem_callback_pause_$TEST_RUN_ID"
PAUSE_PERIOD_END=$(period_end_30_days)
PAUSE_ACTIVE_PAYLOAD=$(cat <<EOF
{
  "id": "evt_creem_callback_pause_active_$TEST_RUN_ID",
  "eventType": "subscription.active",
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "object": {
    "id": "$PAUSE_SUB_ID",
    "customer": {
      "email": "$EMAIL",
      "id": "cust_creem_callback_pause"
    },
    "metadata": {
      "user_id": "$PAUSE_USER"
    },
    "status": "active",
    "product_id": "$PRODUCT_ID_SUB",
    "current_period_end_date": "$PAUSE_PERIOD_END",
    "auto_renewing": true
  }
}
EOF
)
send_creem_webhook "$PAUSE_ACTIVE_PAYLOAD"
wait_for_callback "$PAUSE_USER" ".event_type == \"subscription.activated\" and .subscription_id == \"$PAUSE_SUB_ID\" and .status == \"active\"" "pause setup activation delivered"
sleep 1
PAUSE_PAYLOAD=$(cat <<EOF
{
  "id": "evt_creem_callback_paused_$TEST_RUN_ID",
  "eventType": "subscription.paused",
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "object": {
    "id": "$PAUSE_SUB_ID",
    "customer": {
      "email": "$EMAIL",
      "id": "cust_creem_callback_pause"
    },
    "metadata": {
      "user_id": "$PAUSE_USER"
    },
    "status": "paused",
    "product_id": "$PRODUCT_ID_SUB"
  }
}
EOF
)
send_creem_webhook "$PAUSE_PAYLOAD"
wait_for_callback "$PAUSE_USER" ".event_type == \"subscription.paused\" and .subscription_id == \"$PAUSE_SUB_ID\" and .status == \"paused\"" "paused subscription fields delivered"

EXPIRED_USER="test_creem_callback_expired_$TEST_RUN_ID"
EXPIRED_SUB_ID="sub_creem_callback_expired_$TEST_RUN_ID"
EXPIRED_PERIOD_END=$(period_end_30_days)
EXPIRED_ACTIVE_PAYLOAD=$(cat <<EOF
{
  "id": "evt_creem_callback_expired_active_$TEST_RUN_ID",
  "eventType": "subscription.active",
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "object": {
    "id": "$EXPIRED_SUB_ID",
    "customer": {
      "email": "$EMAIL",
      "id": "cust_creem_callback_expired"
    },
    "metadata": {
      "user_id": "$EXPIRED_USER"
    },
    "status": "active",
    "product_id": "$PRODUCT_ID_SUB",
    "current_period_end_date": "$EXPIRED_PERIOD_END",
    "auto_renewing": true
  }
}
EOF
)
send_creem_webhook "$EXPIRED_ACTIVE_PAYLOAD"
wait_for_callback "$EXPIRED_USER" ".event_type == \"subscription.activated\" and .subscription_id == \"$EXPIRED_SUB_ID\" and .status == \"active\"" "expiry setup activation delivered"
sleep 1
EXPIRED_PAYLOAD=$(cat <<EOF
{
  "id": "evt_creem_callback_expired_$TEST_RUN_ID",
  "eventType": "subscription.expired",
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "object": {
    "id": "$EXPIRED_SUB_ID",
    "customer": {
      "email": "$EMAIL",
      "id": "cust_creem_callback_expired"
    },
    "metadata": {
      "user_id": "$EXPIRED_USER"
    },
    "status": "expired",
    "product_id": "$PRODUCT_ID_SUB"
  }
}
EOF
)
send_creem_webhook "$EXPIRED_PAYLOAD"
wait_for_callback "$EXPIRED_USER" ".event_type == \"subscription.expired\" and .subscription_id == \"$EXPIRED_SUB_ID\" and .status == \"expired\"" "expired subscription fields delivered"
echo ""

echo -e "${YELLOW}[7/7] One-time refund callback body${NC}"
REFUND_USER="test_creem_callback_refund_$TEST_RUN_ID"
REFUND_CHECKOUT_ID="checkout_creem_callback_refund_$TEST_RUN_ID"
REFUND_PURCHASE_PAYLOAD=$(cat <<EOF
{
  "id": "evt_creem_callback_refund_purchase_$TEST_RUN_ID",
  "eventType": "checkout.completed",
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "object": {
    "id": "$REFUND_CHECKOUT_ID",
    "checkout_id": "$REFUND_CHECKOUT_ID",
    "billing_type": "one_time",
    "customer": {
      "email": "$EMAIL",
      "id": "cust_creem_callback_refund"
    },
    "metadata": {
      "user_id": "$REFUND_USER"
    },
    "product_id": "$PRODUCT_ID_OTP",
    "status": "completed",
    "last_transaction": {
      "id": "tx_creem_callback_refund",
      "amount": 2999
    }
  }
}
EOF
)
send_creem_webhook "$REFUND_PURCHASE_PAYLOAD"
REFUND_PAYLOAD=$(cat <<EOF
{
  "id": "evt_creem_callback_refund_$TEST_RUN_ID",
  "eventType": "refund.created",
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "object": {
    "id": "refund_creem_callback_$TEST_RUN_ID",
    "checkout_id": "$REFUND_CHECKOUT_ID",
    "billing_type": "one_time",
    "customer": {
      "email": "$EMAIL",
      "id": "cust_creem_callback_refund"
    },
    "metadata": {
      "user_id": "$REFUND_USER"
    },
    "product_id": "$PRODUCT_ID_OTP",
    "amount": 2999
  }
}
EOF
)
send_creem_webhook "$REFUND_PAYLOAD"
wait_for_callback "$REFUND_USER" ".event_type == \"purchase.one_time\" and .status == \"refunded\" and .revocation_reason == \"REFUND\" and .product_id == \"$PRODUCT_ID_OTP\"" "one-time refund fields delivered"
echo ""

TEST_STATUS="pass"
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)

echo -e "${GREEN}NET-CREEM-CALLBACK-BODY Bridge Test PASSED${NC}"
echo "Test Run ID: $TEST_RUN_ID"
echo "Started: $TEST_STARTED_AT"
echo "Finished: $TEST_FINISHED_AT"
echo "Status: $TEST_STATUS"
echo "Verified callback fields: one-time purchase, active subscription, trial, scheduled cancellation, immediate cancellation, pause, expiry, refund"
exit 0
