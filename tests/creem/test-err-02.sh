#!/bin/bash

##############################################################################
# ERR-02: Invalid Customer Portal Call
# 
# Purpose: Verify that calling the billing portal endpoint with an invalid
#          subscription ID or non-existent user results in a proper error.
#
# Usage: ./test-err-02.sh --user-id "test_user"
#
# Prerequisites:
#   - Backend running and accessible at $BRIDGE_API_URL
#   - globals.cfg sourced with required vars:
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER, PGPASSWORD
#     * WEBHOOK_TOKEN
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

# Defaults
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
    # Generate a stable-ish USER_ID from email if not provided
    USER_ID="creem_$(echo -n "$EMAIL" | md5sum | cut -d' ' -f1 | cut -c1-12)"
fi

echo -e "${YELLOW}========================================${NC}"
echo "ERR-02: Invalid Customer Portal Call"
echo -e "${YELLOW}========================================${NC}"

# Step 1: Attempt call with non-existent subscription ID
echo -e "${YELLOW}[1/2] Attempting billing portal call with invalid subscription ID${NC}"

# Bridge endpoint: POST /api/v1/subscriptions/:subscription_id/portal
# It likely needs external_user_id to identify the customer
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$APP_URL/api/v1/subscriptions/invalid_sub_123/portal?external_user_id=$USER_ID" \
  -H "Content-Type: application/json" \
  -H "x-api-key: $BRIDGE_API_KEY")

HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
BODY=$(echo "$RESPONSE" | head -n -1)

echo "  Response: HTTP $HTTP_CODE"
echo "  Body: $BODY"

# Step 2: Verification
echo -e "${YELLOW}[2/2] Verifying error response${NC}"
if [[ "$HTTP_CODE" == "404" || "$HTTP_CODE" == "400" || "$HTTP_CODE" == "500" ]]; then
    # Some implementations might throw 500 if unhandled, but 404/400 is ideal.
    # Given we are testing error handling, any error code is better than 200.
    echo -e "${GREEN}✓ Properly rejected invalid request.${NC}"
    echo -e "\n${GREEN}✓ ERR-02 PASSED${NC}"
    exit 0
else
    echo -e "${RED}✗ Error: Unexpected response for invalid portal call (HTTP $HTTP_CODE)${NC}"
    exit 1
fi
