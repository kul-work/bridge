#!/bin/bash

##############################################################################
# Cleanup All ACC (Access Control) Test Data
# 
# Purpose: Remove all ACC test reports, suite summaries, and related
#          database entries created during testing.
#
# Usage: ./cleanup-all-acc.sh --email "user@example.com"
#
# What gets cleaned:
#   - ACC test reports (acc-01-report.json, acc-02-report.json, acc-03-report.json)
#   - Suite summary (acc-suite-summary.json)
#   - Test subscription records created by ACC tests
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
DB_URL="$BRIDGE_DB_URL"

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
echo "Cleanup All ACC Test Data"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: Remove ACC test report files
echo -e "${YELLOW}[1/3] Removing ACC test report files${NC}"

ACC_REPORTS=(
    "acc-01-report.json"
    "acc-02-report.json"
    "acc-03-report.json"
    "acc-suite-summary.json"
)

for report in "${ACC_REPORTS[@]}"; do
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
        
        # Delete ACC test pay.subscriptions (those with test tokens containing 'acc')
        psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND purchase_token LIKE '%acc-%';" 2>/dev/null
        echo -e "${GREEN}✓ Removed ACC test subscription records${NC}"
        
        # Delete ACC test pay.payments
        psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.payments WHERE external_user_id = '$USER_ID' AND provider_transaction_id LIKE '%acc-%';" 2>/dev/null
        echo -e "${GREEN}✓ Removed ACC test payment records${NC}"
        
        # Clean up mock user B records
        psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.subscriptions WHERE external_user_id LIKE 'mock-user-b-%';" 2>/dev/null
        echo -e "${GREEN}✓ Removed mock user B test records${NC}"
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
echo -e "${GREEN}✓ ACC test cleanup complete${NC}"
echo ""

# List remaining ACC files (if any)
REMAINING=$(ls -la "$SCRIPT_DIR"/acc-*.json 2>/dev/null || echo "")
if [[ ! -z "$REMAINING" ]]; then
    echo -e "${YELLOW}Remaining ACC files:${NC}"
    echo "$REMAINING"
else
    echo -e "${GREEN}No ACC report files remaining${NC}"
fi
echo ""

exit 0
