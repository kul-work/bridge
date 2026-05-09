#!/bin/bash

##############################################################################
# NET-03: Creem Bridge-to-App Delivery Verification
#
# Purpose: Verify that webhooks are successfully forwarded from Bridge to the
#          downstream application callback URL for Creem events.
#
# Usage: ./test-net-03.sh --email "user@example.com"
#
# Prerequisites:
#   - Bridge Backend running with MOCK_EXTERNAL_APIS=true
#   - HiHa Backend (or mock) running at the configured callback URL
#   - globals.cfg sourced with required vars
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Defaults
EMAIL="${1:-$EMAIL}"
# If --email flag is used
if [[ "${1:-}" == "--email" ]]; then
    EMAIL="$2"
fi

USER_ID="creem_net_$(echo -n "$EMAIL" | md5sum | cut -d' ' -f1 | cut -c1-12)"
TEST_RUN_ID="net-03-$(date +%s)"
EVENT_ID="evt_net_03_$TEST_RUN_ID"
SUBSCRIPTION_ID="sub_net_03_$TEST_RUN_ID"
REPORT_FILE="net-03-report.json"

echo -e "${YELLOW}========================================${NC}"
echo "NET-03: Creem Delivery Verification"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Email: $EMAIL"
echo "User ID: $USER_ID"
echo ""

# Step 1: Trigger a Creem Webhook Event (New Subscription)
echo -e "${YELLOW}[1/3] Sending Creem webhook to generate delivery event${NC}"

PERIOD_END=$(date -u -d "+30 days" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || date -u -v+30d +"%Y-%m-%dT%H:%M:%SZ")

PAYLOAD=$(cat <<EOF
{
  "id": "$EVENT_ID",
  "eventType": "subscription.active",
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "object": {
    "id": "$SUBSCRIPTION_ID",
    "customer": {
      "email": "$EMAIL",
      "id": "cust_net_03"
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

# Use HMAC-SHA256 with Creem Webhook Secret
SIGNATURE=$(echo -n "$PAYLOAD" | openssl dgst -sha256 -hmac "$CREEM_WEBHOOK_SECRET" | sed 's/^.* //')

HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  "$BRIDGE_API_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/creem" \
  -H "Content-Type: application/json" \
  -H "creem-signature: $SIGNATURE" \
  -d "$PAYLOAD")

if [[ "$HTTP_CODE" == "200" || "$HTTP_CODE" == "201" || "$HTTP_CODE" == "204" ]]; then
    echo -e "${GREEN}✓ Webhook accepted by Bridge (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${RED}✗ Error: Bridge rejected webhook with HTTP $HTTP_CODE${NC}"
    exit 1
fi
echo ""

# Step 2: Poll pay.webhook_delivery for success
echo -e "${YELLOW}[2/3] Polling Bridge DB for successful delivery to app${NC}"
echo "      (Allowing up to 10 seconds for async delivery and retries)"

MAX_RETRIES=5
RETRY_INTERVAL=2
SUCCESS=false
DELIVERY_INFO=""

for ((i=1; i<=MAX_RETRIES; i++)); do
    echo "  Attempt $i/$MAX_RETRIES: Checking delivery status..."
    
    DELIVERY_INFO=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
      -c "SELECT wd.forwarded, wd.last_http_status, wd.last_error 
          FROM pay.webhook_delivery wd
          JOIN pay.webhook_provider wp ON wd.webhook_provider_id = wp.id
          WHERE wp.provider_webhook_id = '$EVENT_ID'
          ORDER BY wd.created_at DESC LIMIT 1;" -t | tr -d '[:space:]')

    if [[ "$DELIVERY_INFO" == "t"* ]]; then
        echo -e "${GREEN}✓ Webhook successfully delivered to app!${NC}"
        SUCCESS=true
        break
    fi
    
    if [[ "$i" -lt "$MAX_RETRIES" ]]; then
        sleep $RETRY_INTERVAL
    fi
done

echo ""

# Step 3: Final Assessment
if [[ "$SUCCESS" == "true" ]]; then
    echo -e "${GREEN}✓ Success: Delivery confirmed ($DELIVERY_INFO)${NC}"
    TEST_STATUS="pass"
else
    echo -e "${RED}✗ Failure: Webhook delivery failed or timed out.${NC}"
    echo -e "${BLUE}Last DB Status: $DELIVERY_INFO${NC}"
    echo ""
    echo -e "${YELLOW}Troubleshooting:${NC}"
    echo "1. Is the HiHa backend running at the expected callback URL?"
    echo "2. Check Bridge logs for 'Failed to forward webhook' errors."
    echo "3. Ensure the Creem provider is correctly configured for the app."
    TEST_STATUS="fail"
fi

# Generate JSON report
TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "NET-03",
  "test_name": "Creem Delivery Verification",
  "started_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "$TEST_STATUS",
  "results": {
    "event_id": "$EVENT_ID",
    "delivery_confirmed": $SUCCESS,
    "db_info": "$DELIVERY_INFO"
  }
}
EOF

if [[ "$TEST_STATUS" == "fail" ]]; then
    echo -e "${RED}✗ NET-03 Creem Test FAILED${NC}"
    exit 1
fi

echo -e "${GREEN}✓ NET-03 Creem Test PASSED${NC}"
exit 0
