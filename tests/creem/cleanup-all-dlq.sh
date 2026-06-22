#!/bin/bash

##############################################################################
# Cleanup All DLQ (Dead-Letter & Admin Retry) Test Data — Creem
#
# Purpose: Remove all DLQ test reports, suite summaries, and related
#          database entries created during testing.
#
# Usage: ./cleanup-all-dlq.sh
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

echo -e "${YELLOW}========================================${NC}"
echo "Cleanup All DLQ Test Data (Creem)"
echo -e "${YELLOW}========================================${NC}"
echo ""

# Step 1: Remove DLQ test report files
echo -e "${YELLOW}[1/3] Removing DLQ test report files${NC}"

DLQ_REPORTS=(
    "dlq-01-report.json"
    "dlq-02-report.json"
    "dlq-suite-summary.json"
)

for report in "${DLQ_REPORTS[@]}"; do
    if [[ -f "$SCRIPT_DIR/$report" ]]; then
        rm -f "$SCRIPT_DIR/$report"
        echo -e "${GREEN}✓ Removed: $report${NC}"
    else
        echo -e "${BLUE}  Skipped (not found): $report${NC}"
    fi
done
echo ""

# Step 2: Clean up test database records
echo -e "${YELLOW}[2/3] Cleaning up database records${NC}"

# Delete DLQ test webhook_delivery records
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.webhook_delivery WHERE app_id = '$BRIDGE_APP_ID' AND webhook_provider_id IN (
     SELECT id FROM pay.webhook_provider WHERE provider_webhook_id LIKE 'dlq-%'
   );" 2>/dev/null
echo -e "${GREEN}✓ Removed DLQ test webhook_delivery records${NC}"

# Delete DLQ test webhook_provider records
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.webhook_provider WHERE provider_webhook_id LIKE 'dlq-%';" 2>/dev/null
echo -e "${GREEN}✓ Removed DLQ test webhook_provider records${NC}"

# Delete DLQ test subscription records
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.subscriptions WHERE purchase_token LIKE 'test-dlq-%' OR external_user_id LIKE 'test_dlq_%';" 2>/dev/null
echo -e "${GREEN}✓ Removed DLQ test subscription records${NC}"

# Delete DLQ test payment records
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.payments WHERE external_user_id LIKE 'test_dlq_%';" 2>/dev/null
echo -e "${GREEN}✓ Removed DLQ test payment records${NC}"
echo ""

# Step 3: Summary
echo -e "${YELLOW}[3/3] Cleanup Summary${NC}"
echo ""
echo -e "${GREEN}✓ DLQ test cleanup complete${NC}"
echo ""

# List remaining DLQ files (if any)
REMAINING=$(ls -la "$SCRIPT_DIR"/dlq-*.json 2>/dev/null || echo "")
if [[ -n "$REMAINING" ]]; then
    echo -e "${YELLOW}Remaining DLQ files:${NC}"
    echo "$REMAINING"
else
    echo -e "${GREEN}No DLQ report files remaining${NC}"
fi
echo ""

exit 0