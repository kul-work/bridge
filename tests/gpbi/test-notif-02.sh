#!/bin/bash

##############################################################################
# NOTIF-02: Notification History
#
# Purpose: Verify that notification history can be retrieved.
#
# Usage: ./test-notif-02.sh --email "user@example.com"
##############################################################################

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

EMAIL=""
APP_URL="$APP_URL"
DB_URL="$DATABASE_URL"
export PGPASSWORD="${DB_URL##*:}"
export PGPASSWORD="${PGPASSWORD%%@*}"

while [[ $# -gt 0 ]]; do
    case $1 in
        --email) EMAIL="$2"; shift 2 ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

if [[ -z "$EMAIL" ]]; then echo -e "${RED}Error: --email required${NC}"; exit 1; fi

echo -e "${YELLOW}========================================${NC}"
echo "NOTIF-02: Notification History"
echo -e "${YELLOW}========================================${NC}"
echo ""

# 1. User ID
USER_ID="${USER_ID:-test_user_$(date +%s)}"
# Manual check: Bridge does not track emails. Set USER_ID externally or use default.
if [[ -z "$USER_ID" ]]; then echo -e "${RED}✗ User not found${NC}"; exit 1; fi

# 2. Insert Dummy Notification (Direct DB Injection for reliability)
echo -e "${YELLOW}[1/2] Injecting Test Notification${NC}"
psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p $DATABASE_PORT -d "$DATABASE_NAME" -c "INSERT INTO notifications (external_user_id, recipient_email, notification_type, status, subscription_id, provider) VALUES ('$USER_ID', '$EMAIL', 'test_event', 'sent', 'sub_123', 'google_play');" > /dev/null

# 3. Retrieve History
echo -e "${YELLOW}[2/2] Retrieving Notification History${NC}"
RESP=$(curl -s -H "Authorization: Bearer $API_KEY" -X GET "$APP_URL/api/v1/notifications/history"   -H "x-client-version: 99.99.0")

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
