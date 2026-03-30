#!/bin/bash

##############################################################################
# Cleanup All ERR (Error & Edge Cases) Test Data
# 
# Purpose: Remove all ERR test reports, suite summaries, and related
#          database entries created during testing.
#
# Usage: ./cleanup-all-err.sh --email "user@example.com"
#
# What gets cleaned:
#   - ERR test reports (err-01-report.json, err-02-report.json, etc.)
#   - Suite summary (err-suite-summary.json)
#   - Test subscription records created by ERR tests
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

echo -e "${YELLOW}========================================${NC}"
echo "Cleanup All ERR Test Data"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: Remove ERR test report files
echo -e "${YELLOW}[1/3] Removing ERR test report files${NC}"

ERR_REPORTS=(
    "err-01-report.json"
    "err-02-report.json"
    "err-03-report.json"
    "err-04-report.json"
    "err-05-report.json"
    "err-06-report.json"
    "err-07-report.json"
    "err-suite-summary.json"
)

for report in "${ERR_REPORTS[@]}"; do
    if [[ -f "$SCRIPT_DIR/$report" ]]; then
        rm -f "$SCRIPT_DIR/$report"
        echo -e "${GREEN}✓ Removed: $report${NC}"
    else
        echo -e "${BLUE}  Skipped (not found): $report${NC}"
    fi
done
echo ""

# Step 2: Clean up test database records (if email provided)
if [[ ! -z "$EMAIL" ]]; then
    echo -e "${YELLOW}[2/3] Cleaning up database records for: $EMAIL${NC}"
    
    # Get user_id from email
    USER_ID="${USER_ID:-test_user_$(date +%s)}"
# Manual check: Bridge does not track emails. Set USER_ID externally or use default.
    
    if [[ ! -z "$USER_ID" ]] && [[ "$USER_ID" != *"error"* ]] && [[ "$USER_ID" != *"ERROR"* ]]; then
        USER_ID=$(echo "$USER_ID" | tr -d '[:space:]')
        echo -e "${BLUE}User ID: $USER_ID${NC}"
        
        # Delete ERR test pay.subscriptions
        psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND purchase_token LIKE '%err-%';" 2>/dev/null
        echo -e "${GREEN}✓ Removed ERR test subscription records${NC}"
        
        # Delete ERR test pay.payments
        psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM pay.payments WHERE external_user_id = '$USER_ID' AND provider_transaction_id LIKE '%err-%';" 2>/dev/null
        echo -e "${GREEN}✓ Removed ERR test payment records${NC}"
        
        # Clean up orphan ERR test records
        psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM pay.subscriptions WHERE purchase_token LIKE 'test-err-%';" 2>/dev/null
        psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM pay.subscriptions WHERE purchase_token LIKE 'expired-token-%';" 2>/dev/null
        psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "DELETE FROM pay.subscriptions WHERE purchase_token LIKE 'google-api-error-%';" 2>/dev/null
        echo -e "${GREEN}✓ Removed orphan ERR test records${NC}"
    else
        echo -e "${YELLOW}⚠ Could not find user with email: $EMAIL${NC}"
    fi
else
    echo -e "${YELLOW}[2/3] Skipping database cleanup (no --email provided)${NC}"
fi
echo ""

# Step 3: Summary
echo -e "${YELLOW}[3/3] Cleanup Summary${NC}"
echo ""
echo -e "${GREEN}✓ ERR test cleanup complete${NC}"
echo ""

# List remaining ERR files (if any)
REMAINING=$(ls -la "$SCRIPT_DIR"/err-*.json 2>/dev/null || echo "")
if [[ ! -z "$REMAINING" ]]; then
    echo -e "${YELLOW}Remaining ERR files:${NC}"
    echo "$REMAINING"
else
    echo -e "${GREEN}No ERR report files remaining${NC}"
fi
echo ""

exit 0
