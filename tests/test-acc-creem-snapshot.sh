#!/bin/bash

##############################################################################
# ACC-CREEM-SNAPSHOT: Creem Subscription Status Snapshot Contract Test
# 
# Purpose: Exhaustively verify the /api/v1/users/:id/subscription-status 
#          snapshot contract for Creem-specific statuses.
#
# Usage: ./test-acc-creem-snapshot.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - gpbi/globals.cfg sourced for general config (DB, etc.)
#   - creem/globals.cfg sourced for Creem-specifics
#   - psql and jq installed
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/gpbi/globals.cfg"
# Override with Creem specific configs if needed
source "$SCRIPT_DIR/creem/globals.cfg"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Test configuration
TIMESTAMP=$(date +%s)
TEST_RUN_ID="creem-snapshot-${TIMESTAMP}-$$"
USER_ID="test_creem_snapshot_user_${TIMESTAMP}"

echo -e "${YELLOW}========================================${NC}"
echo "ACC-CREEM-SNAPSHOT: Creem Snapshot Validation"
echo -e "${YELLOW}========================================${NC}"
echo "User ID: $USER_ID"
echo ""

# Helper to call snapshot endpoint
get_snapshot() {
    curl -s -X GET "$BRIDGE_API_URL/api/v1/users/$USER_ID/subscription-status" \
      -H "Authorization: Bearer $BRIDGE_API_KEY" \
      -H "Content-Type: application/json"
}

# Helper to clean up user data
cleanup_user() {
    psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
      -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" > /dev/null
}

# Track failures
FAILED_TESTS=0

assert_field() {
    local label="$1"
    local json="$2"
    local path="$3"
    local expected="$4"
    
    local actual=$(echo "$json" | jq -r "$path")
    
    if [[ "$actual" == "$expected" ]]; then
        echo -e "  ${GREEN}✓ $label: $actual${NC}"
    else
        echo -e "  ${RED}✗ $label: $actual (Expected: $expected)${NC}"
        FAILED_TESTS=$((FAILED_TESTS + 1))
    fi
}

# Step 1: Creem Allowed States
echo -e "${BLUE}[1/3] Testing Creem Allowed States (is_premium=true)${NC}"

# 1.1 ACTIVE
cleanup_user
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, status, provider, current_period_end) VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID_SUB', 'active', 'creem', NOW() + INTERVAL '1 month');" > /dev/null

SNAPSHOT=$(get_snapshot)
echo "Testing: active"
assert_field "is_premium" "$SNAPSHOT" ".is_premium" "true"
assert_field "status" "$SNAPSHOT" ".status" "active"

# 1.2 TRIAL
cleanup_user
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, status, provider, current_period_end) VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID_SUB', 'trial', 'creem', NOW() + INTERVAL '7 days');" > /dev/null

SNAPSHOT=$(get_snapshot)
echo "Testing: trial"
assert_field "is_premium" "$SNAPSHOT" ".is_premium" "true"
assert_field "status" "$SNAPSHOT" ".status" "trial"

# 1.3 CANCELLED (Pre-expiry)
cleanup_user
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, status, provider, current_period_end, auto_renewing) VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID_SUB', 'cancelled', 'creem', NOW() + INTERVAL '10 days', false);" > /dev/null

SNAPSHOT=$(get_snapshot)
echo "Testing: cancelled (Pre-expiry)"
assert_field "is_premium" "$SNAPSHOT" ".is_premium" "true"
assert_field "status" "$SNAPSHOT" ".status" "cancelled"

echo ""

# Step 2: Creem Blocked States
echo -e "${BLUE}[2/3] Testing Blocked States (is_premium=false)${NC}"

# 2.1 EXPIRED
cleanup_user
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, status, provider, current_period_end) VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID_SUB', 'expired', 'creem', NOW() - INTERVAL '1 day');" > /dev/null

SNAPSHOT=$(get_snapshot)
echo "Testing: expired"
assert_field "is_premium" "$SNAPSHOT" ".is_premium" "false"
assert_field "status" "$SNAPSHOT" ".status" "expired"

# 2.2 CANCELLED (Final/Expired)
cleanup_user
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, status, provider, current_period_end) VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID_SUB', 'cancelled', 'creem', NOW() - INTERVAL '1 day');" > /dev/null

SNAPSHOT=$(get_snapshot)
echo "Testing: cancelled (Post-expiry)"
assert_field "is_premium" "$SNAPSHOT" ".is_premium" "false"

echo ""

# Step 3: Ranking
echo -e "${BLUE}[3/3] Testing Ranking (Active vs Trial)${NC}"
cleanup_user

# Insert one TRIAL and one ACTIVE
# ACTIVE should be ranked higher than TRIAL in snapshot_status_rank
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "INSERT INTO pay.subscriptions (id, app_id, external_user_id, subscription_id, status, provider, current_period_end, created_at) VALUES ('$(uuidgen 2>/dev/null || echo "00000000-0000-0000-0000-000000000003")', '$BRIDGE_APP_ID', '$USER_ID', 'creem_trial', 'trial', 'creem', NOW() + INTERVAL '7 days', NOW() - INTERVAL '1 day');" > /dev/null

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "INSERT INTO pay.subscriptions (id, app_id, external_user_id, subscription_id, status, provider, current_period_end, created_at) VALUES ('$(uuidgen 2>/dev/null || echo "00000000-0000-0000-0000-000000000004")', '$BRIDGE_APP_ID', '$USER_ID', 'creem_active', 'active', 'creem', NOW() + INTERVAL '1 month', NOW());" > /dev/null

SNAPSHOT=$(get_snapshot)
echo "Testing: Ranking (Active vs Trial)"
assert_field "status" "$SNAPSHOT" ".status" "active"
assert_field "subscription_id" "$SNAPSHOT" ".subscription_id" "creem_active"

echo ""

# Final Summary
cleanup_user

echo -e "${YELLOW}========================================${NC}"
if [[ $FAILED_TESTS -eq 0 ]]; then
    echo -e "${GREEN}✓ ALL CREEM SNAPSHOT TESTS PASSED${NC}"
else
    echo -e "${RED}✗ $FAILED_TESTS CREEM SNAPSHOT TESTS FAILED${NC}"
fi
echo -e "${YELLOW}========================================${NC}"

if [[ $FAILED_TESTS -eq 0 ]]; then
    exit 0
else
    exit 1
fi
