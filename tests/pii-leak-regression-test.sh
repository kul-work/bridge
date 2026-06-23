#!/bin/bash
# PII Leakage Regression Test for Bridge
#
# Runs the Rust test suite with logs captured and asserts that
# no email addresses, raw purchase tokens, or webhook secrets are printed in the log output.

LOG_FILE="tests_run.log"

# Clean up log file on exit on all paths
cleanup() {
    rm -f "$LOG_FILE"
}
trap cleanup EXIT

echo "Running Cargo tests with logs captured..."
if ! cargo test -- --nocapture > "$LOG_FILE" 2>&1; then
    echo "ERROR: Cargo tests failed! Showing log output:"
    cat "$LOG_FILE"
    exit 1
fi

echo "Checking logs for PII leakage..."

# 1. Check for general email addresses (excluding github/git URLs or cargo registry paths)
LEAKED_EMAILS=$(grep -oE "[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}" "$LOG_FILE" | grep -v "github.com" | grep -v "git@github" || true)

if [ -n "$LEAKED_EMAILS" ]; then
    echo "ERROR: Leaked emails found in tests log!"
    echo "$LEAKED_EMAILS"
    exit 1
fi

# 2. Check for specific test customer emails
LEAKED_TEST_USER=$(grep -o "user@example.com\|user2@example.com\|user3@example.com" "$LOG_FILE" || true)
if [ -n "$LEAKED_TEST_USER" ]; then
    echo "ERROR: Leaked test customer email in log: $LEAKED_TEST_USER"
    exit 1
fi

# 3. Check for specific raw purchase tokens/secrets that should be redacted
SENTINELS=(
    "shared_purchase_token"
    "otp_purchase_token"
    "co_otp_001"
    "purchase-token"
    "token_abc123"
    "my_super_secret_callback_key"
)

for sentinel in "${SENTINELS[@]}"; do
    if grep -q "$sentinel" "$LOG_FILE"; then
        echo "ERROR: Leaked sentinel '$sentinel' found in tests log!"
        grep "$sentinel" "$LOG_FILE"
        exit 1
    fi
done

echo "SUCCESS: No PII leakage or secret leakage detected in logs."
exit 0
