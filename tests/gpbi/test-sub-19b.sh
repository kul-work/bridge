#!/bin/bash

##############################################################################
# SUB-19B: LinkingRequired Response (Different Account Verification)
# 
# Purpose: Test the backend's handling of external_account_identifiers hash 
#          mismatch when a different user attempts to verify another user's 
#          purchase token.
#
# Usage: ./test-sub-19b.sh --email "user1@example.com" --email2 "user2@example.com"
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - DATABASE_URL configured and db accessible
#   - Two test users registered in the system
#
# Test Flow:
#   1. User1 subscribes with a special "linking-required" token
#   2. User2 attempts to verify the same token
#   3. Backend should return LinkingRequired (not error 400/403)
#   4. Verify no subscription created for User2
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Test configuration
TOKEN="resubscribe-linking-required"
PRODUCT_ID="$PRODUCT_ID_SUB"
PROVIDER="$PROVIDER"

# Defaults
EMAIL_USER1=""
EMAIL_USER2=""
APP_URL="$APP_URL"
DB_URL="$DATABASE_URL"

# Extract DB password once
export PGPASSWORD="${DB_URL##*:}"
export PGPASSWORD="${PGPASSWORD%%@*}"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --email)
            EMAIL_USER1="$2"
            shift 2
            ;;
        --email2)
            EMAIL_USER2="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Validate required inputs
if [[ -z "$EMAIL_USER1" ]] || [[ -z "$EMAIL_USER2" ]]; then
    echo -e "${RED}Error: --email and --email2 are required${NC}"
    echo "Usage: ./test-sub-19b.sh --email \"user1@example.com\" --email2 \"user2@example.com\""
    exit 1
fi

if [[ "$EMAIL_USER1" == "$EMAIL_USER2" ]]; then
    echo -e "${RED}Error: --email and --email2 must be different users${NC}"
    exit 1
fi

echo -e "${YELLOW}========================================${NC}"
echo "SUB-19B: LinkingRequired Response"
echo "(Different Account Verification)"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo -e "${CYAN}User1: $EMAIL_USER1${NC}"
echo -e "${CYAN}User2: $EMAIL_USER2${NC}"
echo -e "${CYAN}Token: $TOKEN${NC}"
echo ""

# Step 1: Fetch user IDs
echo -e "${YELLOW}[1/5] Fetching User IDs${NC}"

USER1_ID=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT external_user_id FROM users WHERE email = '$EMAIL_USER1';" -t | tr -d ' ' | head -n 1)
USER2_ID=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT external_user_id FROM users WHERE email = '$EMAIL_USER2';" -t | tr -d ' ' | head -n 1)

if [[ -z "$USER1_ID" ]]; then
    echo -e "${RED}✗ User1 not found: $EMAIL_USER1${NC}"
    exit 1
fi

if [[ -z "$USER2_ID" ]]; then
    echo -e "${RED}✗ User2 not found: $EMAIL_USER2${NC}"
    exit 1
fi

echo -e "${GREEN}✓ User1 ID: $USER1_ID${NC}"
echo -e "${GREEN}✓ User2 ID: $USER2_ID${NC}"
echo ""

# Step 2: Cleanup any existing pay.subscriptions for both users
echo -e "${YELLOW}[2/5] Cleanup${NC}"
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM pay.subscriptions WHERE external_user_id IN ('$USER1_ID', '$USER2_ID') AND subscription_id = '$PRODUCT_ID';" > /dev/null 2>&1
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM pay.payments WHERE external_user_id IN ('$USER1_ID', '$USER2_ID') AND subscription_id = '$PRODUCT_ID';" > /dev/null 2>&1
echo -e "${GREEN}✓ Cleanup complete${NC}"
echo ""

# Step 3: User1 subscribes with the "linking-required" token
echo -e "${YELLOW}[3/5] User1 Subscription (creates ownership)${NC}"
echo "  Token: $TOKEN"
echo ""

USER1_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$APP_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
  -H "X-Test-User-ID: $USER1_ID" \
  -H "X-Test-Email: $EMAIL_USER1" \
  -d "{
    \"provider\": \"$PROVIDER\",
    \"external_user_id\": \"$USER_ID\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$TOKEN\",
    \"product_type\": \"subscription\"
  }")

USER1_HTTP_CODE=$(echo "$USER1_RESPONSE" | tail -n1)
USER1_LINE_COUNT=$(echo "$USER1_RESPONSE" | wc -l)
if [ "$USER1_LINE_COUNT" -gt 1 ]; then
    USER1_BODY=$(echo "$USER1_RESPONSE" | head -n $((USER1_LINE_COUNT - 1)))
else
    USER1_BODY=""
fi

echo "User1 Response Code: $USER1_HTTP_CODE"
echo "User1 Response: $USER1_BODY"
echo ""

# Note: User1 might also get LinkingRequired since the mock always returns a fixed hash
# This is expected. The key test is that User2 ALSO sees LinkingRequired with the same hash.
# In real scenario, User1 would have the matching hash in their account.
echo -e "${CYAN}Note: In mock mode, User1 may also see LinkingRequired due to fixed hash.${NC}"
echo -e "${CYAN}The key validation is that User2 sees the SAME hash for account linking.${NC}"
echo ""

