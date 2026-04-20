#!/bin/bash

##############################################################################
# OTP-02: Sync Redirect Verification
# 
# Purpose: Verify that the backend can validate a payment synchronously
#          via query parameters on the success redirect URL.
#
# Usage: ./test-otp-02.sh --email "user@example.com"
#
# Prerequisites:
#   - Backend running and accessible at $APP_URL
#   - Sync signature verification implemented in /story handler
#   - psql installed and database accessible
#
# Note: This test simulates the Creem redirect with a valid HMAC signature.
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
echo "OTP-02: Sync Redirect Verification"
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

# Step 2: Cleanup
echo -e "${YELLOW}[2/4] Cleaning up old data${NC}"
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM subscriptions WHERE clerk_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID_OTP' AND provider = 'creem';" > /dev/null

# Step 3: Simulate Redirect URL
# Creem redirects to success_url?checkout_id=...&order_id=...&signature=...
CHECKOUT_ID="chk_sync_$(date +%s)"
ORDER_ID="ord_sync_$(date +%s)"

# Construct string for signing (params sorted alphabetically, joined by &)
SIGNING_STRING="checkout_id=$CHECKOUT_ID&customer_id=cust_creem_01&order_id=$ORDER_ID&product_id=$PRODUCT_ID_OTP"
# Note: In real Creem, signature is HMAC-SHA256 of this string using the API Key
# For testing, we assume the backend uses the same secret for verification if MOCK_EXTERNAL_APIS=true
SIGNATURE=$(echo -n "$SIGNING_STRING" | openssl dgst -sha256 -hmac "$CREEM_WEBHOOK_SECRET" | sed 's/^.* //')

echo -e "${YELLOW}[3/4] Calling success_url with signature fallback${NC}"
# We call the /story page (the success_url) with the params
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X GET \
  "$APP_URL/story?checkout_id=$CHECKOUT_ID&customer_id=cust_creem_01&order_id=$ORDER_ID&product_id=$PRODUCT_ID_OTP&signature=$SIGNATURE" \
  -H "X-Test-User-ID: $USER_ID")

if [[ "$HTTP_CODE" == "200" ]]; then
    echo -e "${GREEN}✓ Page loaded (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${RED}✗ Page failed or logic not implemented (HTTP $HTTP_CODE)${NC}"
    exit 1
fi

# Step 4: Verify subscription activation (sync path only activates; payment recording is webhook-only)
echo -e "${YELLOW}[4/4] Verifying subscription activation in database${NC}"
sleep 1
SUB_STATUS=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT status FROM subscriptions WHERE clerk_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID_OTP' AND provider = 'creem';" -t 2>/dev/null | tr -d ' ' || echo "")

if [[ "$SUB_STATUS" == "active" ]]; then
    echo -e "${GREEN}✓ Subscription activation verified!${NC}"
else
    echo -e "${RED}✗ Subscription activation failed. Record not found or status not active.${NC}"
    echo -e "${YELLOW}Note: Sync path should activate subscription for immediate UX.${NC}"
    exit 1
fi

cat > test-otp-02-report.json <<EOF
{
  "test_id": "OTP-02",
  "status": "pass",
  "user_id": "$USER_ID",
  "method": "sync_redirect"
}
EOF
echo -e "${GREEN}✓ OTP-02 PASSED${NC}"
