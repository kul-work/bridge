#!/bin/bash

##############################################################################
# ERR-02: Invalid Customer Portal Call
# 
# Purpose: Verify that calling the billing portal endpoint with an invalid
#          subscription ID or non-existent user results in a proper error.
#
# Usage: ./test-err-02.sh [--email "user@example.com"] [--user-id "test_user"]
#
# Prerequisites:
#   - Backend running and accessible at $BRIDGE_API_URL (via globals.cfg)
#   - globals.cfg sourced with required vars:
#     * BRIDGE_API_KEY (for API access)
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER, PGPASSWORD
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
TEST_RUN_ID="creem-err-02-${TIMESTAMP}-$$"
REPORT_FILE="test-err-02-report.json"
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
echo "ERR-02: Invalid Customer Portal Call"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

# Step 1: Attempt call with non-existent subscription ID
echo -e "${YELLOW}[1/2] Attempting billing portal call with invalid subscription ID${NC}"

# Bridge endpoint: POST /api/v1/subscriptions/:subscription_id/portal
# It likely needs external_user_id to identify the customer
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$APP_URL/api/v1/subscriptions/invalid_sub_123/portal?external_user_id=$USER_ID" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $BRIDGE_API_KEY")

HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
BODY=$(echo "$RESPONSE" | head -n -1)

echo "  Response: HTTP $HTTP_CODE"
echo "  Body: $BODY"

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
OVERALL_STATUS="fail"
if [[ "$HTTP_CODE" == "404" || "$HTTP_CODE" == "400" || "$HTTP_CODE" == "500" ]]; then
    OVERALL_STATUS="pass"
fi

cat > "$REPORT_FILE" <<EOF
{
  "test_id": "ERR-02",
  "test_name": "Invalid Customer Portal Call",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "$OVERALL_STATUS",
  "user_id": "$USER_ID",
  "results": {
    "http_code": $HTTP_CODE,
    "error_body": $(echo "$BODY" | jq -R . 2>/dev/null || echo "\"$BODY\"")
  }
}
EOF

if [[ "$OVERALL_STATUS" == "pass" ]]; then
    echo -e "${GREEN}✓ Properly rejected invalid request.${NC}"
    echo -e "\n${GREEN}✓ ERR-02 PASSED${NC}"
    echo "Report saved to: $REPORT_FILE"
    cat "$REPORT_FILE"
    echo ""
    exit 0
else
    echo -e "${RED}✗ Error: Unexpected response for invalid portal call (HTTP $HTTP_CODE)${NC}"
    echo "Report saved to: $REPORT_FILE"
    cat "$REPORT_FILE"
    echo ""
    exit 1
fi
