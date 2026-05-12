#!/bin/bash

##############################################################################
# ACC-01: Active Subscription Access
# 
# Purpose: Verify that users with 'active' status in the pay.subscriptions 
#          table correctly receive access via the Bridge API.
#
# Usage: ./test-acc-01.sh [--email "user@example.com"] [--user-id "test_user"]
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
TEST_RUN_ID="creem-acc-01-${TIMESTAMP}-$$"
REPORT_FILE="test-acc-01-report.json"
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
echo "ACC-01: Premium Access Granted for Active States"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

# Function to test access for a specific status
test_access_for_status() {
    local status="$1"
    echo -e "${BLUE}Testing status: $status${NC}"
    
    # Calculate future expiry
    local future_expiry=$(date -u +"%Y-%m-%dT%H:%M:%SZ" -d "+30 days" 2>/dev/null || date -u -v+30d +"%Y-%m-%dT%H:%M:%SZ")
    
    # Update DB
    echo "  Updating DB to status=$status, expiry=$future_expiry..."
    psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
      -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" > /dev/null
    psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
      -c "INSERT INTO pay.subscriptions (external_user_id, subscription_id, status, provider, auto_renewing, current_period_end, app_id) \
          VALUES ('$USER_ID', '$PRODUCT_ID_SUB', '$status', 'creem', true, '$future_expiry', '$BRIDGE_APP_ID');" > /dev/null

    # Check Bridge API
    echo "  Checking Bridge API GET /api/v1/users/:id/subscription-status..."
    local RESPONSE=$(curl -s -X GET "$APP_URL/api/v1/users/$USER_ID/subscription-status" \
      -H "Authorization: Bearer $BRIDGE_API_KEY")
    
    local RETURNED_STATUS=$(echo "$RESPONSE" | grep -o '"status":"[^"]*"' | head -n 1 | cut -d'"' -f4 || echo "")
    
    if [[ "$RETURNED_STATUS" == "$status" ]]; then
        echo -e "  ${GREEN}✓ API reports status=$RETURNED_STATUS${NC}"
    else
        echo -e "  ${RED}✗ API reports status=$RETURNED_STATUS (Expected: $status)${NC}"
        echo "  Response: $RESPONSE"
        return 1
    fi

    return 0
}

# Step 2: Test 'active' status
echo -e "${YELLOW}[2/4] Testing 'active' status${NC}"
ACTIVE_PASS="false"
if test_access_for_status "active"; then
    ACTIVE_PASS="true"
fi

# Step 3: Test 'trialing' status
echo -e "${YELLOW}[3/4] Testing 'trialing' status${NC}"
TRIAL_PASS="false"
if test_access_for_status "trialing"; then
    TRIAL_PASS="true"
fi

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
OVERALL_STATUS="fail"
if [[ "$ACTIVE_PASS" == "true" && "$TRIAL_PASS" == "true" ]]; then
    OVERALL_STATUS="pass"
fi

cat > "$REPORT_FILE" <<EOF
{
  "test_id": "ACC-01",
  "test_name": "Active Subscription Access",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "$OVERALL_STATUS",
  "user_id": "$USER_ID",
  "results": {
    "active_pass": $ACTIVE_PASS,
    "trialing_pass": $TRIAL_PASS
  }
}
EOF

if [[ "$OVERALL_STATUS" == "pass" ]]; then
    echo -e "\n${GREEN}✓ ACC-01 PASSED${NC}"
    echo "Report saved to: $REPORT_FILE"
    cat "$REPORT_FILE"
    echo ""
    exit 0
else
    echo -e "\n${RED}✗ ACC-01 FAILED${NC}"
    echo "Report saved to: $REPORT_FILE"
    cat "$REPORT_FILE"
    echo ""
    exit 1
fi
