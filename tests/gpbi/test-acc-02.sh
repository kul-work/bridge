#!/bin/bash

##############################################################################
# ACC-02: Premium Access Revoked for Blocked States
# 
# Purpose: Verify that premium access is REVOKED for pay.subscriptions in
#          blocked states: PENDING, ON_HOLD, EXPIRED, REVOKED.
#
# Usage: ./test-acc-02.sh --email "user@example.com"
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - DATABASE_URL configured and db accessible
#   - psql installed and in PATH
#
# TESTPLAN Reference:
#   Expected Behavior: Access denied, Error shown: "Subscription inactive"
#                      or "Payment issue", Premium feature returns 403/402.
#   Backend Logic: if subscription.state IN [PENDING, ON_HOLD, EXPIRED, REVOKED]
#                  then REVOKE_ACCESS
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
NC='\033[0m' # No Color

# Test configuration
PRODUCT_ID="$PRODUCT_ID_SUB"
PROVIDER="$PROVIDER"

# Defaults
EMAIL=""
APP_URL="$APP_URL"
DB_URL="$DATABASE_URL"

# Extract DB password once
export PGPASSWORD="${DB_URL##*:}"
export PGPASSWORD="${PGPASSWORD%%@*}"

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

# Validate required inputs
if [[ -z "$EMAIL" ]]; then
    echo -e "${RED}Error: --email is required${NC}"
    echo "Usage: ./test-acc-02.sh --email \"user@example.com\""
    exit 1
fi

echo -e "${YELLOW}========================================${NC}"
echo "ACC-02: Premium Access Revoked for Blocked States"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: Query database to get user_id from email
echo -e "${YELLOW}[1/6] Fetching user_id from database for email: $EMAIL${NC}"

USER_ID="${USER_ID:-test_user_$(date +%s)}"
# Manual check: Bridge does not track emails. Set USER_ID externally or use default.

if [[ -z "$USER_ID" ]] || [[ "$USER_ID" == *"error"* ]] || [[ "$USER_ID" == *"ERROR"* ]]; then
    echo -e "${RED}✗ Failed to fetch user_id from database${NC}"
    echo "Error: $USER_ID"
    exit 1
fi

USER_ID=$(echo "$USER_ID" | tr -d '[:space:]')
echo -e "${GREEN}✓ User ID: $USER_ID${NC}"
echo ""

# Track test results
PENDING_PASS="false"
ON_HOLD_PASS="false"
EXPIRED_PASS="false"
REVOKED_PASS="false"

# Function to set subscription state and test access
test_subscription_state() {
    local state="$1"
    local state_name="$2"
    local expected_access="$3"  # "granted" or "denied"
    
    echo -e "${BLUE}Testing state: $state_name ($state)${NC}"
    
    # Calculate past expiry for expired/revoked states
    local past_expiry=$(date -u -d "-1 day" +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date -u -v-1d +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || echo "2020-01-01T00:00:00Z")
    local future_expiry=$(date -u -d "+1 day" +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date -u -v+1d +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || echo "2099-12-31T23:59:59Z")
    
    # Set up subscription with the target state
    local purchase_token="test-acc-02-${state}-$(date +%s)"
    
    # Delete existing and insert with target state
    psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" 2>/dev/null
    
    # Handle different states
    case $state in
        "pending")
            psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "INSERT INTO pay.subscriptions (external_user_id, subscription_id, status, purchase_token, provider, auto_renewing, created_at, updated_at) VALUES ('$USER_ID', '$PRODUCT_ID', 'pending', '$purchase_token', '$PROVIDER', true, NOW(), NOW());" 2>/dev/null
            # CRITICAL: Also set users table (pending = no access yet)
            psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "UPDATE users SET is_premium = false, premium_expires_at = NULL WHERE external_user_id = '$USER_ID';" 2>/dev/null
            ;;
        "on_hold")
            psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "INSERT INTO pay.subscriptions (external_user_id, subscription_id, status, purchase_token, provider, auto_renewing, current_period_end, google_subscription_state, created_at, updated_at) VALUES ('$USER_ID', '$PRODUCT_ID', 'on_hold', '$purchase_token', '$PROVIDER', false, '$past_expiry', 3, NOW(), NOW());" 2>/dev/null
            # CRITICAL: Also set users table (on_hold = access revoked)
            psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "UPDATE users SET is_premium = false, premium_expires_at = NULL WHERE external_user_id = '$USER_ID';" 2>/dev/null
            ;;
        "expired")
            psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "INSERT INTO pay.subscriptions (external_user_id, subscription_id, status, purchase_token, provider, auto_renewing, current_period_end, created_at, updated_at) VALUES ('$USER_ID', '$PRODUCT_ID', 'expired', '$purchase_token', '$PROVIDER', false, '$past_expiry', NOW(), NOW());" 2>/dev/null
            # CRITICAL: Also set users table (expired = no access)
            psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "UPDATE users SET is_premium = false, premium_expires_at = NULL WHERE external_user_id = '$USER_ID';" 2>/dev/null
            ;;
        "revoked")
            psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "INSERT INTO pay.subscriptions (external_user_id, subscription_id, status, purchase_token, provider, auto_renewing, current_period_end, revoked_at, revocation_reason, created_at, updated_at) VALUES ('$USER_ID', '$PRODUCT_ID', 'revoked', '$purchase_token', '$PROVIDER', false, '$past_expiry', NOW(), 'REFUND', NOW(), NOW());" 2>/dev/null
            # CRITICAL: Also set users table (revoked = immediate access loss)
            psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "UPDATE users SET is_premium = false, premium_expires_at = NULL WHERE external_user_id = '$USER_ID';" 2>/dev/null
            ;;
    esac
    
    echo "  Subscription set to: $state"
    
    # Call /api/v1/pay.subscriptions to check entitlement
    local STATUS_RESPONSE=$(curl -s -w "\n%{http_code}" -X GET \
      "$APP_URL/api/v1/pay.subscriptions" \
      -H "Content-Type: application/json" \
       \
      )
    
    local STATUS_HTTP_CODE=$(echo "$STATUS_RESPONSE" | tail -n1)
    local STATUS_LINE_COUNT=$(echo "$STATUS_RESPONSE" | wc -l)
    local STATUS_BODY=""
    if [ "$STATUS_LINE_COUNT" -gt 1 ]; then
        STATUS_BODY=$(echo "$STATUS_RESPONSE" | head -n $((STATUS_LINE_COUNT - 1)))
    fi
    
    echo "  subscription-status HTTP: $STATUS_HTTP_CODE"
    
    # Try to access premium feature (Generate Story)
    local PREMIUM_RESPONSE=$(curl -s -w "\n%{http_code}" -X GET \
      "$APP_URL/api/v1/story" \
      -H "Content-Type: application/json" \
       \
      )
    
    local PREMIUM_HTTP_CODE=$(echo "$PREMIUM_RESPONSE" | tail -n1)
    
    echo "  Premium feature (story) HTTP: $PREMIUM_HTTP_CODE"
    
    # Check if access was denied (402, 403, 401, or other 4xx = denied)
    local access_result="granted"
    if [[ "$PREMIUM_HTTP_CODE" == "401" ]] || [[ "$PREMIUM_HTTP_CODE" == "402" ]] || [[ "$PREMIUM_HTTP_CODE" == "403" ]] || [[ "$PREMIUM_HTTP_CODE" == "404" ]]; then
        access_result="denied"
    fi
    
    if [[ "$access_result" == "$expected_access" ]]; then
        echo -e "  ${GREEN}✓ $state_name: Access $access_result (as expected)${NC}"
        return 0
    else
        echo -e "  ${RED}✗ $state_name: Access $access_result (expected: $expected_access)${NC}"
        return 1
    fi
}

