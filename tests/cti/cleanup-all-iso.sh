#!/bin/bash

##############################################################################
# Cleanup All ISO (Cross-App Isolation) Test Data
##############################################################################

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${YELLOW}========================================${NC}"
echo "Cleanup All ISO Test Data"
echo -e "${YELLOW}========================================${NC}"
echo ""

echo -e "${YELLOW}[1/2] Removing ISO test report files${NC}"

ISO_REPORTS=("iso-01-report.json" "iso-02-report.json" "iso-03-report.json" "iso-04-report.json" "iso-05-report.json" "iso-suite-summary.json")
for report in "${ISO_REPORTS[@]}"; do
    if [[ -f "$SCRIPT_DIR/$report" ]]; then
        rm -f "$SCRIPT_DIR/$report"
        echo -e "${GREEN}✓ Removed: $report${NC}"
    else
        echo -e "${BLUE}  Skipped (not found): $report${NC}"
    fi
done
echo ""

echo -e "${YELLOW}[2/2] Cleaning up database records${NC}"

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.webhook_delivery WHERE webhook_provider_id IN (
     SELECT id FROM pay.webhook_provider WHERE provider_webhook_id LIKE 'iso-%'
   );" 2>/dev/null
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.webhook_provider WHERE provider_webhook_id LIKE 'iso-%';" 2>/dev/null
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.subscriptions WHERE purchase_token LIKE 'test-iso-%' OR external_user_id LIKE 'test_iso_%';" 2>/dev/null
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.payments WHERE provider_transaction_id LIKE 'test-iso-%' OR external_user_id LIKE 'test_iso_%';" 2>/dev/null
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.checkout_idempotency WHERE idempotency_key LIKE 'iso-%';" 2>/dev/null

echo -e "${GREEN}✓ ISO test cleanup complete${NC}"
echo ""
exit 0