#!/bin/bash

##############################################################################
# ACC-03: Premium Access Revoked for Blocked States
# 
# Purpose: Verify that premium access is REVOKED for subscriptions in
#          blocked states: expired, paused, revoked, cancelled (post-expiry).
#
# Usage: ./test-acc-03.sh --email "user@example.com"
#
# Prerequisites:
#   - Backend running and accessible at $APP_URL
#   - psql installed and database accessible
#   - User with given email exists in the database
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [ -f "$SCRIPT_DIR/.env" ]; then
    # Load variables from .env
    set -a
    source "$SCRIPT_DIR/.env"
    set +a
fi
source "$SCRIPT_DIR/globals.cfg"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# Defaults
DB_URL="$DATABASE_URL"
EMAIL=""

# Database password
export PGPASSWORD="${DATABASE_PASSWORD:-}"
if [[ -z "$PGPASSWORD" ]]; then
    export PGPASSWORD="${DB_URL##*:}"
    export PGPASSWORD="${PGPASSWORD%%@*}"
fi

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
echo "ACC-03: Premium Access Revoked for Blocked States"
echo -e "${YELLOW}========================================${NC}"

# Step 1: Fetch user_id
echo -e "${YELLOW}[1/4] Fetching user_id for: $EMAIL${NC}"
USER_ID=$(timeout 5 psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT clerk_id FROM users WHERE email = '$EMAIL';" -t 2>&1 || true)
USER_ID=$(echo "$USER_ID" | tr -d '[:space:]')

if [[ -z "$USER_ID" ]] || [[ "$USER_ID" == *"error"* ]] || [[ "$USER_ID" == *"ERROR"* ]]; then
    echo -e "${RED}✗ User not found or DB error${NC}"
    echo "$USER_ID"
    exit 1
fi
echo -e "${GREEN}✓ User ID: $USER_ID${NC}"

# Function to test access for blocked states
test_revoked_access() {
    local status="$1"
    local expiry_type="$2" # "past" or "future"
    echo -e "${BLUE}Testing status: $status (expiry=$expiry_type)${NC}"
    
    local expiry_date
    if [[ "$expiry_type" == "past" ]]; then
        expiry_date=$(date -u -d "-1 day" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || date -u -v-1d +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || echo "2010-01-01T00:00:00Z")
    else
        expiry_date=$(date -u -d "+30 days" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || date -u -v+30d +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || echo "2099-12-31T23:59:59Z")
    fi
    
    # Update DB
    echo "  Updating DB to status=$status, expiry=$expiry_date..."
    psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM subscriptions WHERE clerk_id = '$USER_ID';" > /dev/null
    psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "INSERT INTO subscriptions (clerk_id, subscription_id, status, provider, auto_renewing, current_period_end) VALUES ('$USER_ID', '$PRODUCT_ID_SUB', '$status', '$PROVIDER', false, '$expiry_date');" > /dev/null
    
    # Crucial: users table should reflect the status
    local is_premium="false"
    # Even if status is something else, if it's not active/trial and expiry is past, it should be false
    psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "UPDATE users SET is_premium = $is_premium, premium_expires_at = '$expiry_date' WHERE clerk_id = '$USER_ID';" > /dev/null

    # 1. Check /api/v1/subscription-status
    echo "  Checking /api/v1/subscription-status..."
    local STATUS_RESPONSE=$(curl -s -X GET "$APP_URL/api/v1/subscription-status" \
      -H "X-Test-User-ID: $USER_ID" \
      -H "X-Test-Email: $EMAIL")
    
    local IS_PREMIUM_RESP=$(echo "$STATUS_RESPONSE" | grep -o '"is_premium":[^,}]*' | cut -d: -f2 | tr -d ' ' || echo "true")
    
    if [[ "$IS_PREMIUM_RESP" == "false" ]]; then
        echo -e "  ${GREEN}✓ /subscription-status reports is_premium=false${NC}"
    else
        echo -e "  ${RED}✗ /subscription-status reports is_premium=$IS_PREMIUM_RESP${NC}"
        # We don't fail yet, let's check the premium endpoint which is the true test
    fi

    # 2. Check premium endpoint (GET /api/v1/story)
    echo "  Checking premium endpoint (GET /api/v1/story)..."
    local PREMIUM_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X GET "$APP_URL/api/v1/story" \
      -H "X-Test-User-ID: $USER_ID" \
      -H "X-Test-Email: $EMAIL")
    
    if [[ "$PREMIUM_HTTP_CODE" == "403" || "$PREMIUM_HTTP_CODE" == "401" || "$PREMIUM_HTTP_CODE" == "402" ]]; then
        echo -e "  ${GREEN}✓ Access DENIED as expected (HTTP $PREMIUM_HTTP_CODE)${NC}"
        return 0
    else
        echo -e "  ${RED}✗ Access GRANTED or unexpected error (HTTP $PREMIUM_HTTP_CODE)${NC}"
        return 1
    fi
}

# Step 2: Test 'expired' status
echo -e "${YELLOW}[2/4] Testing 'expired' status${NC}"
EXPIRED_PASS="false"
if test_revoked_access "expired" "past"; then
    EXPIRED_PASS="true"
fi

# Step 3: Test 'paused' status
echo -e "${YELLOW}[3/4] Testing 'paused' status${NC}"
PAUSED_PASS="false"
if test_revoked_access "paused" "future"; then
    PAUSED_PASS="true"
fi

# Additional: Test 'cancelled' status POST expiry
echo -e "${BLUE}Testing 'cancelled' status (POST-expiry)${NC}"
CANCELLED_POST_PASS="false"
if test_revoked_access "cancelled" "past"; then
    CANCELLED_POST_PASS="true"
fi

# Step 4: Summary and Cleanup
echo -e "${YELLOW}[4/4] Summary${NC}"
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM subscriptions WHERE clerk_id = '$USER_ID';" > /dev/null
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "UPDATE users SET is_premium = false, premium_expires_at = NULL WHERE clerk_id = '$USER_ID';" > /dev/null

echo "Results:"
echo -e "  expired status:    $([ "$EXPIRED_PASS" == "true" ] && echo -e "${GREEN}PASS${NC}" || echo -e "${RED}FAIL${NC}")"
echo -e "  paused status:     $([ "$PAUSED_PASS" == "true" ] && echo -e "${GREEN}PASS${NC}" || echo -e "${RED}FAIL${NC}")"
echo -e "  cancelled (post):  $([ "$CANCELLED_POST_PASS" == "true" ] && echo -e "${GREEN}PASS${NC}" || echo -e "${RED}FAIL${NC}")"

if [[ "$EXPIRED_PASS" == "true" && "$PAUSED_PASS" == "true" && "$CANCELLED_POST_PASS" == "true" ]]; then
    echo -e "\n${GREEN}✓ ACC-03 PASSED${NC}"
    exit 0
else
    echo -e "\n${RED}✗ ACC-03 FAILED${NC}"
    exit 1
fi
