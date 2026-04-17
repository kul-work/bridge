#!/bin/bash

##############################################################################
# Cleanup All NET (Network & Race Conditions) Test Data
# 
# Purpose: Remove all NET test reports, suite summaries, and related
#          database entries created during testing.
#
# Usage: ./cleanup-all-net.sh --email "user@example.com"
#
# What gets cleaned:
#   - NET test reports (net-01-report.json, net-02-report.json, etc.)
#   - Suite summary (net-suite-summary.json)
#   - Test subscription records created by NET tests
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
echo "Cleanup All NET Test Data"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: Remove NET test report files
echo -e "${YELLOW}[1/3] Removing NET test report files${NC}"

NET_REPORTS=(
    "net-01-report.json"
    "net-02-report.json"
    "net-03-report.json"
    "net-04-report.json"
    "net-suite-summary.json"
)

for report in "${NET_REPORTS[@]}"; do
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
        
        # Delete NET test pay.subscriptions (those with test tokens containing 'net')
        psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND purchase_token LIKE '%net-%';" 2>/dev/null
        echo -e "${GREEN}✓ Removed NET test subscription records${NC}"
        
        # Delete NET test pay.payments
        psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.payments WHERE external_user_id = '$USER_ID' AND provider_transaction_id LIKE '%net-%';" 2>/dev/null
        echo -e "${GREEN}✓ Removed NET test payment records${NC}"
        
        # Clean up any orphan NET test records
        psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "DELETE FROM pay.subscriptions WHERE purchase_token LIKE 'test-net-%';" 2>/dev/null
        echo -e "${GREEN}✓ Removed orphan NET test records${NC}"
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
echo -e "${GREEN}✓ NET test cleanup complete${NC}"
echo ""

# List remaining NET files (if any)
REMAINING=$(ls -la "$SCRIPT_DIR"/net-*.json 2>/dev/null || echo "")
if [[ ! -z "$REMAINING" ]]; then
    echo -e "${YELLOW}Remaining NET files:${NC}"
    echo "$REMAINING"
else
    echo -e "${GREEN}No NET report files remaining${NC}"
fi
echo ""

exit 0
