#!/bin/bash

##############################################################################
# cleanup-nuclear.sh - Complete Database Cleanup for Testing
# 
# Purpose: Reset entire test database to a clean state. Truncates all
#          test-related tables and resets user premium statuses. Only
#          runs on localhost connections for safety.
#
# Usage: ./cleanup-nuclear.sh
#        ./cleanup-nuclear.sh --dry-run
#
# What it cleans:
#   - Payments (pay.payments)
#   - Subscriptions (pay.subscriptions)
#   - Webhook delivery logs (pay.webhook_delivery)
#   - Webhook provider configs (pay.webhook_provider)
#   - Notifications (hiha.notifications)
#   - Webhook callbacks (hiha.webhook_callbacks)
#   - User premium status via clerk_id (hiha.users)
#
# Safety: Only executes on localhost connections (127.0.0.1 or ::1)
##############################################################################

set -euo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Parse arguments
DRY_RUN=false
while [[ $# -gt 0 ]]; do
    case $1 in
        --dry-run)
            DRY_RUN=true
            shift
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# DB credentials from globals.cfg (PGPASSWORD already set in globals.cfg)
DATABASE_USER="$BRIDGE_DB_USER"
DATABASE_HOST="$BRIDGE_DB_HOST"
DATABASE_PORT="$BRIDGE_DB_PORT"
DATABASE_NAME="$BRIDGE_DB_NAME"

echo -e "${YELLOW}========================================${NC}"
echo "Nuclear Database Cleanup"
echo -e "${YELLOW}========================================${NC}"
echo ""

if [[ "$DRY_RUN" == "true" ]]; then
    echo -e "${YELLOW}DRY RUN MODE - No changes will be made${NC}"
    echo ""
fi

# SQL cleanup command
CLEANUP_SQL=$(cat <<'EOF'
-- Localhost safety check
DO $$
BEGIN
  IF inet_client_addr() != '127.0.0.1'::inet AND inet_client_addr() != '::1'::inet THEN
    RAISE EXCEPTION 'Nuclear cleanup only allowed from localhost';
  END IF;
END $$;

SET search_path TO pay;

TRUNCATE payments CASCADE;
TRUNCATE subscriptions CASCADE;
TRUNCATE webhook_delivery CASCADE;
TRUNCATE webhook_provider CASCADE;

SET search_path TO hiha;

TRUNCATE notifications CASCADE;
TRUNCATE webhook_callbacks CASCADE;

DO $$
DECLARE
  v_user_id text := 'user_36lLgcNtpsqKzB5hpan8wYIN5ew';
BEGIN
  -- Reset user to free tier
  UPDATE users SET
    is_premium = false,
    premium_activated_at = NULL,
    premium_expires_at = NULL
  WHERE clerk_id = v_user_id;
  
  COMMIT;
END
$$;
EOF
)

if [[ "$DRY_RUN" == "true" ]]; then
    echo "SQL to execute:"
    echo "$CLEANUP_SQL"
    echo ""
    echo -e "${YELLOW}========================================${NC}"
    echo -e "${YELLOW}DRY RUN COMPLETE - No changes made${NC}"
    echo -e "${YELLOW}========================================${NC}"
else
    echo -e "${YELLOW}Executing cleanup...${NC}"
    echo ""
    
    psql -U "$DATABASE_USER" -h "$DATABASE_HOST" -p "$DATABASE_PORT" -d "$DATABASE_NAME" <<< "$CLEANUP_SQL" 2>&1
    
    echo ""
    echo -e "${YELLOW}========================================${NC}"
    echo -e "${GREEN}✓ Nuclear Cleanup Complete${NC}"
    echo -e "${YELLOW}========================================${NC}"
fi

exit 0
