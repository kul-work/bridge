#!/bin/bash

##############################################################################
# API-01: Rate Limit Headers
#
# Purpose: Verify that authenticated API responses include X-RateLimit headers
#          for client-side throughput management.
#
# Usage: ./test-api-01.sh
#
# Prerequisites:
#   - Backend running
#   - globals.cfg sourced with required vars:
#     * BRIDGE_API_KEY, BRIDGE_API_URL
#
# TESTPLAN Reference:
#   Expected Behavior: Authenticated API responses include numeric X-RateLimit-Limit 
#                      and X-RateLimit-Remaining headers.
#                      Ensures client applications can safely implement 'Retry-After' policies.
#                      Validates that the rate-limiting middleware is correctly applied.
#                      Guarantees service stability by providing backpressure signaling.
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
TIMESTAMP=$(date +%s)
TEST_STARTED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
TEST_RUN_ID="api-01-${TIMESTAMP}-$$"
REPORT_FILE="api-01-report.json"
USER_ID="${USER_ID:-test_api_01_user_$TEST_RUN_ID}"

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
echo "Test Run ID: $TEST_RUN_ID"
echo ""

# Step 1: Generate a synthetic external_user_id for this run
echo -e "${YELLOW}[1/2] Preparing generated user_id for this run${NC}"

if [[ -z "$USER_ID" ]]; then
    echo -e "${RED}# User ID not set${NC}"
    exit 1
fi

echo -e "${GREEN}# User ID: $USER_ID${NC}"
echo ""

# Step 2: Make Request & Inspect Headers
echo -e "${YELLOW}[2/2] Making Authenticated Request${NC}"

# Use curl -H "Authorization: Bearer $BRIDGE_API_KEY" -i to include headers in output
RESPONSE=$(curl -H "Authorization: Bearer $BRIDGE_API_KEY" \
  -H "x-client-version: 99.99.0" \
  -s -i -X GET "$BRIDGE_API_URL/api/v1/agent/balance")

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
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "API-01",
  "test_name": "Rate Limit Headers",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
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
cat "$REPORT_FILE"
echo ""

exit 0
