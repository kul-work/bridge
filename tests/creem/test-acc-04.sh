#!/bin/bash

##############################################################################
# ACC-04: Premium Access During Past Due (Grace Period)
# 
# Purpose: Verify that premium access is handled correctly for subscriptions in
#          'past_due' state (Grace Period).
#
# Usage: ./test-acc-04.sh --email "user@example.com"
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
echo "ACC-04: Premium Access During Past Due (Grace Period)"
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

# Step 2: Set up DB with past_due status
echo -e "${YELLOW}[2/4] Setting up DB for past_due state${NC}"
GRACE_START=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
FUTURE_EXPIRY=$(date -u -d "+3 days" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || date -u -v+3d +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || echo "2099-12-31T23:59:59Z")

echo "  Setting status=past_due, google_grace_period_start=$GRACE_START, expiry=$FUTURE_EXPIRY..."
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM subscriptions WHERE clerk_id = '$USER_ID';" > /dev/null
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "INSERT INTO subscriptions (clerk_id, subscription_id, status, provider, auto_renewing, current_period_end, google_grace_period_start) VALUES ('$USER_ID', '$PRODUCT_ID_SUB', 'past_due', '$PROVIDER', true, '$FUTURE_EXPIRY', '$GRACE_START');" > /dev/null
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "UPDATE users SET is_premium = true, premium_expires_at = '$FUTURE_EXPIRY' WHERE clerk_id = '$USER_ID';" > /dev/null
echo -e "${GREEN}✓ DB updated${NC}"

# Step 3: Verify access
echo -e "${YELLOW}[3/4] Verifying access during grace period${NC}"

# Check /api/v1/subscription-status
echo "  Checking /api/v1/subscription-status..."
STATUS_RESPONSE=$(curl -s -X GET "$APP_URL/api/v1/subscription-status" \
  -H "X-Test-User-ID: $USER_ID" \
  -H "X-Test-Email: $EMAIL" \
  -H "x-client-version: 99.99.0")

IS_PREMIUM=$(echo "$STATUS_RESPONSE" | grep -o '"is_premium":[^,}]*' | cut -d: -f2 | tr -d ' ' || echo "false")

if [[ "$IS_PREMIUM" == "true" ]]; then
    echo -e "  ${GREEN}✓ /subscription-status reports is_premium=true (Access Granted)${NC}"
else
    echo -e "  ${YELLOW}! /subscription-status reports is_premium=false (Access Denied)${NC}"
fi

# Check premium endpoint (GET /api/v1/story)
echo "  Checking premium endpoint (GET /api/v1/story)..."
PREMIUM_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X GET "$APP_URL/api/v1/story" \
  -H "X-Test-User-ID: $USER_ID" \
  -H "X-Test-Email: $EMAIL" \
  -H "x-client-version: 99.99.0")

if [[ "$PREMIUM_HTTP_CODE" == "200" || "$PREMIUM_HTTP_CODE" == "201" ]]; then
    echo -e "  ${GREEN}✓ Premium endpoint accessible (HTTP $PREMIUM_HTTP_CODE)${NC}"
    PAST_DUE_PASS="true"
else
    echo -e "  ${RED}✗ Premium endpoint denied (HTTP $PREMIUM_HTTP_CODE)${NC}"
    PAST_DUE_PASS="false"
fi

# Step 4: Summary and Cleanup
echo -e "${YELLOW}[4/4] Summary${NC}"
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM subscriptions WHERE clerk_id = '$USER_ID';" > /dev/null
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "UPDATE users SET is_premium = false, premium_expires_at = NULL WHERE clerk_id = '$USER_ID';" > /dev/null

if [[ "$PAST_DUE_PASS" == "true" ]]; then
    echo -e "\n${GREEN}✓ ACC-04 PASSED (Access granted during grace period)${NC}"
    exit 0
else
    echo -e "\n${RED}✗ ACC-04 FAILED (Access denied during grace period)${NC}"
    exit 1
fi
