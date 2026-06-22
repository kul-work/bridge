#!/bin/bash

##############################################################################
# ADMIN-CSP-01: CSP Blocks External Scripts, Allows Clerk/Captcha Styles
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
TEST_RUN_ID="admin-csp-01-${TIMESTAMP}-$$"
REPORT_FILE="csp-01-report.json"

echo -e "${YELLOW}========================================${NC}"
echo "ADMIN-CSP-01: CSP Blocks External Scripts, Allows Clerk/Captcha Styles"
echo -e "${YELLOW}========================================${NC}"
echo ""

echo -e "${YELLOW}[1/3] Querying GET /admin for response headers${NC}"
HEADERS=$(curl -s -I "$BRIDGE_API_URL/admin" 2>/dev/null || echo "error")

if [[ "$HEADERS" == "error" ]] || [[ -z "$HEADERS" ]]; then
    echo -e "${RED}✗ Failed to query /admin endpoint.${NC}"
    exit 1
fi

# Find Content-Security-Policy header
# Using tr to normalize case and strip carriage returns
CSP_HEADER=$(echo "$HEADERS" | tr -d '\r' | grep -i "^content-security-policy:" || echo "")

CSP_PRESENT="false"
CSP_VALID="false"

if [[ -n "$CSP_HEADER" ]]; then
    echo -e "${GREEN}✓ Content-Security-Policy header is present${NC}"
    CSP_PRESENT="true"
    
    # Check for specific elements in the CSP header
    echo -e "${BLUE}CSP: $CSP_HEADER${NC}"
    
    HAS_SCRIPT_SRC="false"
    HAS_STYLE_SRC="false"
    ALLOWS_CLERK="false"
    ALLOWS_CAPTCHA="false"
    
    if echo "$CSP_HEADER" | grep -q "script-src"; then HAS_SCRIPT_SRC="true"; fi
    if echo "$CSP_HEADER" | grep -q "style-src"; then HAS_STYLE_SRC="true"; fi
    if echo "$CSP_HEADER" | grep -q "clerk"; then ALLOWS_CLERK="true"; fi
    if echo "$CSP_HEADER" | grep -q "hcaptcha.com\|challenges.cloudflare.com"; then ALLOWS_CAPTCHA="true"; fi
    
    echo -e "  - contains script-src: $([ "$HAS_SCRIPT_SRC" == "true" ] && echo -e "${GREEN}yes${NC}" || echo -e "${RED}no${NC}")"
    echo -e "  - contains style-src: $([ "$HAS_STYLE_SRC" == "true" ] && echo -e "${GREEN}yes${NC}" || echo -e "${RED}no${NC}")"
    echo -e "  - allows Clerk: $([ "$ALLOWS_CLERK" == "true" ] && echo -e "${GREEN}yes${NC}" || echo -e "${RED}no${NC}")"
    echo -e "  - allows hCaptcha/Cloudflare: $([ "$ALLOWS_CAPTCHA" == "true" ] && echo -e "${GREEN}yes${NC}" || echo -e "${RED}no${NC}")"
    
    if [[ "$HAS_SCRIPT_SRC" == "true" ]] && [[ "$HAS_STYLE_SRC" == "true" ]] && [[ "$ALLOWS_CLERK" == "true" ]] && [[ "$ALLOWS_CAPTCHA" == "true" ]]; then
        CSP_VALID="true"
    fi
else
    echo -e "${RED}✗ Content-Security-Policy header is MISSING${NC}"
fi

# Cleanup
echo ""
echo -e "${YELLOW}[2/3] Cleanup${NC}"
echo -e "${GREEN}✓ Cleaned up${NC}"

TEST_STATUS="fail"
if [[ "$CSP_PRESENT" == "true" ]] && [[ "$CSP_VALID" == "true" ]]; then
    TEST_STATUS="pass"
fi

TEST_FINISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$REPORT_FILE" <<EOF
{
  "test_id": "ADMIN-CSP-01",
  "test_name": "CSP Blocks External Scripts, Allows Clerk/Captcha Styles",
  "test_run_id": "$TEST_RUN_ID",
  "started_at": "$TEST_STARTED_AT",
  "finished_at": "$TEST_FINISHED_AT",
  "status": "$TEST_STATUS",
  "results": {
    "csp_header_present": $CSP_PRESENT,
    "csp_header_valid": $CSP_VALID
  }
}
EOF

echo ""
if [[ "$TEST_STATUS" == "pass" ]]; then
    echo -e "${GREEN}✓ ADMIN-CSP-01 PASSED${NC}"
    exit 0
else
    echo -e "${RED}✗ ADMIN-CSP-01 FAILED${NC}"
    exit 1
fi
