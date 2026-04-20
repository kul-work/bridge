#!/bin/bash

##############################################################################
# SUB-02: Subscription Renewal (Webhook)
# 
# Purpose: Verify that a subsequent Creem subscription.active webhook 
#          properly updates the current_period_end in the database (Renewal).
#
# Usage: ./test-sub-02.sh --email "user@example.com"
#
# Prerequisites:
#   - Backend running and accessible at $APP_URL
#   - Existing active subscription for the user (run SUB-01 first)
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
NC='\033[0m'

# Defaults
DB_URL="$DATABASE_URL"
EMAIL=""

# Database password
export PGPASSWORD="${DATABASE_PASSWORD:-}"
if [[ -z "$PGPASSWORD" ]]; then
    # Fallback to extraction from URL if not set explicitly
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
echo "SUB-02: Subscription Renewal (Webhook)"
echo -e "${YELLOW}========================================${NC}"

# Step 1: Ensure existing subscription exists
echo -e "${YELLOW}[1/4] Checking for existing active subscription${NC}"
USER_ID=$(timeout 5 psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT clerk_id FROM users WHERE email = '$EMAIL';" -t 2>&1 || true)
USER_ID=$(echo "$USER_ID" | tr -d '[:space:]')

if [[ -z "$USER_ID" ]] || [[ "$USER_ID" == *"error"* ]] || [[ "$USER_ID" == *"ERROR"* ]]; then
    echo -e "${RED}✗ User not found or DB error${NC}"
    echo "$USER_ID"
    exit 1
fi
echo -e "${GREEN}✓ User ID: $USER_ID${NC}"

SUB_EXISTS=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM subscriptions WHERE clerk_id = '$USER_ID' AND subscription_id = '$SUBSCRIPTION_ID' AND status = 'active';" -t 2>/dev/null | tr -d ' ')


if [[ "$SUB_EXISTS" == "0" ]]; then
    echo -e "${YELLOW}No active sub found. Running SUB-01 first...${NC}"
    ./test-sub-01.sh --email "$EMAIL"
fi

# Get current expiry to compare later
OLD_EXPIRY=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT current_period_end FROM subscriptions WHERE clerk_id = '$USER_ID' AND subscription_id = '$SUBSCRIPTION_ID';" -t 2>/dev/null | tr -d ' ')

echo -e "${GREEN}✓ Current Expiry: $OLD_EXPIRY${NC}"

# Step 2: Trigger Renewal Webhook
echo -e "${YELLOW}[2/4] Sending renewal subscription.active webhook${NC}"
EVENT_ID="evt_sub_02_$(date +%s)"
# Set new expiry 60 days out (renewal)
NEW_EXPIRY=$(date -u -d "+60 days" +"%Y-%m-%dT%H:%M:%SZ")

PAYLOAD=$(cat <<EOF
{
  "id": "$EVENT_ID",
  "eventType": "subscription.active",
  "object": {
    "id": "$SUBSCRIPTION_ID",

    "customer": {
      "email": "$EMAIL"
    },
    "metadata": {
      "user_id": "$USER_ID"
    },
    "status": "active",
    "product_id": "$PRODUCT_ID_SUB",
    "current_period_end_date": "$NEW_EXPIRY",
    "last_transaction": {
      "amount": 2999
    }
  }
}
EOF
)

SIGNATURE=$(echo -n "$PAYLOAD" | openssl dgst -sha256 -hmac "$CREEM_WEBHOOK_SECRET" | sed 's/^.* //')

HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  "$APP_URL/webhooks/creem" \
  -H "Content-Type: application/json" \
  -H "creem-signature: $SIGNATURE" \
  -d "$PAYLOAD")

if [[ "$HTTP_CODE" == "200" || "$HTTP_CODE" == "204" ]]; then
    echo -e "${GREEN}✓ Webhook accepted (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${RED}✗ Webhook failed (HTTP $HTTP_CODE)${NC}"
    exit 1
fi

# Step 3: Verify DB update
echo -e "${YELLOW}[3/4] Verifying expiry date updated${NC}"
sleep 2
UPDATED_EXPIRY=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT current_period_end FROM subscriptions WHERE clerk_id = '$USER_ID' AND subscription_id = '$SUBSCRIPTION_ID';" -t 2>/dev/null | tr -d ' ')


if [[ "$UPDATED_EXPIRY" != "$OLD_EXPIRY" ]]; then
    echo -e "${GREEN}✓ Expiry updated to: $UPDATED_EXPIRY${NC}"
else
    echo -e "${RED}✗ Expiry date did not change!${NC}"
    exit 1
fi

# Step 4: Report
cat > test-sub-02-report.json <<EOF
{
  "test_id": "SUB-02",
  "status": "pass",
  "old_expiry": "$OLD_EXPIRY",
  "new_expiry": "$UPDATED_EXPIRY"
}
EOF
echo -e "${GREEN}✓ SUB-02 PASSED${NC}"
