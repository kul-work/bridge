#!/bin/bash

##############################################################################
# WHK-04: Unknown Event Type
# 
# Purpose: Verify that the backend gracefully handles (ignores) webhooks 
#          with unknown or future event types without erroring.
#
# Usage: ./test-whk-04.sh [--email "user@example.com"] [--user-id "test_user"]
#
# Prerequisites:
#   - Backend running and accessible at $BRIDGE_API_URL (via globals.cfg)
#   - globals.cfg sourced with required vars:
#     * BRIDGE_DB_HOST, BRIDGE_DB_PORT, BRIDGE_DB_NAME, BRIDGE_DB_USER, PGPASSWORD
#     * WEBHOOK_INGRESS_TOKEN, CREEM_WEBHOOK_SECRET (for simulation)
#   - psql installed and database accessible
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
  "createdAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
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
  "$APP_URL/webhooks/$WEBHOOK_INGRESS_TOKEN/creem" \
  -H "Content-Type: application/json" \
  -H "creem-signature: $SIGNATURE" \
  -H "X-Webhook-Verification-Mode: off" \
  -d "$PAYLOAD")

if [[ "$HTTP_CODE" == "200" || "$HTTP_CODE" == "201" || "$HTTP_CODE" == "204" ]]; then
    echo -e "${GREEN}✓ Webhook accepted (HTTP $HTTP_CODE) - Forward compatibility works!${NC}"
    echo -e "\n${GREEN}✓ WHK-04 PASSED${NC}"
    exit 0
else
    echo -e "${RED}✗ Webhook FAILED (HTTP $HTTP_CODE) - Backend should not error on unknown types${NC}"
    exit 1
fi
