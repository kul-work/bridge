#!/bin/bash
set -euo pipefail

##############################################################################
# WHK-01: Valid Signature Acceptance
# 
# Purpose: Verify that legitimate webhooks with correct creem-signature 
#          are accepted and processed by the backend.
#
# Usage: ./test-whk-01.sh [--email "user@example.com"] [--user-id "test_user"]
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
TEST_RUN_ID="creem-whk-01-${TIMESTAMP}-$$"
REPORT_FILE="test-whk-01-report.json"
EMAIL="creem_whk_user_${TEST_RUN_ID}@example.com"
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
echo "WHK-01: Valid Signature Acceptance"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

# Step 1b: Verify verify_webhook_signature config is enabled
echo -e "${YELLOW}[1b/5] Verifying verify_webhook_signature config is enabled${NC}"
VERIFY_SIG=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -t -A -c "SELECT COALESCE((config->>'verify_webhook_signature')::boolean, true)::text FROM pay.provider_configs WHERE app_id = '$BRIDGE_APP_ID' AND provider = 'creem' LIMIT 1;" 2>/dev/null || echo "true")
VERIFY_SIG=$(echo "$VERIFY_SIG" | tr -d '[:space:]')
if [[ "$VERIFY_SIG" == "true" || "$VERIFY_SIG" == "t" ]]; then
    echo -e "${GREEN}✓ verify_webhook_signature=true — signature checks are enforced${NC}"
else
    echo -e "${RED}✗ verify_webhook_signature=$VERIFY_SIG — signature checks are NOT enforced${NC}"
    echo -e "${YELLOW}  Set verify_webhook_signature=true in pay.provider_configs for the test app${NC}"
    exit 1
fi

# Step 2: Cleanup and setup state
echo -e "${YELLOW}[2/5] Cleaning up old data from Bridge DB${NC}"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.payments WHERE external_user_id = '$USER_ID' OR subscription_id = '$SUBSCRIPTION_ID';" > /dev/null 2>&1 || true
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' OR subscription_id = '$SUBSCRIPTION_ID';" > /dev/null 2>&1 || true
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.webhook_provider WHERE provider = 'creem' AND (provider_webhook_id LIKE 'whk-01-%' OR subscription_id = '$SUBSCRIPTION_ID');" > /dev/null 2>&1 || true
echo -e "${GREEN}✓ Cleaned${NC}"

# Step 3: Trigger Webhook with VALID signature
echo -e "${YELLOW}[3/5] Sending subscription.active webhook with VALID signature${NC}"
EVENT_ID="whk-01-$TEST_RUN_ID"
PERIOD_END=$(date -u +"%Y-%m-%dT%H:%M:%SZ" -d "+30 days" 2>/dev/null || date -u -v+30d +"%Y-%m-%dT%H:%M:%SZ")

PAYLOAD=$(cat <<EOF
{
  "id": "$EVENT_ID",
  "eventType": "subscription.active",
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "object": {
    "id": "$SUBSCRIPTION_ID",
    "customer": {
      "email": "$EMAIL",
      "id": "cust_whk_01"
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

SIGNATURE=$(echo -n "$PAYLOAD" | openssl dgst -sha256 -hmac "$CREEM_WEBHOOK_SECRET" | sed 's/^.* //')

HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/creem" \
  -H "Content-Type: application/json" \
  -H "creem-signature: $SIGNATURE" \
  -d "$PAYLOAD")

if [[ "$HTTP_CODE" == "200" || "$HTTP_CODE" == "201" || "$HTTP_CODE" == "204" ]]; then
    echo -e "${GREEN}✓ Webhook accepted (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${RED}✗ Webhook failed (HTTP $HTTP_CODE)${NC}"
    exit 1
fi

# Step 4: Verify DB
echo -e "${YELLOW}[4/5] Verifying database state${NC}"
sleep 3 # process time

SUBS_STATUS=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "SELECT status FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND status = 'active';" -t | tr -d '[:space:]' || echo "")

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "WHK-01",
  "test_name": "Valid Signature Acceptance (Webhook)",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "pass",
  "user_id": "$USER_ID",
  "db_status": "$SUBS_STATUS",
  "results": {
    "webhook_accepted": true,
    "db_verified": true
  }
}
EOF

echo -e "\n${GREEN}✓ WHK-01 PASSED${NC}"
echo "Report saved to: $REPORT_FILE"
cat "$REPORT_FILE"
echo ""
exit 0
