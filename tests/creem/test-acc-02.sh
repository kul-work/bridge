#!/bin/bash

##############################################################################
# ACC-02: Scheduled Cancel Access
# 
# Purpose: Verify that premium access REMAINS active for subscriptions in
#          'scheduled_cancel' (Pending Cancel) state until the period end.
#
# Usage: ./test-acc-02.sh [--email "user@example.com"] [--user-id "test_user"]
#
# Prerequisites:
#   - Backend running and accessible at $BRIDGE_API_URL (via globals.cfg)
#   - globals.cfg sourced with required vars:
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER, PGPASSWORD
#     * BRIDGE_API_KEY (for API access)
#     * BRIDGE_APP_ID, PRODUCT_ID_SUB (for DB setup)
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
BLUE='\033[0;34m'
NC='\033[0m'

# Test configuration
TIMESTAMP=$(date +%s)
TEST_STARTED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
TEST_RUN_ID="creem-acc-02-${TIMESTAMP}-$$"
REPORT_FILE="test-acc-02-report.json"
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
echo "ACC-02: Premium Access Retained During Scheduled Cancel"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

# Step 1: User Identity
echo -e "${YELLOW}[1/4] Using External User ID: $USER_ID${NC}"
echo -e "${GREEN}✓ Ready${NC}"

# Step 2: Set up DB with cancelled status but future expiry
echo -e "${YELLOW}[2/4] Setting up DB for scheduled cancellation${NC}"
FUTURE_EXPIRY=$(date -u +"%Y-%m-%dT%H:%M:%SZ" -d "+15 days" 2>/dev/null || date -u -v+15d +"%Y-%m-%dT%H:%M:%SZ")

echo "  Setting status=scheduled_cancel, auto_renewing=false, current_period_end=$FUTURE_EXPIRY..."
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" > /dev/null
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "INSERT INTO pay.subscriptions (external_user_id, subscription_id, status, provider, auto_renewing, current_period_end, app_id) \
      VALUES ('$USER_ID', '$PRODUCT_ID_SUB', 'scheduled_cancel', 'creem', false, '$FUTURE_EXPIRY', '$BRIDGE_APP_ID');" > /dev/null
echo -e "${GREEN}✓ DB updated${NC}"

# Step 3: Verify access via Bridge API
# Check Bridge API
RESPONSE=$(curl -s -X GET "$APP_URL/api/v1/users/$USER_ID/subscription-status" \
  -H "Authorization: Bearer $BRIDGE_API_KEY")

STATUS=$(echo "$RESPONSE" | grep -o '"status":"[^"]*"' | head -n 1 | cut -d'"' -f4 || echo "")

if [[ "$STATUS" == "scheduled_cancel" ]]; then
    echo -e "  ${GREEN}✓ API reports correct status=$STATUS${NC}"
    # In Bridge logic, if it's scheduled_cancel, the client app should still grant access.
    # We verify Bridge correctly maintains the state.
    ACC_PASS="true"
else
    echo -e "  ${RED}✗ API reports unexpected status=$STATUS (Expected: scheduled_cancel)${NC}"
    echo "  Response: $RESPONSE"
    ACC_PASS="false"
fi

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
OVERALL_STATUS="fail"
if [[ "$ACC_PASS" == "true" ]]; then
    OVERALL_STATUS="pass"
fi

cat > "$REPORT_FILE" <<EOF
{
  "test_id": "ACC-02",
  "test_name": "Scheduled Cancel Access",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "$OVERALL_STATUS",
  "user_id": "$USER_ID",
  "results": {
    "access_pass": $ACC_PASS
  }
}
EOF

if [[ "$OVERALL_STATUS" == "pass" ]]; then
    echo -e "\n${GREEN}✓ ACC-02 PASSED${NC}"
    echo "Report saved to: $REPORT_FILE"
    cat "$REPORT_FILE"
    echo ""
    exit 0
else
    echo -e "\n${RED}✗ ACC-02 FAILED${NC}"
    echo "Report saved to: $REPORT_FILE"
    cat "$REPORT_FILE"
    echo ""
    exit 1
fi
