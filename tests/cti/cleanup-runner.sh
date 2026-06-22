#!/bin/bash

##############################################################################
# CTI Master Cleanup Runner
#
# Purpose: Single entry point to reset state for all Contract & Tenant
#          Isolation tests. Executes cleanup scripts for both sub-suites.
#
# Usage: ./cleanup-runner.sh
##############################################################################

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/globals.cfg"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}Running cti master cleanup${NC}"

cd "$SCRIPT_DIR"

run_clean() {
    local script="$1"
    if [[ -f "$script" ]]; then
        bash "$script" > /dev/null 2>&1
        echo -e "${GREEN}✓ Cleaned: $script${NC}"
    else
        echo -e "${YELLOW}⚠ Missing: $script (skipping)${NC}"
    fi
}

# 1. Isolation test data
run_clean "cleanup-all-iso.sh"

# 2. Contract test data
run_clean "cleanup-all-contract.sh"

# 3. Clean Report Files (JSON)
echo -e "${BLUE}Cleaning report files...${NC}"
rm -f *-report.json
rm -f *-summary.json

echo -e "${GREEN}CTI Grand Cleanup Complete.${NC}"
exit 0