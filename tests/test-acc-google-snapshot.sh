#!/bin/bash

##############################################################################
# ACC-SNAPSHOT: Subscription Status Snapshot Contract Test
# 
# Purpose: Exhaustively verify the /api/v1/users/:id/subscription-status 
#          snapshot contract across all Google Play lifecycle states.
#
# Usage: ./test-acc-snapshot.sh
#
# Prerequisites:
#   - Backend running with MOCK_EXTERNAL_APIS=true
#   - globals.cfg sourced with required vars:
#     * PROVIDER, PRODUCT_ID_SUB, BRIDGE_APP_ID
#     * BRIDGE_API_KEY, BRIDGE_API_URL
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER
#   - psql installed and in PATH
#   - jq installed and in PATH
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/gpbi/globals.cfg"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Test configuration
TIMESTAMP=$(date +%s)
TEST_RUN_ID="snapshot-${TIMESTAMP}-$$"
USER_ID="test_snapshot_user_${TIMESTAMP}"

echo -e "${YELLOW}========================================${NC}"
echo "ACC-SNAPSHOT: Snapshot Contract Validation"
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

# Step 1: Basic Allowed States
echo -e "${BLUE}[1/4] Testing Basic Allowed States (is_premium=true)${NC}"

# 1.1 ACTIVE
cleanup_user
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, status, provider, current_period_end) VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID_SUB', 'active', '$PROVIDER', NOW() + INTERVAL '1 month');" > /dev/null

SNAPSHOT=$(get_snapshot)
echo "Testing: ACTIVE"
assert_field "is_premium" "$SNAPSHOT" ".is_premium" "true"
assert_field "status" "$SNAPSHOT" ".status" "active"

# 1.2 TRIAL
cleanup_user
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, status, provider, current_period_end) VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID_SUB', 'trial', '$PROVIDER', NOW() + INTERVAL '7 days');" > /dev/null

SNAPSHOT=$(get_snapshot)
echo "Testing: TRIAL"
assert_field "is_premium" "$SNAPSHOT" ".is_premium" "true"
assert_field "status" "$SNAPSHOT" ".status" "trial"

# 1.3 PAST_DUE (Grace Period)
cleanup_user
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, status, provider, current_period_end) VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID_SUB', 'past_due', '$PROVIDER', NOW() + INTERVAL '1 day');" > /dev/null

SNAPSHOT=$(get_snapshot)
echo "Testing: PAST_DUE"
assert_field "is_premium" "$SNAPSHOT" ".is_premium" "true"
assert_field "status" "$SNAPSHOT" ".status" "past_due"

# 1.4 CANCELLED (Pre-expiry)
cleanup_user
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, status, provider, current_period_end, auto_renewing) VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID_SUB', 'cancelled', '$PROVIDER', NOW() + INTERVAL '5 days', false);" > /dev/null

SNAPSHOT=$(get_snapshot)
echo "Testing: CANCELLED (Pre-expiry)"
assert_field "is_premium" "$SNAPSHOT" ".is_premium" "true"
assert_field "status" "$SNAPSHOT" ".status" "cancelled"
assert_field "auto_renewing" "$SNAPSHOT" ".auto_renewing" "false"

echo ""

# Step 2: Blocked States
echo -e "${BLUE}[2/4] Testing Blocked States (is_premium=false)${NC}"

# 2.1 PENDING
cleanup_user
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, status, provider) VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID_SUB', 'pending', '$PROVIDER');" > /dev/null

SNAPSHOT=$(get_snapshot)
echo "Testing: PENDING"
assert_field "is_premium" "$SNAPSHOT" ".is_premium" "false"
assert_field "status" "$SNAPSHOT" ".status" "pending"

# 2.2 ON_HOLD
cleanup_user
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, status, provider, payment_failure_notification) VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID_SUB', 'on_hold', '$PROVIDER', true);" > /dev/null

SNAPSHOT=$(get_snapshot)
echo "Testing: ON_HOLD"
assert_field "is_premium" "$SNAPSHOT" ".is_premium" "false"
assert_field "status" "$SNAPSHOT" ".status" "on_hold"
assert_field "payment_failure_notification" "$SNAPSHOT" ".payment_failure_notification" "true"

# 2.3 PAUSED
cleanup_user
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, status, provider) VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID_SUB', 'paused', '$PROVIDER');" > /dev/null

SNAPSHOT=$(get_snapshot)
echo "Testing: PAUSED"
assert_field "is_premium" "$SNAPSHOT" ".is_premium" "false"
assert_field "status" "$SNAPSHOT" ".status" "paused"

# 2.4 EXPIRED
cleanup_user
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, status, provider, current_period_end) VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID_SUB', 'expired', '$PROVIDER', NOW() - INTERVAL '1 day');" > /dev/null

SNAPSHOT=$(get_snapshot)
echo "Testing: EXPIRED"
assert_field "is_premium" "$SNAPSHOT" ".is_premium" "false"
assert_field "status" "$SNAPSHOT" ".status" "expired"

