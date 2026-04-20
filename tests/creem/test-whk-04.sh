#!/bin/bash

##############################################################################
# WHK-04: Unknown Event Type
# 
# Purpose: Verify that the backend gracefully handles (ignores) webhooks
#          with unknown or future event types without erroring.
#
# Usage: ./test-whk-04.sh --email "user@example.com"
#
# Prerequisites:
#   - Backend running and accessible at $APP_URL
#   - Creem Webhook Secret configured in .env
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [ -f "$SCRIPT_DIR/.env" ]; then
    # Load variables from .env
    set -a
    source "$SCRIPT_DIR/.env"
    set +a
fi
source "$SCRIPT_DIR/globals.cfg"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
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

if [[ -z "$EMAIL" ]]; then
    echo -e "${RED}Error: --email is required${NC}"
    exit 1
fi

echo -e "${YELLOW}========================================${NC}"
echo "WHK-04: Unknown Event Type"
echo -e "${YELLOW}========================================${NC}"

# Step 1: Trigger Webhook with UNKNOWN eventType
echo -e "${YELLOW}[1/1] Sending webhook with unknown eventType (e.g., 'future.feature.enabled')${NC}"
EVENT_ID="whk-04-unknown-$(date +%s)"

PAYLOAD=$(cat <<EOF
{
  "id": "$EVENT_ID",
  "eventType": "future.feature.enabled",
  "object": {
    "id": "unknown_object_123",
    "metadata": {
       "info": "this is a test for forward compatibility"
    }
  }
}
EOF
)

SIGNATURE=$(echo -n "$PAYLOAD" | openssl dgst -sha256 -hmac "$CREEM_WEBHOOK_SECRET" | sed 's/^.* //')

HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  "$APP_URL/webhooks/creem" \
  -H "Content-Type: application/json" \
  -H "creem-signature: $SIGNATURE" \
  -d "$PAYLOAD")

if [[ "$HTTP_CODE" == "200" || "$HTTP_CODE" == "204" ]]; then
    echo -e "${GREEN}✓ Webhook accepted (HTTP $HTTP_CODE) - Forward compatibility works!${NC}"
    echo -e "\n${GREEN}✓ WHK-04 PASSED${NC}"
    exit 0
else
    echo -e "${RED}✗ Webhook FAILED (HTTP $HTTP_CODE) - Backend should not error on unknown types${NC}"
    exit 1
fi
