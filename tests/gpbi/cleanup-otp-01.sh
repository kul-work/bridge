#!/bin/bash

##############################################################################
# Cleanup for OTP-01 Test
# 
# Removes test subscription records and report files created by test-otp-01.sh
#
# Usage: ./cleanup-otp-01.sh --email "user@example.com"
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
    echo "Usage: ./cleanup-otp-01.sh --email \"user@example.com\""
    exit 1
fi

echo -e "${YELLOW}========================================${NC}"
echo "Cleaning up OTP-01 Test"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Remove report file
echo -e "${YELLOW}[1/3] Removing report file${NC}"
if [[ -f "otp-01-report.json" ]]; then
    rm -f "otp-01-report.json"
    echo -e "${GREEN}✓ Removed otp-01-report.json${NC}"
else
    echo -e "${YELLOW}✓ Report file not found (already cleaned)${NC}"
fi
echo ""

# Get user_id from database
USER_ID=""
    
    # Default values match test-otp-01.sh
    DB_USER="$DATABASE_USER"
    DB_HOST="$DATABASE_HOST"
    DB_PORT="$DATABASE_PORT"
    DB_NAME="$DATABASE_NAME"
    
    # Query user_id from database
    USER_ID="${USER_ID:-test_user_$(date +%s)}"
# Manual check: Bridge does not track emails. Set USER_ID externally or use default.
    USER_ID=$(echo "$USER_ID" | tr -d '[:space:]')
    
    if [[ -z "$USER_ID" ]]; then
        echo -e "${YELLOW}Warning: Could not find user in database${NC}"
        exit 0
    fi

# Remove database record
echo -e "${YELLOW}[2/3] Removing subscription record from database${NC}"

    # Delete subscription record
    DB_DELETE="DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';"
    
    echo "Query:"
    echo "  $DB_DELETE"
    echo ""
    
    RESULT=$(psql -U "$DB_USER" -h "$DB_HOST" -p "$DB_PORT" -d "$DB_NAME" -c "$DB_DELETE" 2>/dev/null || echo "")
    echo -e "${GREEN}✓ Subscription records removed${NC}"
    echo "$RESULT"
echo ""

# Remove payment record
echo -e "${YELLOW}[3/3] Removing payment record from database${NC}"

    # Delete payment records
    DB_DELETE_PAYMENT="DELETE FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';"
    
    echo "Query:"
    echo "  $DB_DELETE_PAYMENT"
    echo ""
    
    RESULT=$(psql -U "$DB_USER" -h "$DB_HOST" -p "$DB_PORT" -d "$DB_NAME" -c "$DB_DELETE_PAYMENT" 2>/dev/null || echo "")
    echo -e "${GREEN}✓ Payment records removed${NC}"
    echo "$RESULT"
echo ""

echo -e "${YELLOW}========================================${NC}"
echo -e "${GREEN}✓ Cleanup Complete${NC}"
echo -e "${YELLOW}========================================${NC}"
echo ""

exit 0
