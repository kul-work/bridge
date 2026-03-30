#!/bin/bash

# Cleanup script for API & Notification tests (Section H)

echo "Cleaning up API test artifacts..."

rm -f api-01-report.json
rm -f notif-01-report.json
rm -f notif-02-report.json

echo "Cleanup complete."
