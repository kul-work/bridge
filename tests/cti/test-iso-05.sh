#!/bin/bash

##############################################################################
# ISO-05: Checkout Idempotency Key Isolation
#
# Purpose: Verify that idempotency keys are scoped per app, not globally.
#          App A and App B using the same idempotency_key should both succeed
#          independently and not receive each other's cached checkout response.
#
# Usage: ./test-iso-05.sh
#
# TESTPLAN Reference:
#   ISO-05: Checkout idempotency keys are scoped per app, not globally.
#   checkout_idempotency table has UNIQUE (app_id, idempotency_key).
##############################################################################

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

TIMESTAMP=$(date +%s)
TEST_STARTED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
TEST_RUN_ID="iso-05-${TIMESTAMP}-$$"
REPORT_FILE="iso-05-report.json"
IDEMPOTENCY_KEY="iso-05-key-$TEST_RUN_ID"
USER_ID="test_iso_05_user_$TEST_RUN_ID"

echo -e "${YELLOW}========================================${NC}"
echo "ISO-05: Checkout Idempotency Key Isolation"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Test Run ID: $TEST_RUN_ID"
echo ""

if [[ -z "$APP_A_ID" ]] || [[ -z "$APP_B_ID" ]]; then
    echo -e "${RED}✗ Both APP_A_ID and APP_B_ID must be set.${NC}"
    exit 1
fi

# Step 1: Seed idempotency record for App A
echo -e "${YELLOW}[1/4] Seeding checkout idempotency record for App A${NC}"

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.checkout_idempotency WHERE idempotency_key = '$IDEMPOTENCY_KEY';" 2>/dev/null

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "INSERT INTO pay.checkout_idempotency (app_id, idempotency_key, request_fingerprint, response_payload)
   VALUES ('$APP_A_ID', '$IDEMPOTENCY_KEY', 'fingerprint_a_$TEST_RUN_ID',
           '{\"checkout_url\": \"https://checkout.example.com/app-a\", \"provider\": \"creem\"}');" 2>/dev/null

echo -e "${GREEN}✓ Created idempotency record for App A (key: $IDEMPOTENCY_KEY)${NC}"
echo ""

# Step 2: Verify App A's idempotency record exists and is scoped to App A
echo -e "${YELLOW}[2/4] Verifying App A's idempotency record is app-scoped${NC}"

APP_A_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -At -c \
  "SELECT COUNT(*) FROM pay.checkout_idempotency WHERE app_id = '$APP_A_ID' AND idempotency_key = '$IDEMPOTENCY_KEY';" 2>/dev/null | tr -d '[:space:]')

if [[ "$APP_A_COUNT" != "1" ]]; then
    echo -e "${RED}✗ App A should have 1 idempotency record, got $APP_A_COUNT${NC}"
    exit 1
fi
echo -e "${GREEN}✓ App A has its idempotency record (count: $APP_A_COUNT)${NC}"
echo ""

# Step 3: Verify App B can use the SAME idempotency key without collision
echo -e "${YELLOW}[3/4] Verifying App B can use the same idempotency key independently${NC}"

# Insert the same key for App B — this should succeed because the UNIQUE constraint is (app_id, idempotency_key)
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "INSERT INTO pay.checkout_idempotency (app_id, idempotency_key, request_fingerprint, response_payload)
   VALUES ('$APP_B_ID', '$IDEMPOTENCY_KEY', 'fingerprint_b_$TEST_RUN_ID',
           '{\"checkout_url\": \"https://checkout.example.com/app-b\", \"provider\": \"creem\"}');" 2>/dev/null

APP_B_COUNT=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -At -c \
  "SELECT COUNT(*) FROM pay.checkout_idempotency WHERE app_id = '$APP_B_ID' AND idempotency_key = '$IDEMPOTENCY_KEY';" 2>/dev/null | tr -d '[:space:]')

# Verify both apps have their own records
TOTAL_SAME_KEY=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -At -c \
  "SELECT COUNT(*) FROM pay.checkout_idempotency WHERE idempotency_key = '$IDEMPOTENCY_KEY';" 2>/dev/null | tr -d '[:space:]')

ISOLATION_OK="true"
if [[ "$APP_B_COUNT" != "1" ]]; then
    echo -e "${RED}✗ App B's insert with same key failed (expected 1 row, got $APP_B_COUNT)${NC}"
    ISOLATION_OK="false"
else
    echo -e "${GREEN}✓ App B can use the same idempotency key independently (count: $APP_B_COUNT)${NC}"
fi

if [[ "$TOTAL_SAME_KEY" != "2" ]]; then
    echo -e "${RED}✗ Expected 2 total records with same key (1 per app), got $TOTAL_SAME_KEY${NC}"
    ISOLATION_OK="false"
else
    echo -e "${GREEN}✓ Both apps have independent records with the same key (total: $TOTAL_SAME_KEY)${NC}"
fi

# Verify each app gets its own response payload (not the other's)
APP_A_PAYLOAD=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -At -c \
  "SELECT response_payload->>'checkout_url' FROM pay.checkout_idempotency WHERE app_id = '$APP_A_ID' AND idempotency_key = '$IDEMPOTENCY_KEY';" 2>/dev/null | tr -d '[:space:]')

APP_B_PAYLOAD=$(psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -At -c \
  "SELECT response_payload->>'checkout_url' FROM pay.checkout_idempotency WHERE app_id = '$APP_B_ID' AND idempotency_key = '$IDEMPOTENCY_KEY';" 2>/dev/null | tr -d '[:space:]')

if [[ "$APP_A_PAYLOAD" == "$APP_B_PAYLOAD" ]]; then
    echo -e "${RED}✗ App A and App B have the same response payload (collision!)${NC}"
    ISOLATION_OK="false"
else
    echo -e "${GREEN}✓ App A and App B have different response payloads (isolated)${NC}"
fi
echo ""

# Step 4: Cleanup
echo -e "${YELLOW}[4/4] Cleanup${NC}"

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p $BRIDGE_DB_PORT -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.checkout_idempotency WHERE idempotency_key = '$IDEMPOTENCY_KEY';" 2>/dev/null
echo -e "${GREEN}✓ Cleaned up test data${NC}"
echo ""

if [[ "$ISOLATION_OK" == "true" ]]; then
    TEST_STATUS="pass"
    TEST_RESULT_MSG="${GREEN}✓ ISO-05 Test PASSED${NC}"
else
    TEST_STATUS="fail"
    TEST_RESULT_MSG="${RED}✗ ISO-05 Test FAILED${NC}"
fi

TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "ISO-05",
  "test_name": "Checkout Idempotency Key Isolation",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "$TEST_STATUS",
  "idempotency_key": "$IDEMPOTENCY_KEY",
  "results": {
    "app_a_count": $APP_A_COUNT,
    "app_b_count": $APP_B_COUNT,
    "total_same_key": $TOTAL_SAME_KEY,
    "app_a_payload": "$APP_A_PAYLOAD",
    "app_b_payload": "$APP_B_PAYLOAD",
    "isolation_enforced": $ISOLATION_OK
  }
}
EOF

echo -e "${YELLOW}========================================${NC}"
echo -e "$TEST_RESULT_MSG"
echo -e "${YELLOW}========================================${NC}"
echo ""
echo "Report saved to: $REPORT_FILE"

if [[ "$TEST_STATUS" == "fail" ]]; then
    exit 1
fi
exit 0