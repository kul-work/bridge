#!/bin/bash

##############################################################################
# Cleanup Runner for Bridge Admin Interface Tests
##############################################################################

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

echo "Cleaning up admin test data..."

# Clean up webhook delivery and provider records matching admin patterns
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.webhook_delivery WHERE webhook_provider_id IN (
     SELECT id FROM pay.webhook_provider WHERE provider_webhook_id LIKE 'admin-%'
   );" 2>/dev/null || true

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.webhook_provider WHERE provider_webhook_id LIKE 'admin-%';" 2>/dev/null || true

psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c \
  "DELETE FROM pay.subscriptions WHERE purchase_token LIKE 'test-admin-%';" 2>/dev/null || true

# Restore notes for the test app if it was modified
psql -U "$BRIDGE_DB_USER" -h "$BRIDGE_DB_HOST" -p "$BRIDGE_DB_PORT" -d "$BRIDGE_DB_NAME" -c \
  "UPDATE pay.apps SET notes = NULL WHERE notes LIKE 'audit-test-note-%';" 2>/dev/null || true

echo "Cleanup complete."
