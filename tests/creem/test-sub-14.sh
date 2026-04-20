#!/bin/bash

##############################################################################
# SUB-14: Scheduled Cancellation Period Ends (Expiry Webhook)
# 
# Purpose: Verify that a Creem subscription.expired webhook properly 
#          terminates a subscription that was previously scheduled to cancel.
#
# Usage: ./test-sub-14.sh --email "user@example.com"
#
# Prerequisites:
#   - Backend running and accessible at $APP_URL
#   - Existing sub in scheduled_cancel state (run SUB-06 first)
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
echo "SUB-14: Scheduled Cancellation Expiry"
echo -e "${YELLOW}========================================${NC}"

# Step 1: Ensure active subscription exists with auto_renewing=false
echo -e "${YELLOW}[1/4] Checking for existing scheduled cancellation${NC}"
USER_ID=$(timeout 5 psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT clerk_id FROM users WHERE email = '$EMAIL';" -t 2>&1 || true)
USER_ID=$(echo "$USER_ID" | tr -d '[:space:]')

if [[ -z "$USER_ID" ]] || [[ "$USER_ID" == *"error"* ]] || [[ "$USER_ID" == *"ERROR"* ]]; then
    echo -e "${RED}✗ User not found or DB error${NC}"
    echo "$USER_ID"
    exit 1
fi
echo -e "${GREEN}✓ User ID: $USER_ID${NC}"

# Ensure it's active but auto_renewing is false
STATUS_CHECK=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT status, auto_renewing FROM subscriptions WHERE clerk_id = '$USER_ID' AND subscription_id = '$SUBSCRIPTION_ID';" -t 2>/dev/null || echo "")
STATUS_VAL=$(echo "$STATUS_CHECK" | awk -F '|' '{print $1}' | tr -d ' ')
AUTO_RENEW_VAL=$(echo "$STATUS_CHECK" | awk -F '|' '{print $2}' | tr -d ' ')

if [[ "$STATUS_VAL" != "active" ]]; then
    echo -e "${YELLOW}Subscription not in scheduled_cancel state. Running SUB-06 first...${NC}"
    ./test-sub-06.sh --email "$EMAIL"
else
    # Verify auto_renewing is actually false/empty (scheduled for cancellation)
    if [[ "$AUTO_RENEW_VAL" != "f" && "$AUTO_RENEW_VAL" != "false" && -n "$AUTO_RENEW_VAL" ]]; then
        echo -e "${YELLOW}Subscription not in scheduled_cancel state (auto_renewing is not false). Running SUB-06 first...${NC}"
        ./test-sub-06.sh --email "$EMAIL"
    fi
fi

# Step 2: Trigger Expired Webhook (subscription.expired)
echo -e "${YELLOW}[2/4] Sending subscription.expired webhook${NC}"
EVENT_ID="evt_sub_14_$(date +%s)"

PAYLOAD=$(cat <<EOF
{
  "id": "$EVENT_ID",
  "eventType": "subscription.expired",
  "object": {
    "id": "$SUBSCRIPTION_ID",
    "customer": {
      "email": "$EMAIL"
    },
    "metadata": {
      "user_id": "$USER_ID"
    },
    "status": "expired",
    "product_id": "$PRODUCT_ID_SUB"
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

# Step 3: Verify DB status is 'expired'
echo -e "${YELLOW}[3/4] Verifying status is 'expired'${NC}"
sleep 1
QUERY_RESULT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT status FROM subscriptions WHERE clerk_id = '$USER_ID' AND subscription_id = '$SUBSCRIPTION_ID';" -t 2>/dev/null || echo "")

STATUS=$(echo "$QUERY_RESULT" | awk -F '|' '{print $1}' | tr -d ' ')

if [[ "$STATUS" == "expired" ]]; then
    echo -e "${GREEN}✓ Verification passed: Status=$STATUS${NC}"
else
    echo -e "${RED}✗ Verification failed: Status=$STATUS (Expected: expired)${NC}"
    exit 1
fi

# Step 4: Report
cat > test-sub-14-report.json <<EOF
{
  "test_id": "SUB-14",
  "status": "pass"
}
EOF
echo -e "${GREEN}✓ SUB-14 PASSED${NC}"