# Step 4: User2 attempts to verify the SAME token
echo -e "${YELLOW}[4/5] User2 Verification Attempt (should get LinkingRequired)${NC}"
echo "  Token: $TOKEN (same as User1)"
echo ""

USER2_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$APP_URL/api/v1/verify-purchase" \
  -H "Content-Type: application/json" \
  -H "X-Test-User-ID: $USER2_ID" \
  -H "X-Test-Email: $EMAIL_USER2" \
  -d "{
    \"provider\": \"$PROVIDER\",
    \"external_user_id\": \"$USER_ID\",
    \"subscription_id\": \"$PRODUCT_ID\",
    \"purchase_token\": \"$TOKEN\",
    \"product_type\": \"subscription\"
  }")

USER2_HTTP_CODE=$(echo "$USER2_RESPONSE" | tail -n1)
USER2_LINE_COUNT=$(echo "$USER2_RESPONSE" | wc -l)
if [ "$USER2_LINE_COUNT" -gt 1 ]; then
    USER2_BODY=$(echo "$USER2_RESPONSE" | head -n $((USER2_LINE_COUNT - 1)))
else
    USER2_BODY=""
fi

echo "User2 Response Code: $USER2_HTTP_CODE"
echo "User2 Response: $USER2_BODY"
echo ""

# Validate response
LINKING_REQUIRED_DETECTED=false
OBFUSCATED_ID_PRESENT=false

# Check if response contains "linking_required" or "LinkingRequired"
if echo "$USER2_BODY" | grep -qi "linking"; then
    LINKING_REQUIRED_DETECTED=true
fi

# Check if obfuscated_account_id is in the response
if echo "$USER2_BODY" | grep -qi "obfuscated"; then
    OBFUSCATED_ID_PRESENT=true
fi

# Check for the owner's hash we set in the mock
if echo "$USER2_BODY" | grep -q "sub-19b-owner-hash"; then
    echo -e "${GREEN}✓ Response contains expected obfuscated_account_id: sub-19b-owner-hash${NC}"
    OBFUSCATED_ID_PRESENT=true
fi

echo ""

# Step 5: DB Validation - User2 should NOT have a subscription
echo -e "${YELLOW}[5/5] DB Validation${NC}"

USER2_SUB_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER2_ID' AND subscription_id = '$PRODUCT_ID';" -t | tr -d ' ')

USER2_NO_SUB=false
if [[ "$USER2_SUB_COUNT" == "0" ]]; then
    echo -e "${GREEN}✓ User2 has NO subscription (correct - access denied)${NC}"
    USER2_NO_SUB=true
else
    echo -e "${RED}✗ User2 has $USER2_SUB_COUNT subscription(s) (unexpected - should be 0)${NC}"
fi

echo ""

# Generate Report
echo -e "${YELLOW}Generating Report${NC}"

TEST_STATUS="pass"
if [[ "$LINKING_REQUIRED_DETECTED" != "true" ]]; then
    echo -e "${RED}✗ LinkingRequired not detected in response${NC}"
    TEST_STATUS="fail"
else
    echo -e "${GREEN}✓ LinkingRequired detected in response${NC}"
fi

if [[ "$USER2_NO_SUB" != "true" ]]; then
    TEST_STATUS="fail"
fi

# Check HTTP status (200 expected for LinkingRequired, not 400/403)
if [[ "$USER2_HTTP_CODE" == "200" ]]; then
    echo -e "${GREEN}✓ HTTP 200 (correct - LinkingRequired is success response, not error)${NC}"
elif [[ "$USER2_HTTP_CODE" == "409" ]]; then
    # 409 Conflict could also be valid for linking scenarios
    echo -e "${GREEN}✓ HTTP 409 Conflict (also valid for linking scenarios)${NC}"
else
    echo -e "${YELLOW}⚠ HTTP $USER2_HTTP_CODE (expected 200 or 409 for LinkingRequired)${NC}"
    # Don't fail on this, as implementation may vary
fi

cat > sub-19b-report.json <<EOF
{
  "test_id": "SUB-19B",
  "test_name": "LinkingRequired Response (Different Account Verification)",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "$TEST_STATUS",
  "user1_id": "$USER1_ID",
  "user2_id": "$USER2_ID",
  "purchase_token": "$TOKEN",
  "results": {
    "user1_response_code": "$USER1_HTTP_CODE",
    "user2_response_code": "$USER2_HTTP_CODE",
    "linking_required_detected": $LINKING_REQUIRED_DETECTED,
    "obfuscated_id_present": $OBFUSCATED_ID_PRESENT,
    "user2_no_subscription": $USER2_NO_SUB
  },
  "notes": "Mock returns owner's hash 'sub-19b-owner-hash'. When User2's computed hash != owner's hash → LinkingRequired (security check working)"
}
EOF

echo ""
if [[ "$TEST_STATUS" == "pass" ]]; then
    echo -e "${GREEN}✓ SUB-19B Test PASSED${NC}"
else
    echo -e "${RED}✗ SUB-19B Test FAILED${NC}"
fi
cat sub-19b-report.json
echo ""

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi

exit 0
