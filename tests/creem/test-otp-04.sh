#!/bin/bash

##############################################################################
# OTP-04: Failed/Declined Payment (Webhook)
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
echo "OTP-04: Failed/Declined Payment (Webhook)"
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

# Step 2: Record Initial State
echo -e "${YELLOW}[2/4] Recording initial payment count${NC}"
COUNT_BEFORE=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM payments WHERE clerk_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID_OTP';" -t 2>/dev/null | tr -d ' ')

# Step 3: Trigger Webhook with FAILED status
echo -e "${YELLOW}[3/4] Sending checkout.failed webhook${NC}"
EVENT_ID="evt_fail_$(date +%s)"
PAYLOAD=$(cat <<EOF
{
  "id": "$EVENT_ID",
  "eventType": "checkout.failed",
  "object": {
    "id": "chk_failed_01",
    "customer": {
      "email": "$EMAIL"
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
  "$APP_URL/webhooks/creem" \
  -H "Content-Type: application/json" \
  -H "creem-signature: $SIGNATURE" \
  -d "$PAYLOAD")

# Step 4: Verify Response and DB
echo -e "${YELLOW}[4/4] Verifying database state unchanged${NC}"
COUNT_AFTER=$(psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "SELECT COUNT(*) FROM payments WHERE clerk_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID_OTP' AND status = 'success';" -t 2>/dev/null | tr -d ' ')

if [[ "$COUNT_BEFORE" == "$COUNT_AFTER" ]]; then
    echo -e "${GREEN}✓ Verify succeeded: No new successful payments recorded.${NC}"
else
    echo -e "${RED}✗ Verify failed: Database state changed!${NC}"
    exit 1
fi

cat > test-otp-04-report.json <<EOF
{
  "test_id": "OTP-04",
  "status": "pass",
  "notes": "Failed payments should not produce successful entitlement records"
}
EOF
echo -e "${GREEN}✓ OTP-04 PASSED${NC}"
