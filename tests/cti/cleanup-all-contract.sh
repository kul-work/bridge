#!/bin/bash

##############################################################################
# Cleanup All CONTRACT Test Data
##############################################################################

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${YELLOW}========================================${NC}"
echo "Cleanup All CONTRACT Test Data"
echo -e "${YELLOW}========================================${NC}"
echo ""

echo -e "${YELLOW}[1/2] Removing CONTRACT test report files${NC}"

CONTRACT_REPORTS=("contract-01-report.json" "contract-02-report.json" "contract-03-report.json"
                  "contract-04-report.json" "contract-05-report.json" "contract-06-report.json"
                  "contract-suite-summary.json")
for report in "${CONTRACT_REPORTS[@]}"; do
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
  "DELETE FROM pay.checkout_idempotency WHERE idempotency_key LIKE 'contract-%';" 2>/dev/null
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.subscriptions WHERE external_user_id LIKE 'test_contract_%' OR purchase_token LIKE 'test-contract-%';" 2>/dev/null
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.payments WHERE external_user_id LIKE 'test_contract_%' OR provider_purchase_token LIKE 'test-contract-%';" 2>/dev/null

echo -e "${GREEN}✓ CONTRACT test cleanup complete${NC}"
echo ""
exit 0