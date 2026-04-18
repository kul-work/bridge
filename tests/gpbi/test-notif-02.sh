#!/bin/bash

##############################################################################
# NOTIF-02: Notification History
#
# Purpose: Verify that notification history can be retrieved.
#
# Usage: ./test-notif-02.sh
##############################################################################

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

RUN_ID="$(date +%s)-$RANDOM"
USER_ID="${USER_ID:-test_notif_02_user_$RUN_ID}"
EMAIL="test_$RUN_ID@example.com"
APP_URL="${BRIDGE_API_URL:-http://localhost:5555}"
DB_URL="${BRIDGE_DB_URL}"
# Extract DB password if needed
if [[ "$DB_URL" == *":"* ]]; then
    export PGPASSWORD="${DB_URL##*:}"
    export PGPASSWORD="${PGPASSWORD%%@*}"
fi

echo -e "${YELLOW}========================================${NC}"
echo "NOTIF-02: Notification History"
echo -e "${YELLOW}========================================${NC}"
echo ""

# 1. Prepare generated user_id for this run
echo -e "${YELLOW}[1/2] Preparing generated user_id for this run${NC}"
if [[ -z "$USER_ID" ]]; then echo -e "${RED}✗ User not found${NC}"; exit 1; fi
echo -e "${GREEN}✓ User ID: $USER_ID${NC}"
echo ""

# 2. Insert Dummy Notification (Direct DB Injection for reliability)
echo -e "${YELLOW}[2/3] Injecting Test Notification${NC}"
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c "INSERT INTO notifications (external_user_id, recipient_email, notification_type, status, subscription_id, provider) VALUES ('$USER_ID', '$EMAIL', 'test_event', 'sent', 'sub_123', 'google_play');" > /dev/null

# 3. Retrieve History
echo -e "${YELLOW}[3/3] Retrieving Notification History${NC}"
RESP=$(curl -s -H "Authorization: Bearer $API_KEY" -X GET "$BRIDGE_API_URL/api/v1/notifications/history"   -H "x-client-version: 99.99.0")

# Check if array is not empty and contains our test event
COUNT=$(echo "$RESP" | jq 'length')
CONTAINS_TEST=$(echo "$RESP" | jq '[.[] | select(.notification_type == "test_event")] | length')

if [[ "$COUNT" -gt 0 ]] && [[ "$CONTAINS_TEST" -gt 0 ]]; then
    echo -e "${GREEN}✓ Success: History returned $COUNT items (found test_event)${NC}"
else
    echo -e "${RED}✗ Failure: History empty or missing test_event. Count: $COUNT${NC}"
    echo "Response: $RESP"
    exit 1
fi

cat > notif-02-report.json <<EOF
{
  "test_id": "NOTIF-02",
  "test_name": "Notification History",
  "status": "pass",
  "user_id": "$USER_ID"
}
EOF
echo -e "${GREEN}✓ NOTIF-02 Test PASSED${NC}"
exit 0
