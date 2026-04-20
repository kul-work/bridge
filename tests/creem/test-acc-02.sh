#!/bin/bash

##############################################################################
# ACC-02: Premium Access Retained During Scheduled Cancel
# 
# Purpose: Verify that premium access is RETAINED for subscriptions in
#          'cancelled' state BEFORE the current_period_end date.
#
# Usage: ./test-acc-02.sh --email "user@example.com"
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
echo "ACC-02: Premium Access Retained During Scheduled Cancel"
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

# Step 2: Set up DB with cancelled status but future expiry
echo -e "${YELLOW}[2/4] Setting up DB for scheduled cancellation${NC}"
FUTURE_EXPIRY=$(date -u -d "+15 days" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || date -u -v+15d +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || echo "2099-12-31T23:59:59Z")
CANCELLATION_TIME=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

echo "  Setting status=cancelled, auto_renewing=false, current_period_end=$FUTURE_EXPIRY..."
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM subscriptions WHERE clerk_id = '$USER_ID';" > /dev/null
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "INSERT INTO subscriptions (clerk_id, subscription_id, status, provider, auto_renewing, current_period_end, cancellation_initiated_at) VALUES ('$USER_ID', '$PRODUCT_ID_SUB', 'cancelled', '$PROVIDER', false, '$FUTURE_EXPIRY', '$CANCELLATION_TIME');" > /dev/null
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "UPDATE users SET is_premium = true, premium_expires_at = '$FUTURE_EXPIRY' WHERE clerk_id = '$USER_ID';" > /dev/null
echo -e "${GREEN}✓ DB updated${NC}"

# Step 3: Verify access
echo -e "${YELLOW}[3/4] Verifying access${NC}"

# Check /api/v1/subscription-status
echo "  Checking /api/v1/subscription-status..."
STATUS_RESPONSE=$(curl -s -X GET "$APP_URL/api/v1/subscription-status" \
  -H "X-Test-User-ID: $USER_ID" \
  -H "X-Test-Email: $EMAIL" \
  -H "x-client-version: 99.99.0")

IS_PREMIUM=$(echo "$STATUS_RESPONSE" | grep -o '"is_premium":[^,}]*' | cut -d: -f2 | tr -d ' ' || echo "false")

if [[ "$IS_PREMIUM" == "true" ]]; then
    echo -e "  ${GREEN}✓ /subscription-status reports is_premium=true${NC}"
else
    echo -e "  ${RED}✗ /subscription-status reports is_premium=$IS_PREMIUM${NC}"
    echo "  Response: $STATUS_RESPONSE"
fi

# Check a premium endpoint (GET /api/v1/story)
echo "  Checking premium endpoint (GET /api/v1/story)..."
PREMIUM_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X GET "$APP_URL/api/v1/story" \
  -H "X-Test-User-ID: $USER_ID" \
  -H "X-Test-Email: $EMAIL" \
  -H "x-client-version: 99.99.0")

if [[ "$PREMIUM_HTTP_CODE" == "200" || "$PREMIUM_HTTP_CODE" == "201" ]]; then
    echo -e "  ${GREEN}✓ Premium endpoint accessible (HTTP $PREMIUM_HTTP_CODE)${NC}"
    CANCELLED_PRE_EXPIRY_PASS="true"
else
    echo -e "  ${RED}✗ Premium endpoint denied (HTTP $PREMIUM_HTTP_CODE)${NC}"
    CANCELLED_PRE_EXPIRY_PASS="false"
fi

# Step 4: Summary and Cleanup
echo -e "${YELLOW}[4/4] Summary${NC}"
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM subscriptions WHERE clerk_id = '$USER_ID';" > /dev/null
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "UPDATE users SET is_premium = false, premium_expires_at = NULL WHERE clerk_id = '$USER_ID';" > /dev/null

if [[ "$CANCELLED_PRE_EXPIRY_PASS" == "true" ]]; then
    echo -e "\n${GREEN}✓ ACC-02 PASSED${NC}"
    exit 0
else
    echo -e "\n${RED}✗ ACC-02 FAILED${NC}"
    exit 1
fi
