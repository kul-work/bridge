# Bridge Haiku Implementation Gap Analysis

Based on a thorough review of the provided PRD, architecture, and API contract documents, and comparing them against the current implementation in `bridge.haiku` and the source of truth (`c:\share\hiha`), here is the gap analysis and explicitly voiced contradictions.

## 1. Missing Endpoints
The implementation in `bridge.haiku` is significantly behind the API contract. The following endpoints are documented but entirely **missing** from the routing (`main.rs`) and `src/handlers`:

*   **Subscription Management**:
    *   `POST /api/v1/subscriptions/:subscription_id/cancel`
    *   `POST /api/v1/subscriptions/:subscription_id/resume`
    *   `POST /api/v1/subscriptions/:subscription_id/acknowledge`
    *   `POST /api/v1/subscriptions/:subscription_id/portal`
*   **Google Play Price Step-Up Flow**:
    *   `POST /api/v1/subscriptions/:subscription_id/price-step-up/accept`
    *   `POST /api/v1/subscriptions/:subscription_id/price-step-up/decline`
*   **Payments**:
    *   `GET /api/v1/payments` (Payment history)
    *   `POST /api/v1/purchases/register` (Server-side manual grants)

## 2. Contradictions & Missing Specifications

### A. Provider Disambiguation for Subscriptions (Contract vs. Code)
*   **The Contradiction:** The `pay-tydecode-api-contract.md` explicitly calls out the problem of multiple subscriptions with the same ID across different providers. It mandates: *"always include the `provider` parameter... `provider` is required to disambiguate"*.
*   **Current implementation (`bridge.haiku`):** In `src/handlers/subscriptions.rs`, `GetSubscriptionQuery` only extracts `external_user_id`. It completely ignores `provider`.
*   **Source of Truth (`hiha`):** The legacy monolith has a database constraint `ON CONFLICT (clerk_id, subscription_id, provider)` which confirms the documentation is anatomically correct—you *must* use `provider` to uniquely identify a subscription row. The `haiku` code fails to implement this API contract rule.

### B. Agent Micropayment `charge` Endpoint Mocking
*   **The Issue:** While the agent endpoints (402 flow) are wired up, they are incomplete. In `src/handlers/agent.rs` (`charge` function), the route succeeds but hardcodes `"amount_cents": 0` in the response, along with the comment: `// In practice, would return token amount.` It's returning a dummy placeholder instead of the token cost.

