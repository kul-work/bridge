#!/bin/bash

##############################################################################
# Cleanup for All OTP Tests (OTP-01 to OTP-06 and OTP-RTDN-01/02)
# 
# Removes all test subscription records and report files
#
# Usage: ./cleanup-all-otp.sh
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

# Defaults (already sourced from globals.cfg)
DB_USER="$BRIDGE_DB_USER"
DB_HOST="$BRIDGE_DB_HOST"
DB_PORT="$BRIDGE_DB_PORT"
DB_NAME="$BRIDGE_DB_NAME"
PRODUCT_ID="$PRODUCT_ID_OTP"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

echo -e "${YELLOW}========================================${NC}"
echo "Cleaning up ALL OTP Tests (OTP-01 to OTP-06, OTP-RTDN-01/02)"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Remove all report files
echo -e "${YELLOW}[1/4] Removing test report files${NC}"

REMOVED_COUNT=0
for report in otp-{01..06}-report.json otp-rtdn-{01..02}-report.json otp-suite-summary.json; do
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

# Use the common OTP test user prefix shared across OTP-01..06 and RTDN cleanup.
USER_ID_PREFIX="test_otp_"

# Remove subscription database records
echo -e "${YELLOW}[2/4] Removing all subscription records from database${NC}"

# Delete all subscription records for OTP test users with the product
DB_DELETE="DELETE FROM pay.subscriptions WHERE external_user_id LIKE '${USER_ID_PREFIX}%' AND subscription_id = '$PRODUCT_ID';"

echo "Query:"
echo "  $DB_DELETE"
echo ""

RESULT=$(psql -U "$DB_USER" -h "$DB_HOST" -p "$DB_PORT" -d "$DB_NAME" -c "$DB_DELETE" 2>/dev/null || echo "")
echo -e "${GREEN}✓ Subscription records removed${NC}"
echo "$RESULT"
echo ""

# Remove payment database records
echo -e "${YELLOW}[3/4] Removing all payment records from database${NC}"

# Delete all payment records for OTP test users with the product
DB_DELETE_PAYMENT="DELETE FROM pay.payments WHERE external_user_id LIKE '${USER_ID_PREFIX}%' AND product_id = '$PRODUCT_ID';"

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

echo -e "${YELLOW}========================================${NC}"
echo -e "${GREEN}✓ All Cleanup Complete${NC}"
echo -e "${YELLOW}========================================${NC}"
echo ""

exit 0
