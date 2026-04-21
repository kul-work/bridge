#!/bin/bash

##############################################################################
# Cleanup CBI Access Control Tests (ACC-01 to ACC-04)
# 
# Purpose: Removes all access control test records and reports generated 
#          by ACC-01 through ACC-04 to ensure a clean state for re-runs.
#
# Usage: ./cleanup-all-acc.sh --email "user@example.com"
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

# Defaults
EMAIL=""

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



echo -e "${YELLOW}Cleaning up CBI Access Control data for $EMAIL...${NC}"

# Derive User ID from email (matching the generation logic in test scripts)
USER_ID="creem_$(echo -n "$EMAIL" | md5sum | cut -d' ' -f1 | cut -c1-12)"

if [[ -n "$USER_ID" ]]; then
    # Remove subscription records
    psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
      -c "DELETE FROM pay.subscriptions WHERE external_user_id = '$USER_ID';" > /dev/null
    
    # Remove associated payment records
    psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
      -c "DELETE FROM pay.payments WHERE external_user_id = '$USER_ID';" > /dev/null

    echo -e "${GREEN}✓ Database records removed for User $USER_ID${NC}"
fi

# Cleanup JSON reports
rm -f test-acc-*-report.json acc-suite-summary.json
echo -e "${GREEN}✓ JSON reports removed${NC}"

echo -e "${GREEN}Cleanup complete.${NC}"
