#!/bin/bash

##############################################################################
# cleanup-all-sub.sh - Clean up all subscription test data
# 
# Purpose: Remove all subscription test reports and database records for a user.
#          Used to reset test state for re-running the subscription test suite.
#
# Usage: ./cleanup-all-sub.sh [--dry-run]
#
# What it cleans:
#   - All sub-XX-report.json files
#   - All ack-XX-report.json files
#   - All suite summary files (*-suite-summary.json)
#   - Database: pay.subscriptions table records for the test user
#   - Database: pay.payments table records for the test product
#   - Database: processed_webhooks table for test webhook IDs
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
DB_USER="$BRIDGE_DB_USER"
DB_HOST="$BRIDGE_DB_HOST"
DB_PORT="$BRIDGE_DB_PORT"
DB_NAME="$BRIDGE_DB_NAME"

# Defaults
DRY_RUN=false

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --dry-run)
            DRY_RUN=true
            shift
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

echo -e "${YELLOW}========================================${NC}"
echo "Subscription Test Suite Cleanup"
echo -e "${YELLOW}========================================${NC}"
echo ""

if [[ "$DRY_RUN" == "true" ]]; then
    echo -e "${YELLOW}DRY RUN MODE - No changes will be made${NC}"
    echo ""
fi

# Step 1: Get user_id
echo -e "${YELLOW}[1/4] Fetching user_id from database${NC}"

USER_ID="${USER_ID:-test_user_$(date +%s)}"
# Manual check: Bridge does not track emails. Set USER_ID externally or use default.

if [[ -z "$USER_ID" ]] || [[ "$USER_ID" == *"error"* ]] || [[ "$USER_ID" == *"ERROR"* ]]; then
    echo -e "${RED}✗ Failed to fetch user_id - skipping database cleanup${NC}"
    USER_ID=""
else
    USER_ID=$(echo "$USER_ID" | tr -d '[:space:]')
    echo -e "${GREEN}✓ User ID: $USER_ID${NC}"
fi
echo ""

# Step 2: Clean report files
echo -e "${YELLOW}[2/4] Removing test report files${NC}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Clean SUB reports
SUB_REPORTS=$(find "$SCRIPT_DIR" -name "sub-*-report.json" -not -name "sub-pause-*-report.json" 2>/dev/null || echo "")
if [[ -n "$SUB_REPORTS" ]]; then
    echo "$SUB_REPORTS" | while read -r file; do
        if [[ -f "$file" ]]; then
            if [[ "$DRY_RUN" == "true" ]]; then
                echo "  Would remove: $file"
            else
                rm -f "$file"
                echo "  Removed: $file"
            fi
        fi
    done
else
    echo "  No SUB report files found"
fi

# Clean all suite summary files
SUITE_SUMMARIES=(
    "sub-suite-summary.json"
    "sub-core-suite-summary.json"
    "trials-suite-summary.json"
    "restore-suite-summary.json"
    "price-suite-summary.json"
)

for summary in "${SUITE_SUMMARIES[@]}"; do
    if [[ -f "$SCRIPT_DIR/$summary" ]]; then
        if [[ "$DRY_RUN" == "true" ]]; then
            echo "  Would remove: $summary"
        else
            rm -f "$SCRIPT_DIR/$summary"
            echo "  Removed: $summary"
        fi
    fi
done
echo ""

# Step 3: Clean database records
if [[ -n "$USER_ID" ]]; then
    echo -e "${YELLOW}[3/4] Cleaning database records${NC}"

    # Delete pay.subscriptions
    if [[ "$DRY_RUN" == "true" ]]; then
        COUNT=$(psql -U "$DB_USER" -h "$DB_HOST" -p $DB_PORT -d "$DB_NAME" -c "SELECT COUNT(*) FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t 2>/dev/null | tr -d ' ')
        echo "  Would delete $COUNT subscription record(s)"
    else
        psql -U "$DB_USER" -h "$DB_HOST" -p $DB_PORT -d "$DB_NAME" -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" 2>/dev/null
        echo -e "  ${GREEN}✓ Subscription records deleted${NC}"
    fi

    # Delete pay.payments
    if [[ "$DRY_RUN" == "true" ]]; then
        COUNT=$(psql -U "$DB_USER" -h "$DB_HOST" -p $DB_PORT -d "$DB_NAME" -c "SELECT COUNT(*) FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" -t 2>/dev/null | tr -d ' ')
        echo "  Would delete $COUNT payment record(s)"
    else
        psql -U "$DB_USER" -h "$DB_HOST" -p $DB_PORT -d "$DB_NAME" -c "DELETE FROM pay.payments WHERE external_user_id = '$USER_ID' AND subscription_id = '$PRODUCT_ID';" 2>/dev/null
        echo -e "  ${GREEN}✓ Payment records deleted${NC}"
    fi

    # Reset user premium status
    if [[ "$DRY_RUN" == "true" ]]; then
        echo "  Would reset user is_premium to false"
    else
        psql -U "$DB_USER" -h "$DB_HOST" -p $DB_PORT -d "$DB_NAME" -c "UPDATE users SET is_premium = false, premium_expires_at = NULL WHERE external_user_id = '$USER_ID';" 2>/dev/null
        echo -e "  ${GREEN}✓ User premium status reset${NC}"
    fi
else
    echo -e "${YELLOW}[3/4] Skipping database cleanup (no user_id)${NC}"
fi
echo ""

# Step 4: Clean processed webhooks
echo -e "${YELLOW}[4/4] Cleaning processed webhook records${NC}"

if [[ "$DRY_RUN" == "true" ]]; then
    COUNT=$(psql -U "$DB_USER" -h "$DB_HOST" -p $DB_PORT -d "$DB_NAME" -c "SELECT COUNT(*) FROM webhooks WHERE provider_webhook_id LIKE 'test-webhook-sub%' OR provider_webhook_id LIKE 'test-webhook-ack%' OR provider_webhook_id LIKE 'wh-sub%';" -t 2>/dev/null | tr -d ' ')
    echo "  Would delete $COUNT webhook record(s)"
else
    psql -U "$DB_USER" -h "$DB_HOST" -p $DB_PORT -d "$DB_NAME" -c "DELETE FROM webhooks WHERE provider_webhook_id LIKE 'test-webhook-sub%' OR provider_webhook_id LIKE 'test-webhook-ack%' OR provider_webhook_id LIKE 'wh-sub%';" 2>/dev/null
    echo -e "  ${GREEN}✓ Webhook records deleted${NC}"
fi
echo ""

echo -e "${YELLOW}========================================${NC}"
if [[ "$DRY_RUN" == "true" ]]; then
    echo -e "${YELLOW}DRY RUN COMPLETE - No changes made${NC}"
else
    echo -e "${GREEN}✓ Cleanup Complete${NC}"
fi
echo -e "${YELLOW}========================================${NC}"

exit 0
