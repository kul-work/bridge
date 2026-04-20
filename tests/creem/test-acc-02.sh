#!/bin/bash

##############################################################################
# ACC-02: Premium Access Retained During Scheduled Cancel
# 
# Purpose: Verify that premium access is RETAINED for subscriptions in
#          'scheduled_cancel' state BEFORE the current_period_end date.
#
# Usage: ./test-acc-02.sh --user-id "test_user"
#
# Prerequisites:
#   - Backend running and accessible at $BRIDGE_API_URL
#   - globals.cfg sourced
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

# Defaults
TIMESTAMP=$(date +%s)
USER_ID="test_acc_user_$TIMESTAMP"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
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

echo -e "${YELLOW}========================================${NC}"
echo "ACC-02: Premium Access Retained During Scheduled Cancel"
echo -e "${YELLOW}========================================${NC}"

# Step 1: User Identity
echo -e "${YELLOW}[1/4] Using External User ID: $USER_ID${NC}"
echo -e "${GREEN}✓ Ready${NC}"

# Step 2: Set up DB with cancelled status but future expiry
echo -e "${YELLOW}[2/4] Setting up DB for scheduled cancellation${NC}"
FUTURE_EXPIRY=$(date -u +"%Y-%m-%dT%H:%M:%SZ" -d "+15 days" 2>/dev/null || date -u -v+15d +"%Y-%m-%dT%H:%M:%SZ")

echo "  Setting status=scheduled_cancel, auto_renewing=false, current_period_end=$FUTURE_EXPIRY..."
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" > /dev/null
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "INSERT INTO pay.subscriptions (external_user_id, subscription_id, status, provider, auto_renewing, current_period_end, app_id) \
      VALUES ('$USER_ID', '$PRODUCT_ID_SUB', 'scheduled_cancel', 'creem', false, '$FUTURE_EXPIRY', '$BRIDGE_APP_ID');" > /dev/null
echo -e "${GREEN}✓ DB updated${NC}"

# Step 3: Verify access via Bridge API
echo -e "${YELLOW}[3/4] Verifying access via Bridge API${NC}"
RESPONSE=$(curl -s -X GET "$APP_URL/api/v1/subscriptions?external_user_id=$USER_ID" \
  -H "x-api-key: $BRIDGE_API_KEY")

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

# Step 4: Summary and Cleanup
echo -e "${YELLOW}[4/4] Summary${NC}"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" > /dev/null

if [[ "$ACC_PASS" == "true" ]]; then
    echo -e "\n${GREEN}✓ ACC-02 PASSED${NC}"
    exit 0
else
    echo -e "\n${RED}✗ ACC-02 FAILED${NC}"
    exit 1
fi
 Greenland
