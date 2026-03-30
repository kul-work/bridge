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
#   - User premium status (sets is_premium=false, expires_at=NULL)
#   - Subscriptions table (TRUNCATE)
#   - Payments table (TRUNCATE)
#   - Notifications table (TRUNCATE)
#   - Webhooks table (TRUNCATE)
#   - Rate limits table (TRUNCATE)
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

# Extract DB password
export PGPASSWORD="${DATABASE_URL##*:}"
export PGPASSWORD="${PGPASSWORD%%@*}"

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
-- Clean DB (localhost only)
DO $$
BEGIN
  -- Only run if connected from localhost
  IF inet_client_addr() = '127.0.0.1'::inet OR inet_client_addr() = '::1'::inet THEN
    UPDATE "public"."users" SET "is_premium"='false', "premium_activated_at"=NULL, "premium_expires_at"=NULL WHERE "email"='test@test.ro';
    UPDATE "public"."users" SET "is_premium"='false', "premium_activated_at"=NULL, "premium_expires_at"=NULL WHERE "email"='test2@test.ro';
    TRUNCATE pay.subscriptions;
    TRUNCATE pay.payments;
    TRUNCATE notifications;
    TRUNCATE webhooks;
    TRUNCATE rate_limits;
    RAISE NOTICE 'Cleanup completed on localhost';
  ELSE
    RAISE WARNING 'Cleanup skipped: not connected from localhost';
  END IF;
END $$;
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