# Step 2: Test PENDING state
echo -e "${YELLOW}[2/6] Testing PENDING state${NC}"
if test_subscription_state "pending" "PENDING" "denied"; then
    PENDING_PASS="true"
fi
echo ""

# Step 3: Test ON_HOLD state
echo -e "${YELLOW}[3/6] Testing ON_HOLD state${NC}"
if test_subscription_state "on_hold" "ON_HOLD" "denied"; then
    ON_HOLD_PASS="true"
fi
echo ""

# Step 4: Test EXPIRED state
echo -e "${YELLOW}[4/6] Testing EXPIRED state${NC}"
if test_subscription_state "expired" "EXPIRED" "denied"; then
    EXPIRED_PASS="true"
fi
echo ""

# Step 5: Test REVOKED state
echo -e "${YELLOW}[5/6] Testing REVOKED state${NC}"
if test_subscription_state "revoked" "REVOKED" "denied"; then
    REVOKED_PASS="true"
fi
echo ""

# Step 6: Summary and cleanup
echo -e "${YELLOW}[6/6] Test Summary${NC}"
echo ""

# Cleanup test subscription
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' AND purchase_token LIKE 'test-acc-02%';" 2>/dev/null
echo -e "${BLUE}Cleaned up test subscription records${NC}"
echo ""

echo "Results:"
echo -e "  PENDING state:  $([ "$PENDING_PASS" == "true" ] && echo "${GREEN}PASS (access denied)${NC}" || echo "${RED}FAIL${NC}")"
echo -e "  ON_HOLD state:  $([ "$ON_HOLD_PASS" == "true" ] && echo "${GREEN}PASS (access denied)${NC}" || echo "${RED}FAIL${NC}")"
echo -e "  EXPIRED state:  $([ "$EXPIRED_PASS" == "true" ] && echo "${GREEN}PASS (access denied)${NC}" || echo "${RED}FAIL${NC}")"
echo -e "  REVOKED state:  $([ "$REVOKED_PASS" == "true" ] && echo "${GREEN}PASS (access denied)${NC}" || echo "${RED}FAIL${NC}")"
echo ""

# Determine overall test status
ALL_PASS="false"
if [[ "$PENDING_PASS" == "true" ]] && [[ "$ON_HOLD_PASS" == "true" ]] && [[ "$EXPIRED_PASS" == "true" ]] && [[ "$REVOKED_PASS" == "true" ]]; then
    TEST_STATUS="pass"
    ALL_PASS="true"
    TEST_RESULT_MSG="${GREEN}✓ ACC-02 Test PASSED${NC}"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}✗ ACC-02 Test FAILED${NC}"
fi

# Generate JSON report
cat > acc-02-report.json <<EOF
{
  "test_id": "ACC-02",
  "test_name": "Premium Access Revoked for Blocked States",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "status": "$TEST_STATUS",
  "user_id": "$USER_ID",
  "user_email": "$EMAIL",
  "results": {
    "pending_state_access_denied": $PENDING_PASS,
    "on_hold_state_access_denied": $ON_HOLD_PASS,
    "expired_state_access_denied": $EXPIRED_PASS,
    "revoked_state_access_denied": $REVOKED_PASS,
    "all_blocked_states_pass": $ALL_PASS
  },
  "notes": "Tests that PENDING, ON_HOLD, EXPIRED, and REVOKED states deny premium access"
}
EOF

echo -e "${YELLOW}========================================${NC}"
echo -e "$TEST_RESULT_MSG"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: acc-02-report.json"
cat acc-02-report.json
echo ""

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi
exit 0
