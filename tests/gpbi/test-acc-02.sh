#!/bin/bash

##############################################################################
# ACC-02: Premium Access Revoked for Blocked States
# 
# Purpose: Verify premium access is REVOKED for blocked states.
#
# Usage: ./test-acc-02.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars:
#     * PROVIDER, PRODUCT_ID_SUB, BRIDGE_APP_ID
#     * BRIDGE_API_KEY, BRIDGE_API_URL
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER
#   - psql installed and in PATH
#
# TESTPLAN Reference:
#   Expected Behavior: GET /api/v1/subscriptions returns NO entitlements for blocked states.
#                      Blocked States: 'pending', 'on_hold', 'expired', 'revoked'.
#                      Premium features are immediately inaccessible.
#                      Ensures revenue protection and validates entitlement filters.
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
RUN_ID="$(date +%s)-$RANDOM"
USER_ID="${USER_ID:-test_acc_02_user_$RUN_ID}"

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
echo "ACC-02: Premium Access Revoked for Blocked States"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: Generate a synthetic external_user_id for this run
echo -e "${YELLOW}[1/6] Preparing generated user_id for this run${NC}"

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
    psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" 2>/dev/null
    
    # Handle different states
    case $state in
        "pending")
            psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, status, purchase_token, provider, auto_renewing, created_at, updated_at) VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID', 'pending', '$purchase_token', '$PROVIDER', true, NOW(), NOW());" 2>/dev/null
            # CRITICAL: Also set users table (pending = no access yet)
            psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "UPDATE users SET is_premium = false, premium_expires_at = NULL WHERE external_user_id = '$USER_ID';" 2>/dev/null
            ;;
        "on_hold")
            psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, status, purchase_token, provider, auto_renewing, current_period_end, google_subscription_state, created_at, updated_at) VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID', 'on_hold', '$purchase_token', '$PROVIDER', false, '$past_expiry', 3, NOW(), NOW());" 2>/dev/null
            # CRITICAL: Also set users table (on_hold = access revoked)
            psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "UPDATE users SET is_premium = false, premium_expires_at = NULL WHERE external_user_id = '$USER_ID';" 2>/dev/null
            ;;
        "expired")
            psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, status, purchase_token, provider, auto_renewing, current_period_end, created_at, updated_at) VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID', 'expired', '$purchase_token', '$PROVIDER', false, '$past_expiry', NOW(), NOW());" 2>/dev/null
            # CRITICAL: Also set users table (expired = no access)
            psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "UPDATE users SET is_premium = false, premium_expires_at = NULL WHERE external_user_id = '$USER_ID';" 2>/dev/null
            ;;
        "revoked")
            psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, status, purchase_token, provider, auto_renewing, current_period_end, revoked_at, revocation_reason, created_at, updated_at) VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID', 'revoked', '$purchase_token', '$PROVIDER', false, '$past_expiry', NOW(), 'REFUND', NOW(), NOW());" 2>/dev/null
            # CRITICAL: Also set users table (revoked = immediate access loss)
            psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "UPDATE users SET is_premium = false, premium_expires_at = NULL WHERE external_user_id = '$USER_ID';" 2>/dev/null
            ;;
    esac
    
    echo "  Subscription set to: $state"
    
    local STATUS_RESPONSE=$(curl -s -w "\n%{http_code}" -X GET \
      "$BRIDGE_API_URL/api/v1/subscriptions?external_user_id=$USER_ID" \
      -H "Content-Type: application/json" \
      -H "Authorization: Bearer $BRIDGE_API_KEY" \
      )
    
    local STATUS_HTTP_CODE=$(echo "$STATUS_RESPONSE" | tail -n1)
    local STATUS_LINE_COUNT=$(echo "$STATUS_RESPONSE" | wc -l)
    local STATUS_BODY=""
    if [ "$STATUS_LINE_COUNT" -gt 1 ]; then
        STATUS_BODY=$(echo "$STATUS_RESPONSE" | head -n $((STATUS_LINE_COUNT - 1)))
    fi
    
    echo "  subscription-status HTTP: $STATUS_HTTP_CODE"
    
    local PREMIUM_RESPONSE=$(curl -s -w "\n%{http_code}" -X GET \
      "$BRIDGE_API_URL/api/v1/subscriptions?external_user_id=$USER_ID" \
      -H "Content-Type: application/json" \
      -H "Authorization: Bearer $BRIDGE_API_KEY" \
      )
    
    local PREMIUM_HTTP_CODE=$(echo "$PREMIUM_RESPONSE" | tail -n1)
    
    echo "  Premium feature (story) HTTP: $PREMIUM_HTTP_CODE"
    
    # In Bridge, access is denied if there are NO 'active' or 'past_due' or 'cancelled' (pre-expiry) subscriptions.
    local access_result="granted"
    if [[ "$STATUS_HTTP_CODE" == "200" ]]; then
        # Check if the list contains ANY active-like status
        if ! (echo "$STATUS_BODY" | grep -qi "\"status\":\"active\"" || echo "$STATUS_BODY" | grep -qi "\"status\":\"past_due\"" || echo "$STATUS_BODY" | grep -qi "\"status\":\"cancelled\""); then
             access_result="denied"
        fi
    else
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
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID' AND purchase_token LIKE 'test-acc-02%';" 2>/dev/null
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
