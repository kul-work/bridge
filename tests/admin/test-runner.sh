#!/bin/bash

##############################################################################
# Admin Test Master Runner
##############################################################################

set -uo pipefail

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

# Defaults
CLEAR_FIRST=false
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --clear)
            CLEAR_FIRST=true
            shift
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

echo -e "${CYAN}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║             Bridge Admin Interface - Test Runner           ║${NC}"
echo -e "${CYAN}╚════════════════════════════════════════════════════════════╝${NC}"
echo "Time:  $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo ""

cd "$SCRIPT_DIR"
source "globals.cfg"

# Verify Bridge server is running
echo "Checking if Bridge server is running at $BRIDGE_API_URL..."
if ! curl -s -f "$BRIDGE_API_URL/health" >/dev/null; then
    echo -e "${RED}Error: Bridge server is not running at $BRIDGE_API_URL.${NC}"
    echo -e "${YELLOW}Please start the Bridge server before running these tests.${NC}"
    exit 1
fi
echo -e "${GREEN}✓ Bridge server is online.${NC}"
echo ""

# Ensure mock Clerk port is clean
echo "Preparing Mock Clerk server on port $MOCK_CLERK_PORT..."
curl -s "http://localhost:$MOCK_CLERK_PORT/shutdown" >/dev/null 2>&1 || true
sleep 1

# Start Mock Clerk server in the background
python "$SCRIPT_DIR/mock_clerk.py" &
MOCK_CLERK_PID=$!

# Register exit trap to request self-shutdown of the mock clerk server
cleanup_clerk() {
    echo -e "\n${YELLOW}Shutting down Mock Clerk server...${NC}"
    curl -s "http://localhost:$MOCK_CLERK_PORT/shutdown" >/dev/null 2>&1 || true
}
trap cleanup_clerk EXIT

# Wait for mock clerk to become ready
MOCK_READY=false
for i in {1..5}; do
    if curl -s "http://localhost:$MOCK_CLERK_PORT/.well-known/jwks.json" >/dev/null 2>&1; then
        MOCK_READY=true
        break
    fi
    sleep 1
done

if [[ "$MOCK_READY" != "true" ]]; then
    echo -e "${RED}Error: Failed to start Mock Clerk server on port $MOCK_CLERK_PORT.${NC}"
    exit 1
fi
echo -e "${GREEN}✓ Mock Clerk server is ready.${NC}"
echo ""

# Optional clear
if [[ "$CLEAR_FIRST" == "true" ]]; then
    echo -e "${YELLOW}Clearing test data before run...${NC}"
    bash cleanup-runner.sh
    echo ""
fi

# Run the test suite
EXIT_CODE=0
if bash run-all-admin-tests.sh; then
    echo -e "${GREEN}✓ Admin Test Suite PASSED${NC}"
else
    echo -e "${RED}✗ Admin Test Suite FAILED${NC}"
    EXIT_CODE=1
fi

# Run final cleanup
echo -e "${YELLOW}Running post-test cleanup...${NC}"
bash cleanup-runner.sh

exit $EXIT_CODE
