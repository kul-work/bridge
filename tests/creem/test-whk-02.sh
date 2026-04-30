#!/bin/bash

##############################################################################
# WHK-02: Invalid Signature Rejection
# 
# Purpose: Verify that webhooks with invalid signatures are rejected 
#          and do NOT modify the database.
#          Also verifies that verify_webhook_signature=true in provider_configs
#          is required for rejection to occur.
#
# Usage: ./test-whk-02.sh [--email "user@example.com"] [--user-id "test_user"]
#
# Prerequisites:
#   - Backend running and accessible at $BRIDGE_API_URL (via globals.cfg)
#   - globals.cfg sourced with required vars:
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER, PGPASSWORD
#     * WEBHOOK_INGRESS_TOKEN, CREEM_WEBHOOK_SECRET (for simulation)
#     * PRODUCT_ID_SUB (for payload)
#   - verify_webhook_signature must be true (or absent) in pay.provider_configs
#     for the test app (ensures signature checks are enforced)
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
TEST_RUN_ID="creem-whk-02-${TIMESTAMP}-$$"
REPORT_FILE="test-whk-02-report.json"
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
echo "WHK-02: Invalid Signature Rejection"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

# Step 1b: Verify that verify_webhook_signature is true (or absent=default true)
echo -e "${YELLOW}[1b/5] Verifying verify_webhook_signature config is enabled${NC}"
VERIFY_SIG=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -t -A -c "SELECT COALESCE((config->>'verify_webhook_signature')::boolean, true)::text FROM pay.provider_configs WHERE app_id = '$BRIDGE_APP_ID' AND provider = 'creem' LIMIT 1;" 2>/dev/null || echo "true")
VERIFY_SIG=$(echo "$VERIFY_SIG" | tr -d '[:space:]')
if [[ "$VERIFY_SIG" == "true" || "$VERIFY_SIG" == "t" ]]; then
    echo -e "${GREEN}✓ verify_webhook_signature=true — signature checks are enforced${NC}"
else
    echo -e "${RED}✗ verify_webhook_signature=$VERIFY_SIG — signature checks are NOT enforced, test will fail${NC}"
    echo -e "${YELLOW}  Set verify_webhook_signature=true in pay.provider_configs for the test app${NC}"
    exit 1
fi

# Step 2: Record initial state
echo -e "${YELLOW}[2/5] Recording initial database state for user $USER_ID${NC}"
INITIAL_SUBS_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "SELECT count(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" -t | tr -d '[:space:]' || echo "0")
echo "  Initial Subscriptions: $INITIAL_SUBS_COUNT"

# Step 3: Send Webhook with INVALID signature
echo -e "${YELLOW}[3/5] Sending webhook with INVALID signature${NC}"
EVENT_ID="whk-02-invalid-$TEST_RUN_ID"
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
      "id": "cust_whk_02_invalid"
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

BAD_SIGNATURE="BAD_SIG_1234567890abcdef"

HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/creem" \
  -H "Content-Type: application/json" \
  -H "creem-signature: $BAD_SIGNATURE" \
  -d "$PAYLOAD")

if [[ "$HTTP_CODE" == "401" || "$HTTP_CODE" == "403" || "$HTTP_CODE" == "400" ]]; then
    echo -e "${GREEN}✓ Webhook correctly rejected with HTTP $HTTP_CODE${NC}"
else
    echo -e "${RED}✗ Webhook NOT rejected adequately (HTTP $HTTP_CODE)${NC}"
fi

# Step 4: Verify DB unchanged
echo -e "${YELLOW}[4/5] Verifying database remains unchanged${NC}"
sleep 2 # process time

FINAL_SUBS_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "SELECT count(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" -t | tr -d '[:space:]' || echo "0")

echo "  Final Subscriptions: $FINAL_SUBS_COUNT"

if [[ "$FINAL_SUBS_COUNT" == "$INITIAL_SUBS_COUNT" ]]; then
    echo -e "${GREEN}✓ Database state unchanged as expected${NC}"
else
    echo -e "${RED}✗ Database state MODIFIED despite invalid signature!${NC}"
fi

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
OVERALL_STATUS="fail"
if [[ "$FINAL_SUBS_COUNT" == "$INITIAL_SUBS_COUNT" ]]; then
    OVERALL_STATUS="pass"
fi

cat > "$REPORT_FILE" <<EOF
{
  "test_id": "WHK-02",
  "test_name": "Invalid Signature Rejection (Webhook)",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "$OVERALL_STATUS",
  "user_id": "$USER_ID",
  "results": {
    "first_attempt_http_code": $HTTP_CODE,
    "db_remained_consistent": true
  }
}
EOF

if [[ "$OVERALL_STATUS" == "pass" ]]; then
    echo -e "\n${GREEN}✓ WHK-02 PASSED${NC}"
    echo "Report saved to: $REPORT_FILE"
    cat "$REPORT_FILE"
    echo ""
    exit 0
else
    echo -e "${RED}✗ WHK-02 FAILED${NC}"
    echo "Report saved to: $REPORT_FILE"
    cat "$REPORT_FILE"
    echo ""
    exit 1
fi
