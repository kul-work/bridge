#!/bin/bash

##############################################################################
# NET-01: Webhook Retry & Backoff
# 
# Purpose: Verify that the backend handles retried webhooks correctly.
#          Simulates an initial failure (e.g., bad signature)
#          followed by a successful retry.
#
# Usage: ./test-net-01.sh [--email "user@example.com"] [--user-id "test_user"]
#
# Prerequisites:
#   - Backend running and accessible at $BRIDGE_API_URL (via globals.cfg)
#   - globals.cfg sourced with required vars:
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER, PGPASSWORD
#     * WEBHOOK_INGRESS_TOKEN, CREEM_WEBHOOK_SECRET (for simulation)
#     * PRODUCT_ID_SUB (for payload)
#   - psql installed and database accessible
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

# Test configuration
TIMESTAMP=$(date +%s)
TEST_STARTED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
TEST_RUN_ID="creem-net-01-${TIMESTAMP}-$$"
REPORT_FILE="test-net-01-report.json"
EMAIL="creem_net_user_${TEST_RUN_ID}@example.com"
USER_ID=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --email)
            EMAIL="$2"
            shift 2
            ;;
        --user-id)
            USER_ID="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

if [[ -z "$USER_ID" ]]; then
    # Generate a unique USER_ID for this run
    USER_ID="creem_user_$TEST_RUN_ID"
fi

echo -e "${YELLOW}========================================${NC}"
echo "NET-01: Webhook Retry & Backoff"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

# Step 2: Cleanup
echo -e "${YELLOW}[2/4] Cleaning initial state for user $USER_ID${NC}"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" > /dev/null
echo -e "${GREEN}✓ Cleaned${NC}"

# Step 3: Send Webhook that "FAILS" (Bad Signature)
echo -e "${YELLOW}[3/4] Sending webhook with INVALID signature (Simulating failure)${NC}"
EVENT_ID="net-01-$TEST_RUN_ID"
PERIOD_END=$(date -u +"%Y-%m-%dT%H:%M:%SZ" -d "+30 days" 2>/dev/null || date -u -v+30d +"%Y-%m-%dT%H:%M:%SZ")

PAYLOAD=$(cat <<EOF
{
  "id": "$EVENT_ID",
  "eventType": "subscription.active",
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "object": {
    "id": "sub-$EVENT_ID",
    "customer": {
      "email": "$EMAIL",
      "id": "cust_net_01"
    },
    "metadata": {
      "user_id": "$USER_ID"
    },
    "status": "active",
    "product_id": "$PRODUCT_ID_SUB",
    "current_period_end_date": "$PERIOD_END"
  }
}
EOF
)

# Bad signature
HTTP_CODE_1=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/creem" \
  -H "Content-Type: application/json" \
  -H "creem-signature: BAD_SIG" \
  -d "$PAYLOAD")

echo "  Initial attempt (bad sig) response: HTTP $HTTP_CODE_1"

# Step 4: Send Webhook and Succeed (Retry)
echo -e "${YELLOW}[4/4] Sending same webhook with VALID signature (Simulating retry success)${NC}"

SIGNATURE=$(echo -n "$PAYLOAD" | openssl dgst -sha256 -hmac "$CREEM_WEBHOOK_SECRET" | sed 's/^.* //')

HTTP_CODE_2=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/creem" \
  -H "Content-Type: application/json" \
  -H "creem-signature: $SIGNATURE" \
  -d "$PAYLOAD")

echo "  Retry attempt (valid sig) response: HTTP $HTTP_CODE_2"

# Verification
sleep 2 # process time
SUBS_STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "SELECT status FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND status = 'active';" -t | tr -d '[:space:]' || echo "")

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
OVERALL_STATUS="fail"
if [[ ("$HTTP_CODE_1" == "401" || "$HTTP_CODE_1" == "400" || "$HTTP_CODE_1" == "403") && ("$HTTP_CODE_2" == "200" || "$HTTP_CODE_2" == "201" || "$HTTP_CODE_2" == "204") && "$SUBS_STATUS" == "active" ]]; then
    OVERALL_STATUS="pass"
fi

cat > "$REPORT_FILE" <<EOF
{
  "test_id": "NET-01",
  "test_name": "Webhook Retry & Backoff",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "$OVERALL_STATUS",
  "user_id": "$USER_ID",
  "results": {
    "first_http_code": $HTTP_CODE_1,
    "retry_http_code": $HTTP_CODE_2,
    "db_status": "$SUBS_STATUS"
  }
}
EOF

if [[ "$OVERALL_STATUS" == "pass" ]]; then
    echo -e "${GREEN}✓ Webhook retry handled correctly. Entitlement granted.${NC}"
    echo -e "\n${GREEN}✓ NET-01 PASSED${NC}"
    echo "Report saved to: $REPORT_FILE"
    cat "$REPORT_FILE"
    echo ""
    exit 0
else
    echo -e "${RED}✗ Webhook retry failed or logic incorrect${NC}"
    echo "  Status in DB: $SUBS_STATUS"
    echo "Report saved to: $REPORT_FILE"
    cat "$REPORT_FILE"
    echo ""
    exit 1
fi
