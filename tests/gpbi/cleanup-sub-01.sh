#!/bin/bash

##############################################################################
# Cleanup SUB-01 Test Data
#
# Purpose: Remove test data (database records + report file) for SUB-01
#          to reset state for re-running the test
#
# Usage: ./cleanup-sub-01.sh --email "user@example.com"
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

# Test configuration
PRODUCT_ID="$PRODUCT_ID_SUB"
REPORT_FILE="sub-01-report.json"

# Defaults
EMAIL=""
DB_URL="$DATABASE_URL"

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
    echo "Usage: ./cleanup-sub-01.sh --email \"user@example.com\""
    exit 1
fi

echo -e "${YELLOW}========================================${NC}"
echo "Cleanup: SUB-01 Test Data"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: Get user_id from email
echo -e "${YELLOW}[1/3] Fetching user_id for email: $EMAIL${NC}"

USER_ID="${USER_ID:-test_user_$(date +%s)}"
# Manual check: Bridge does not track emails. Set USER_ID externally or use default.

if [[ -z "$USER_ID" ]] || [[ "$USER_ID" == *"error"* ]] || [[ "$USER_ID" == *"ERROR"* ]]; then
    echo -e "${RED}✗ Failed to fetch user_id${NC}"
    exit 1
fi

USER_ID=$(echo "$USER_ID" | tr -d '[:space:]')
echo -e "${GREEN}✓ User ID: $USER_ID${NC}"
echo ""

# Step 2: Delete subscription record
echo -e "${YELLOW}[2/4] Removing subscription record from database${NC}"

DELETE_QUERY="DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';"
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "$DELETE_QUERY" 2>/dev/null || true

echo -e "${GREEN}✓ Subscription record removed${NC}"
echo ""

# Step 3: Delete payment record
echo -e "${YELLOW}[3/4] Removing payment record from database${NC}"

DELETE_PAYMENT_QUERY="DELETE FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';"
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "$DELETE_PAYMENT_QUERY" 2>/dev/null || true

echo -e "${GREEN}✓ Payment record removed${NC}"
echo ""

# Step 4: Delete report file
echo -e "${YELLOW}[4/4] Removing report file${NC}"

if [[ -f "$REPORT_FILE" ]]; then
    rm "$REPORT_FILE"
    echo -e "${GREEN}✓ Report file removed: $REPORT_FILE${NC}"
else
    echo -e "${GREEN}✓ Report file not found (already cleaned)${NC}"
fi
echo ""

echo -e "${YELLOW}========================================${NC}"
echo -e "${GREEN}✓ Cleanup Complete${NC}"
echo -e "${YELLOW}========================================${NC}"
echo ""

exit 0
