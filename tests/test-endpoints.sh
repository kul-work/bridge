#!/bin/bash
# Test script for Bridge API endpoints

API_URL="http://localhost:3000"
API_KEY="sk_test_hiha_1234567890abcdef"

echo "=== Bridge API Tests ==="
echo ""

# Test 1: Health check (no auth required)
echo "1. Health Check"
curl -s -X GET "$API_URL/health" | jq .
echo ""

# Test 2: POST /api/v1/checkout (requires auth)
echo "2. Create Checkout"
curl -s -X POST "$API_URL/api/v1/checkout" \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "external_user_id": "user_123",
    "email": "test@example.com",
    "provider": "google_play",
    "product_id": "premium_monthly",
    "product_type": "subscription",
    "idempotency_key": "idem_12345"
  }' | jq .
echo ""

# Test 3: POST /api/v1/verify-purchase (requires auth)
echo "3. Verify Purchase"
curl -s -X POST "$API_URL/api/v1/verify-purchase" \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "external_user_id": "user_123",
    "provider": "google_play",
    "subscription_id": "sub_123",
    "purchase_token": "token_abc123",
    "product_type": "subscription"
  }' | jq .
echo ""

# Test 4: GET /api/v1/users/:id/subscription-status (requires auth)
echo "4. Get Subscription Status Snapshot"
curl -s -X GET "$API_URL/api/v1/users/user_123/subscription-status" \
  -H "Authorization: Bearer $API_KEY" | jq .
echo ""

# Test 5: GET /api/v1/subscriptions/:id (requires auth)
echo "5. Get Single Subscription"
curl -s -X GET "$API_URL/api/v1/subscriptions/sub_123?external_user_id=user_123" \
  -H "Authorization: Bearer $API_KEY" | jq .
echo ""


# Test 6: Missing Auth
echo "6. Test Missing Authorization"
curl -s -X GET "$API_URL/api/v1/users/user_123/subscription-status" | jq .
echo ""

echo "=== Tests Complete ==="