# 2.5 REVOKED
cleanup_user
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, status, provider, revoked_at, revocation_reason) VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID_SUB', 'revoked', '$PROVIDER', NOW(), 'REFUND');" > /dev/null

SNAPSHOT=$(get_snapshot)
echo "Testing: REVOKED"
assert_field "is_premium" "$SNAPSHOT" ".is_premium" "false"
assert_field "status" "$SNAPSHOT" ".status" "revoked"
assert_field "revocation_reason" "$SNAPSHOT" ".revocation_reason" "REFUND"
if echo "$SNAPSHOT" | jq -e '.revoked_at != null' > /dev/null; then
    echo -e "  ${GREEN}✓ revoked_at: present${NC}"
else
    echo -e "  ${RED}✗ revoked_at: missing${NC}"
    FAILED_TESTS=$((FAILED_TESTS + 1))
fi

echo ""

# Step 3: Google Lifecycle Fields
echo -e "${BLUE}[3/4] Testing Google Lifecycle Fields${NC}"

# 3.1 Price Step-Up
cleanup_user
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, status, provider, google_requires_price_step_up_consent, google_new_price_cents, google_price_step_up_consent_deadline) VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID_SUB', 'active', '$PROVIDER', true, 1299, NOW() + INTERVAL '7 days');" > /dev/null

SNAPSHOT=$(get_snapshot)
echo "Testing: Price Step-Up fields"
assert_field "google_requires_price_step_up_consent" "$SNAPSHOT" ".google_requires_price_step_up_consent" "true"
assert_field "google_new_price_cents" "$SNAPSHOT" ".google_new_price_cents" "1299"

# 3.2 Pause Scheduled
cleanup_user
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, status, provider, google_pause_scheduled_at) VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID_SUB', 'active', '$PROVIDER', NOW() + INTERVAL '10 days');" > /dev/null

SNAPSHOT=$(get_snapshot)
echo "Testing: Pause Scheduled fields"
if echo "$SNAPSHOT" | jq -e '.google_pause_scheduled_at != null' > /dev/null; then
    echo -e "  ${GREEN}✓ google_pause_scheduled_at: present${NC}"
else
    echo -e "  ${RED}✗ google_pause_scheduled_at: missing${NC}"
    FAILED_TESTS=$((FAILED_TESTS + 1))
fi

# 3.3 Deferred Until
cleanup_user
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "INSERT INTO pay.subscriptions (app_id, external_user_id, subscription_id, status, provider, google_deferred_until) VALUES ('$BRIDGE_APP_ID', '$USER_ID', '$PRODUCT_ID_SUB', 'active', '$PROVIDER', NOW() + INTERVAL '2 days');" > /dev/null

SNAPSHOT=$(get_snapshot)
echo "Testing: Deferred Until fields"
if echo "$SNAPSHOT" | jq -e '.google_deferred_until != null' > /dev/null; then
    echo -e "  ${GREEN}✓ google_deferred_until: present${NC}"
else
    echo -e "  ${RED}✗ google_deferred_until: missing${NC}"
    FAILED_TESTS=$((FAILED_TESTS + 1))
fi

echo ""

# Step 4: Multi-Subscription Ranking
echo -e "${BLUE}[4/4] Testing Multi-Subscription Ranking${NC}"
cleanup_user

# Insert one EXPIRED and one ACTIVE
# ACTIVE should be selected for the snapshot
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "INSERT INTO pay.subscriptions (id, app_id, external_user_id, subscription_id, status, provider, current_period_end, created_at) VALUES ('$(uuidgen 2>/dev/null || echo "00000000-0000-0000-0000-000000000001")', '$BRIDGE_APP_ID', '$USER_ID', 'sub_old', 'expired', '$PROVIDER', NOW() - INTERVAL '1 month', NOW() - INTERVAL '1 month');" > /dev/null

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "INSERT INTO pay.subscriptions (id, app_id, external_user_id, subscription_id, status, provider, current_period_end, created_at) VALUES ('$(uuidgen 2>/dev/null || echo "00000000-0000-0000-0000-000000000002")', '$BRIDGE_APP_ID', '$USER_ID', 'sub_new', 'active', '$PROVIDER', NOW() + INTERVAL '1 month', NOW());" > /dev/null

SNAPSHOT=$(get_snapshot)
echo "Testing: Ranking (Active vs Expired)"
assert_field "is_premium" "$SNAPSHOT" ".is_premium" "true"
assert_field "status" "$SNAPSHOT" ".status" "active"
assert_field "subscription_id" "$SNAPSHOT" ".subscription_id" "sub_new"

echo ""

# Final Summary
cleanup_user

echo -e "${YELLOW}========================================${NC}"
if [[ $FAILED_TESTS -eq 0 ]]; then
    echo -e "${GREEN}✓ ALL SNAPSHOT TESTS PASSED${NC}"
    STATUS="pass"
else
    echo -e "${RED}✗ $FAILED_TESTS SNAPSHOT TESTS FAILED${NC}"
    STATUS="fail"
fi
echo -e "${YELLOW}========================================${NC}"
if [[ $FAILED_TESTS -eq 0 ]]; then
    exit 0
else
    exit 1
fi
