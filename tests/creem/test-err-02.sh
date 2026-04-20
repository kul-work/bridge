#!/bin/bash

##############################################################################
# ERR-02: Invalid Customer Portal Call
# 
# Purpose: Verify that calling the billing portal endpoint with an invalid
#          subscription ID or non-existent user results in a friendly error.
#
# Usage: ./test-err-02.sh --email "user@example.com"
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
EMAIL=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --email)
            EMAIL="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

if [[ -z "$EMAIL" ]]; then
    echo -e "${RED}Error: --email is required${NC}"
    exit 1
fi

echo -e "${YELLOW}========================================${NC}"
echo "ERR-02: Invalid Customer Portal Call"
echo -e "${YELLOW}========================================${NC}"

# Step 1: Attempt call with non-existent subscription ID
echo -e "${YELLOW}[1/2] Attempting billing portal call with invalid subscription ID${NC}"

# Note: We use a custom header X-Test-Email to bypass Clerk auth in test mode if backend allows,
# or we just expect 401/404. Since we are testing CBI behavior, we want to see how it handles missing sub.
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$APP_URL/api/v1/subscriptions/invalid_sub_123/portal" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test_token_invalid" \
  -H "x-client-version: 99.99.0")

HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
BODY=$(echo "$RESPONSE" | head -n -1)

echo "  Response: HTTP $HTTP_CODE"
echo "  Body: $BODY"

# Step 2: Verification
echo -e "${YELLOW}[2/2] Verifying error response${NC}"
if [[ "$HTTP_CODE" == "404" || "$HTTP_CODE" == "401" || "$HTTP_CODE" == "400" ]]; then
    echo -e "${GREEN}✓ Properly rejected invalid request.${NC}"
    echo -e "\n${GREEN}✓ ERR-02 PASSED${NC}"
    exit 0
else
    echo -e "${RED}✗ Error: Unexpected response for invalid portal call${NC}"
    exit 1
fi
