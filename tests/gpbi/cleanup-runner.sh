#!/bin/bash

##############################################################################
# GPBI Master Cleanup Runner
#
# Purpose: Single entry point to reset state for all Google Play Billing tests.
#          Executes specific cleanup scripts for all sub-modules.
#
# Location: tests/gpbi/cleanup-runner.sh
#
# Usage: ./cleanup-runner.sh --email "user@example.com"
##############################################################################

set -uo pipefail

# Source global configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

# Colors
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

echo -e "${BLUE}running gpbi master cleanup for: $EMAIL${NC}"

cd "$SCRIPT_DIR"

# Helper for silent cleanup
run_clean() {
    local script="$1"
    if [[ -f "$script" ]]; then
        bash "$script" --email "$EMAIL" > /dev/null 2>&1
        echo -e "${GREEN}✓ Cleaned: $script${NC}"
    else
        echo -e "${YELLOW}⚠ Missing: $script (skipping)${NC}"
    fi
}

# 1. Critical State (Commerce) - Do these first
run_clean "cleanup-all-sub.sh"
run_clean "cleanup-all-otp.sh"

# 2. Infrastructure & Validation State
run_clean "cleanup-all-acc.sh"
run_clean "cleanup-all-whk.sh"
run_clean "cleanup-all-net.sh"
run_clean "cleanup-all-err.sh"
run_clean "cleanup-all-log.sh"
run_clean "cleanup-all-api.sh"

# 3. Clean Report Files (JSON)
echo -e "${BLUE}Cleaning report files...${NC}"
rm -f *-report.json
rm -f *-summary.json

echo -e "${GREEN}Grand Cleanup Complete.${NC}"
exit 0
