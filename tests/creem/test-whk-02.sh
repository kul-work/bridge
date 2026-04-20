#!/bin/bash

##############################################################################
# WHK-02: Invalid Signature Rejection
# 
# Purpose: Verify that webhooks with invalid signatures are rejected
#          and do NOT modify the database.
#
# Usage: ./test-whk-02.sh --email "user@example.com"
#
# Prerequisites:
#   - Backend running and accessible at $APP_URL
#   - Creem Webhook Secret configured in .env
#   - psql installed and database accessible
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
echo "WHK-02: Invalid Signature Rejection"
echo -e "${YELLOW}========================================${NC}"

# Step 1: Fetch user_id
echo -e "${YELLOW}[1/4] Fetching user_id for: $EMAIL${NC}"
USER_ID=$(timeout 5 psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT clerk_id FROM users WHERE email = '$EMAIL';" -t 2>&1 || true)
USER_ID=$(echo "$USER_ID" | tr -d '[:space:]')

if [[ -z "$USER_ID" ]] || [[ "$USER_ID" == *"error"* ]] || [[ "$USER_ID" == *"ERROR"* ]]; then
    echo -e "${RED}✗ User not found or DB error${NC}"
    exit 1
fi
echo -e "${GREEN}✓ User ID: $USER_ID${NC}"

# Step 2: Record initial state
echo -e "${YELLOW}[2/4] Recording initial database state${NC}"
INITIAL_STATE=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT is_premium, premium_expires_at FROM users WHERE clerk_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')
INITIAL_SUBS_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT count(*) FROM subscriptions WHERE clerk_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')

echo "  Initial Premium state: $INITIAL_STATE"
echo "  Initial Subscriptions: $INITIAL_SUBS_COUNT"

# Step 3: Send Webhook with INVALID signature
echo -e "${YELLOW}[3/4] Sending webhook with INVALID signature${NC}"
EVENT_ID="whk-02-invalid-$(date +%s)"
PERIOD_END=$(date -u -d "+30 days" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || date -u -v+30d +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || echo "2099-12-31T23:59:59Z")

PAYLOAD=$(cat <<EOF
{
  "id": "$EVENT_ID",
  "eventType": "subscription.active",
  "object": {
    "id": "$SUBSCRIPTION_ID",
    "customer": {
      "email": "$EMAIL",
      "id": "cust_whk_02_invalid"
    },
    "metadata": {
      "user_id": "$USER_ID"
    },
    "status": "active",
    "product_id": "$PRODUCT_ID_SUB",
    "current_period_end_date": "$PERIOD_END"
  }
}
EOF
)

# Use arbitrary bad signature
BAD_SIGNATURE="BAD_SIG_1234567890abcdef"

HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  "$APP_URL/webhooks/creem" \
  -H "Content-Type: application/json" \
  -H "creem-signature: $BAD_SIGNATURE" \
  -d "$PAYLOAD")

# Standard behavior is 401 Unauthorized or 403 Forbidden
if [[ "$HTTP_CODE" == "401" || "$HTTP_CODE" == "403" || "$HTTP_CODE" == "400" ]]; then
    echo -e "${GREEN}✓ Webhook correctly rejected with HTTP $HTTP_CODE${NC}"
else
    echo -e "${RED}✗ Webhook NOT rejected adequately (HTTP $HTTP_CODE)${NC}"
    # Continue to check if DB was modified
fi

# Step 4: Verify DB unchanged
echo -e "${YELLOW}[4/4] Verifying database remains unchanged${NC}"
sleep 2 # process time

FINAL_STATE=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT is_premium, premium_expires_at FROM users WHERE clerk_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')
FINAL_SUBS_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT count(*) FROM subscriptions WHERE clerk_id = '$USER_ID';" -t 2>/dev/null | tr -d ' ')

echo "  Final Premium state: $FINAL_STATE"
echo "  Final Subscriptions: $FINAL_SUBS_COUNT"

if [[ "$FINAL_STATE" == "$INITIAL_STATE" && "$FINAL_SUBS_COUNT" == "$INITIAL_SUBS_COUNT" ]]; then
    echo -e "${GREEN}✓ Database state unchanged as expected${NC}"
    echo -e "\n${GREEN}✓ WHK-02 PASSED${NC}"
    exit 0
else
    echo -e "${RED}✗ Database state MODIFIED despite invalid signature!${NC}"
    exit 1
fi
