#!/bin/bash

##############################################################################
# Cleanup All ACC (Access Control) Test Data
#
# Purpose: Remove all ACC test reports, suite summaries, and related
#          database entries created during testing.
#
# Usage: ./cleanup-all-acc.sh
#
# What gets cleaned:
#   - ACC test reports (acc-01-report.json, acc-02-report.json, acc-03-report.json)
#   - Suite summary (acc-suite-summary.json)
#   - Test subscription and payment records created by ACC tests
##############################################################################

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

DB_URL="$BRIDGE_DB_URL"
export PGPASSWORD="${DB_URL##*:}"
export PGPASSWORD="${PGPASSWORD%%@*}"

while [[ $# -gt 0 ]]; do
    case $1 in
        *)
            echo "Unknown option: $1"
            echo "Usage: ./cleanup-all-acc.sh"
            exit 1
            ;;
    esac
done

echo -e "${YELLOW}========================================${NC}"
echo "Cleanup All ACC Test Data"
echo -e "${YELLOW}========================================${NC}"
echo ""

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
        echo -e "${GREEN}Removed: $report${NC}"
    else
        echo -e "${BLUE}Skipped (not found): $report${NC}"
    fi
done
echo ""

echo -e "${YELLOW}[2/3] Cleaning up generated ACC database records${NC}"

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.subscriptions WHERE external_user_id LIKE 'test_acc_%' OR purchase_token LIKE 'test-acc-%' OR external_user_id LIKE 'mock-user-b-%';" \
  2>/dev/null || true
echo -e "${GREEN}Removed ACC test subscription records${NC}"

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
  -c "DELETE FROM pay.payments WHERE external_user_id LIKE 'test_acc_%' OR provider_transaction_id LIKE 'test-acc-%';" \
  2>/dev/null || true
echo -e "${GREEN}Removed ACC test payment records${NC}"
echo ""

echo -e "${YELLOW}[3/3] Cleanup Summary${NC}"
echo -e "${GREEN}ACC test cleanup complete${NC}"
echo ""

if compgen -G "$SCRIPT_DIR/acc-*.json" > /dev/null; then
    echo -e "${YELLOW}Remaining ACC files:${NC}"
    ls -la "$SCRIPT_DIR"/acc-*.json
else
    echo -e "${GREEN}No ACC report files remaining${NC}"
fi
echo ""

exit 0
