#!/bin/bash

##############################################################################
# NET-01: Webhook Retry & Backoff
# 
# Purpose: Verify that the backend handles retried webhooks correctly.
#          Simulates an initial failure (e.g., bad signature or network error)
#          followed by a successful retry.
#
# Usage: ./test-net-01.sh --email "user@example.com"
#
# Prerequisites:
#   - Backend running and accessible at $APP_URL
#   - Creem Webhook Secret configured in .env
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
EMAIL=""

# Database password
export PGPASSWORD="${DATABASE_PASSWORD:-}"
if [[ -z "$PGPASSWORD" ]]; then
    export PGPASSWORD="${DATABASE_URL##*:}"
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
echo "NET-01: Webhook Retry & Backoff"
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

# Step 2: Cleanup
echo -e "${YELLOW}[2/4] Cleaning initial state${NC}"
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM subscriptions WHERE clerk_id = '$USER_ID';" > /dev/null
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM webhooks WHERE provider = 'creem' AND provider_webhook_id LIKE 'net-01%';" 2>/dev/null || true
echo -e "${GREEN}✓ Cleaned${NC}"

# Step 3: Send Webhook that "FAILS" (Bad Signature)
echo -e "${YELLOW}[3/4] Sending webhook with INVALID signature (Simulating failure)${NC}"
EVENT_ID="net-01-$(date +%s)"
PERIOD_END=$(date -u -d "+30 days" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || date -u -v+30d +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || echo "2099-12-31T23:59:59Z")

PAYLOAD=$(cat <<EOF
{
  "id": "$EVENT_ID",
  "eventType": "subscription.active",
  "object": {
    "id": "$SUBSCRIPTION_ID",
    "customer": {
      "email": "$EMAIL",
      "id": "cust_net_01"
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

# Bad signature
HTTP_CODE_1=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  "$APP_URL/webhooks/creem" \
  -H "Content-Type: application/json" \
  -H "creem-signature: BAD_SIG" \
  -d "$PAYLOAD")

echo "  Initial attempt (bad sig) response: HTTP $HTTP_CODE_1"

# Step 4: Send Webhook and Succeed (Retry)
echo -e "${YELLOW}[4/4] Sending same webhook with VALID signature (Simulating retry success)${NC}"

SIGNATURE=$(echo -n "$PAYLOAD" | openssl dgst -sha256 -hmac "$CREEM_WEBHOOK_SECRET" | sed 's/^.* //')

HTTP_CODE_2=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  "$APP_URL/webhooks/creem" \
  -H "Content-Type: application/json" \
  -H "creem-signature: $SIGNATURE" \
  -d "$PAYLOAD")

echo "  Retry attempt (valid sig) response: HTTP $HTTP_CODE_2"

# Verification
sleep 2 # process time
SUBS_STATUS=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT status FROM subscriptions WHERE clerk_id = '$USER_ID' AND status = 'active';" -t 2>/dev/null | tr -d ' ')

if [[ ("$HTTP_CODE_1" == "401" || "$HTTP_CODE_1" == "400" || "$HTTP_CODE_1" == "403") && ("$HTTP_CODE_2" == "200" || "$HTTP_CODE_2" == "204") && "$SUBS_STATUS" == "active" ]]; then
    echo -e "${GREEN}✓ Webhook retry handled correctly. Entitlement granted.${NC}"
    echo -e "\n${GREEN}✓ NET-01 PASSED${NC}"
    exit 0
else
    echo -e "${RED}✗ Webhook retry failed or logic incorrect${NC}"
    echo "  Status: $SUBS_STATUS"
    exit 1
fi
