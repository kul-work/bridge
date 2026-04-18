#!/bin/bash

##############################################################################
# API-01: Rate Limit Headers Test
#
# Purpose: Verify that authenticated API responses include X-RateLimit headers.
#
# Usage: ./test-api-01.sh
#
# Prerequisites:
#   - Backend running
#   - DATABASE_URL configured
#
# Test Flow:
#   1. Make authenticated request to /api/v1/joke
#   2. Inspect headers for X-RateLimit-Limit and X-RateLimit-Remaining
#   3. Verify headers are present and numeric
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Test configuration
RUN_ID="$(date +%s)-$RANDOM"
USER_ID="${USER_ID:-test_api_01_user_$RUN_ID}"

# Defaults
APP_URL="${BRIDGE_API_URL:-http://localhost:5555}"
DB_URL="${BRIDGE_DB_URL}"

# Extract DB password once
# Extract DB password if needed
if [[ "$DB_URL" == *":"* ]]; then
    export PGPASSWORD="${DB_URL##*:}"
    export PGPASSWORD="${PGPASSWORD%%@*}"
fi

echo -e "${YELLOW}========================================${NC}"
echo "API-01: Rate Limit Headers"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: Generate a synthetic external_user_id for this run
echo -e "${YELLOW}[1/2] Preparing generated user_id for this run${NC}"

if [[ -z "$USER_ID" ]]; then
    echo -e "${RED}✗ User ID not set${NC}"
    exit 1
fi

echo -e "${GREEN}✓ User ID: $USER_ID${NC}"
echo ""

# Step 2: Make Request & Inspect Headers
echo -e "${YELLOW}[2/2] Making Authenticated Request${NC}"

# Use curl -H "Authorization: Bearer $BRIDGE_API_KEY" -i to include headers in output (use test headers for test mode)
RESPONSE=$(curl -H "Authorization: Bearer $BRIDGE_API_KEY" -s -i -X GET "$BRIDGE_API_URL/api/v1/joke" \
   \
   \
  -H "x-client-version: 99.99.0")

# Extract headers
LIMIT_HEADER=$(echo "$RESPONSE" | grep -i "X-RateLimit-Limit" | cut -d':' -f2 | tr -d ' \r')
REMAINING_HEADER=$(echo "$RESPONSE" | grep -i "X-RateLimit-Remaining" | cut -d':' -f2 | tr -d ' \r')

echo "X-RateLimit-Limit: $LIMIT_HEADER"
echo "X-RateLimit-Remaining: $REMAINING_HEADER"

# Step 3: Validate Headers
echo -e "${YELLOW}[2/2] Validating Headers${NC}"

if [[ -n "$LIMIT_HEADER" ]] && [[ "$LIMIT_HEADER" =~ ^[0-9]+$ ]]; then
    echo -e "${GREEN}✓ X-RateLimit-Limit is present and numeric: $LIMIT_HEADER${NC}"
else
    echo -e "${RED}✗ X-RateLimit-Limit missing or invalid: '$LIMIT_HEADER'${NC}"
    exit 1
fi

if [[ -n "$REMAINING_HEADER" ]] && [[ "$REMAINING_HEADER" =~ ^[0-9]+$ ]]; then
    echo -e "${GREEN}✓ X-RateLimit-Remaining is present and numeric: $REMAINING_HEADER${NC}"
else
    echo -e "${RED}✗ X-RateLimit-Remaining missing or invalid: '$REMAINING_HEADER'${NC}"
    exit 1
fi

# Determine test status
TEST_STATUS="pass"

# Step 4: Report
cat > api-01-report.json <<EOF
{
  "test_id": "API-01",
  "test_name": "Rate Limit Headers",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "results": {
    "limit_header_present": true,
    "remaining_header_present": true,
    "limit": $LIMIT_HEADER,
    "remaining": $REMAINING_HEADER
  }
}
EOF

echo -e "${GREEN}✓ API-01 Test PASSED${NC}"
cat api-01-report.json
echo ""

exit 0
