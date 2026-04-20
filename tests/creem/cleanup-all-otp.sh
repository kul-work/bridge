#!/bin/bash

##############################################################################
# Cleanup CBI OTP Tests
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



echo -e "${YELLOW}Cleaning up CBI OTP reports and test data for $EMAIL...${NC}"

# Derive User ID from email (matching the generation logic in test scripts)
USER_ID="creem_$(echo -n "$EMAIL" | md5sum | cut -d' ' -f1 | cut -c1-12)"

if [[ -n "$USER_ID" ]]; then
    # Remove payment records for OTP
    psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
      -c "DELETE FROM pay.payments WHERE external_user_id = '$USER_ID' AND product_id = '$PRODUCT_ID_OTP';" > /dev/null
    
    # Remove webhook logs for this user/app
    psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" \
      -c "DELETE FROM pay.webhook_provider WHERE app_id = '$BRIDGE_APP_ID' AND provider = 'creem' AND (payload->'object'->'metadata'->>'user_id' = '$USER_ID' OR payload->'object'->'customer'->>'email' = '$EMAIL');" > /dev/null
      
    echo -e "${GREEN}✓ Database records removed${NC}"
fi

rm -f test-otp-*-report.json otp-suite-summary.json
echo -e "${GREEN}✓ JSON reports removed${NC}"

echo -e "${GREEN}Cleanup complete.${NC}"
