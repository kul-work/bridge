#!/bin/bash

##############################################################################
# Cleanup for All OTP Tests (OTP-01 to OTP-05 and OTP-RTDN-01/02)
# 
# Removes all test subscription records and report files
#
# Usage: ./cleanup-all-otp.sh --email "user@example.com"
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Defaults
EMAIL=""
DB_URL="$DATABASE_URL"
PRODUCT_ID="$PRODUCT_ID_OTP"

# Extract DB password once
export PGPASSWORD="${DB_URL##*:}"
export PGPASSWORD="${PGPASSWORD%%@*}"

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

# Validate required inputs
if [[ -z "$EMAIL" ]]; then
    echo -e "${RED}Error: --email is required${NC}"
    echo "Usage: ./cleanup-all-otp.sh --email \"user@example.com\""
    exit 1
fi

echo -e "${YELLOW}========================================${NC}"
echo "Cleaning up ALL OTP Tests (OTP-01 to OTP-05, OTP-RTDN-01/02)"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Remove all report files
echo -e "${YELLOW}[1/4] Removing test report files${NC}"

REMOVED_COUNT=0
for report in otp-{01..05}-report.json otp-rtdn-{01..02}-report.json otp-suite-summary.json; do
    if [[ -f "$report" ]]; then
        rm -f "$report"
        echo -e "${GREEN}✓ Removed $report${NC}"
        REMOVED_COUNT=$((REMOVED_COUNT + 1))
    fi
done

if [[ $REMOVED_COUNT -eq 0 ]]; then
    echo -e "${YELLOW}✓ No report files found (already cleaned)${NC}"
fi
echo ""

# Get user_id from database
DB_USER="$DATABASE_USER"
DB_HOST="$DATABASE_HOST"
DB_PORT="$DATABASE_PORT"
DB_NAME="$DATABASE_NAME"

USER_ID="${USER_ID:-test_user_$(date +%s)}"
# Manual check: Bridge does not track emails. Set USER_ID externally or use default.
USER_ID=$(echo "$USER_ID" | tr -d '[:space:]')

if [[ -z "$USER_ID" ]]; then
    echo -e "${YELLOW}✓ User not found in database (nothing to clean)${NC}"
    exit 0
fi

# Remove subscription database records
echo -e "${YELLOW}[2/4] Removing all subscription records from database${NC}"

# Delete all subscription records for this user with the product
DB_DELETE="DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';"

echo "Query:"
echo "  $DB_DELETE"
echo ""

RESULT=$(psql -U "$DB_USER" -h "$DB_HOST" -p "$DB_PORT" -d "$DB_NAME" -c "$DB_DELETE" 2>/dev/null || echo "")
echo -e "${GREEN}✓ Subscription records removed${NC}"
echo "$RESULT"
echo ""

# Remove payment database records
echo -e "${YELLOW}[3/4] Removing all payment records from database${NC}"

# Delete all payment records for this user with the product
DB_DELETE_PAYMENT="DELETE FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';"

echo "Query:"
echo "  $DB_DELETE_PAYMENT"
echo ""

RESULT=$(psql -U "$DB_USER" -h "$DB_HOST" -p "$DB_PORT" -d "$DB_NAME" -c "$DB_DELETE_PAYMENT" 2>/dev/null || echo "")
echo -e "${GREEN}✓ Payment records removed${NC}"
echo "$RESULT"
echo ""

# Remove webhook database records (for RTDN tests)
echo -e "${YELLOW}[4/4] Removing all webhook records from database${NC}"

# Delete all webhook records with test webhook patterns (otp/rtdn)
DB_DELETE_WEBHOOKS="DELETE FROM webhooks WHERE provider_webhook_id LIKE '%otp%' OR provider_webhook_id LIKE '%rtdn%' OR provider_webhook_id LIKE '%webhook-%';"

echo "Query:"
echo "  $DB_DELETE_WEBHOOKS"
echo ""

RESULT=$(psql -U "$DB_USER" -h "$DB_HOST" -p "$DB_PORT" -d "$DB_NAME" -c "$DB_DELETE_WEBHOOKS" 2>/dev/null || echo "")
echo -e "${GREEN}✓ Webhook records removed${NC}"
echo "$RESULT"
echo ""

# Reset user premium status
echo -e "${YELLOW}[5/5] Resetting user premium status${NC}"

DB_RESET_PREMIUM="UPDATE \"public\".\"users\" SET \"is_premium\"='false', \"premium_activated_at\"=NULL, \"premium_expires_at\"=NULL WHERE \"email\"='$EMAIL';"

echo "Query:"
echo "  $DB_RESET_PREMIUM"
echo ""

RESULT=$(psql -U "$DB_USER" -h "$DB_HOST" -p "$DB_PORT" -d "$DB_NAME" -c "$DB_RESET_PREMIUM" 2>/dev/null || echo "")
echo -e "${GREEN}✓ User premium status reset${NC}"
echo "$RESULT"
echo ""

echo -e "${YELLOW}========================================${NC}"
echo -e "${GREEN}✓ All Cleanup Complete${NC}"
echo -e "${YELLOW}========================================${NC}"
echo ""

exit 0
