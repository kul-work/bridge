#!/bin/bash

##############################################################################
# ERR-01: Missing metadata.user_id
# 
# Purpose: Verify that webhooks missing metadata.user_id are handled
#          gracefully by the backend (logged but not causing crashes).
#
# Usage: ./test-err-01.sh --email "user@example.com"
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
echo "ERR-01: Missing metadata.user_id"
echo -e "${YELLOW}========================================${NC}"

# Step 1: Prepare payload WITHOUT metadata.user_id
echo -e "${YELLOW}[1/3] Preparing payload missing metadata.user_id${NC}"
EVENT_ID="err-01-missing-user-$(date +%s)"
SUB_ID="sub_err_01_$(date +%s)"

PAYLOAD=$(cat <<EOF
{
  "id": "$EVENT_ID",
  "eventType": "subscription.active",
  "object": {
    "id": "$SUB_ID",
    "customer": {
      "email": "$EMAIL",
      "id": "cust_err_01"
    },
    "metadata": {
      "some_other_key": "exists"
    },
    "status": "active",
    "product_id": "$PRODUCT_ID_SUB"
  }
}
EOF
)

SIGNATURE=$(echo -n "$PAYLOAD" | openssl dgst -sha256 -hmac "$CREEM_WEBHOOK_SECRET" | sed 's/^.* //')

# Step 2: Send Webhook
echo -e "${YELLOW}[2/3] Sending webhook (expecting 200/204 but logic-side error logging)${NC}"
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  "$APP_URL/webhooks/creem" \
  -H "Content-Type: application/json" \
  -H "creem-signature: $SIGNATURE" \
  -d "$PAYLOAD")

echo "  Response: HTTP $HTTP_CODE"

# Step 3: Verify no record created and handled gracefully
echo -e "${YELLOW}[3/3] Verifying no records created${NC}"
sleep 2 # process time
SUBS_COUNT=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT count(*) FROM subscriptions WHERE subscription_id = '$SUB_ID';" -t 2>/dev/null | tr -d ' ')

if [[ ("$HTTP_CODE" == "200" || "$HTTP_CODE" == "204") && "$SUBS_COUNT" == "0" ]]; then
    echo -e "${GREEN}✓ Webhook handled gracefully (ignored orphaned payment).${NC}"
    echo -e "\n${GREEN}✓ ERR-01 PASSED${NC}"
    exit 0
else
    echo -e "${RED}✗ Error handling logic failed${NC}"
    echo "  HTTP Code: $HTTP_CODE"
    echo "  Subs Count: $SUBS_COUNT"
    exit 1
fi
