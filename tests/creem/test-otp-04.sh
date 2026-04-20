#!/bin/bash

##############################################################################
# OTP-04: Failed/Declined Payment (Webhook)
# 
# Purpose: Verify that a Creem checkout.failed webhook does not create 
#          a successful payment record.
#
# Usage: ./test-otp-04.sh --user-id "test_user"
#
# Prerequisites:
#   - Backend running and accessible at $BRIDGE_API_URL
#   - globals.cfg sourced
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

# Defaults
TIMESTAMP=$(date +%s)
EMAIL="creem_otp_user_$TIMESTAMP@example.com"
USER_ID="test_otp_user_$TIMESTAMP"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --user-id)
            USER_ID="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

echo -e "${YELLOW}========================================${NC}"
echo "OTP-04: Failed/Declined Payment (Webhook)"
echo -e "${YELLOW}========================================${NC}"

# Step 1: User Identity
echo -e "${YELLOW}[1/4] Using External User ID: $USER_ID${NC}"
echo -e "${GREEN}✓ Ready${NC}"

# Step 2: Record Initial State
echo -e "${YELLOW}[2/4] Recording initial payment count (success only)${NC}"
COUNT_BEFORE=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT COUNT(*) FROM pay.payments WHERE external_user_id = '$USER_ID' AND product_id = '$PRODUCT_ID_OTP' AND status = 'success';" -t | tr -d '[:space:]' || echo "0")

# Step 3: Trigger Webhook with FAILED status
echo -e "${YELLOW}[3/4] Sending checkout.failed webhook${NC}"
EVENT_ID="evt_fail_$(date +%s)"
PAYLOAD=$(cat <<EOF
{
  "id": "$EVENT_ID",
  "eventType": "checkout.failed",
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "object": {
    "id": "chk_failed_04_$(date +%s)",
    "customer": {
      "email": "$EMAIL",
      "id": "cust_creem_04"
    },
    "metadata": {
      "user_id": "$USER_ID"
    },
    "product_id": "$PRODUCT_ID_OTP",
    "status": "failed"
  }
}
EOF
)

SIGNATURE=$(echo -n "$PAYLOAD" | openssl dgst -sha256 -hmac "$CREEM_WEBHOOK_SECRET" | sed 's/^.* //')

HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  "$APP_URL/webhooks/$WEBHOOK_TOKEN/creem" \
  -H "Content-Type: application/json" \
  -H "creem-signature: $SIGNATURE" \
  -d "$PAYLOAD")

if [[ "$HTTP_CODE" == "200" || "$HTTP_CODE" == "201" || "$HTTP_CODE" == "204" ]]; then
    echo -e "${GREEN}✓ Webhook accepted (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${RED}✗ Webhook failed (HTTP $HTTP_CODE)${NC}"
    exit 1
fi

# Step 4: Verify Response and DB
echo -e "${YELLOW}[4/4] Verifying database state unchanged for success count${NC}"
sleep 2
COUNT_AFTER=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" \
  -c "SELECT COUNT(*) FROM pay.payments WHERE external_user_id = '$USER_ID' AND product_id = '$PRODUCT_ID_OTP' AND status = 'success';" -t | tr -d '[:space:]' || echo "0")

if [[ "$COUNT_BEFORE" == "$COUNT_AFTER" ]]; then
    echo -e "${GREEN}✓ Verify succeeded: No new successful payments recorded.${NC}"
else
    echo -e "${RED}✗ Verify failed: Database state changed! (Before: $COUNT_BEFORE, After: $COUNT_AFTER)${NC}"
    exit 1
fi

# Report
cat > test-otp-04-report.json <<EOF
{
  "test_id": "OTP-04",
  "status": "pass",
  "user_id": "$USER_ID",
  "notes": "Failed payments should not produce successful entitlement records"
}
EOF
echo -e "${GREEN}✓ OTP-04 PASSED${NC}"
