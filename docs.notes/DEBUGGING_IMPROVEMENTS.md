# Proposed Debugging Improvements for Bridge

This document outlines a series of technical and process improvements designed to enhance the observability, traceability, and debuggability of the Bridge payment service and its integration with client applications (e.g., Hiha).

---

## 1. Observability & Logging

### 1.1 Unified Correlation IDs
*   **Description**: Generate a unique `X-Correlation-ID` (UUID) at the edge (either at the Test Runner or Bridge API/Webhook ingress).
*   **Requirement**: 
    *   Include this ID in all logs (Bridge, Hiha, and Runner).
    *   Pass the ID via headers in all downstream requests (Bridge → App callback).
*   **Benefit**: Enables "single-trace" debugging. You can grep one ID across all service logs to see the complete lifecycle of a transaction.

### 1.2 Structured JSON Logging
*   **Description**: Transition from plain-text logs to structured JSON formatting.
*   **Requirement**: Include fields like `app_id`, `provider`, `event_type`, `external_user_id`, and `correlation_id` as top-level JSON keys.
*   **Benefit**: Allows for powerful filtering and analysis using tools like `jq` or centralized log aggregators (ELK, CloudWatch).

### 1.3 Enhanced Forwarding Diagnostics
*   **Description**: Log full request and response context for failed webhook deliveries.
*   **Requirement**: On any non-2xx response from a client app, log:
    *   The outbound JSON payload (scrubbed of PII).
    *   The full response body returned by the app.
*   **Benefit**: Eliminates the "silent failure" problem where Bridge knows a delivery failed but doesn't know *why* the app rejected it.

---

## 2. Test Runner Enhancements

### 2.1 Verbose Debug Mode
*   **Description**: Add a `--verbose` or `-v` flag to `test-runner.sh`.
*   **Requirement**: 
    *   Print the full `curl` command (including headers and body) for every API call.
    *   Tail the Bridge logs in a background process or dump them immediately upon failure.
*   **Benefit**: Provides immediate visual feedback on the data being sent and the server's internal reaction.

### 2.2 Automated State Snapshots
*   **Description**: Capture database state automatically on test failure.
*   **Requirement**: If an assertion fails, the script should automatically execute:
    ```sql
    SELECT * FROM pay.subscriptions WHERE external_user_id = $1;
    SELECT * FROM pay.payments WHERE external_user_id = $1;
    ```
    And append the results to the test report.
*   **Benefit**: Confirms if the DB state matched expectations without manual intervention.

---

## 3. Error Handling & Diagnostics

### 3.1 "Lookup Cascade" Traceability
*   **Description**: Detailed logging for user resolution failures.
*   **Requirement**: When a webhook is discarded due to "unable to resolve external_user_id," log the specific steps that failed:
    *   `[1/3] purchase_token lookup: NOT FOUND`
    *   `[2/3] metadata.external_user_id lookup: NOT FOUND`
    *   `[3/3] email fallback: DISABLED`
*   **Benefit**: Pinpoints exactly where the identity mapping broke down.

### 3.2 Diagnostic Debug Context
*   **Description**: Richer error responses in non-production environments.
*   **Requirement**: Include a `debug_context` field in error responses (4xx/5xx) that explains the internal state or the mock provider's logic.
*   **Benefit**: Distinguishes between a system error and an intentional error triggered by a specific test token.

---

## 4. Admin Tools

### 4.1 Webhook Inspection API
*   **Description**: A dedicated endpoint for developers to inspect the "history" of a webhook.
*   **Requirement**: `GET /admin/webhooks/:provider_webhook_id/debug` should return:
    *   Raw ingress payload.
    *   Normalization steps taken.
    *   History of delivery attempts to the app.
*   **Benefit**: Provides a "God view" of the webhook flow for troubleshooting difficult race conditions or signature issues.
