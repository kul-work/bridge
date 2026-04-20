# Bridge Reference Patterns

## Pattern: New HTTP Handler

**Location:** `src/handlers/`

**Structure:**
```rust
pub async fn handle_request(
  State(state): State<AppState>,
  req: RequestType,
) -> Result<ResponseType> {
  let result = application::process(state, req).await?;
  Ok(Json(result))
}
```

**Rules:**
- Extract + validate params from HTTP request
- Call application/ or services/ layer
- Map result to HTTP response code + body
- NEVER: direct DB access, business logic in handler, provider API calls
- Keep handlers thin: route → orchestrate → respond

---

## Pattern: Webhook Ingress

**Location:** `src/webhooks/ingress.rs`

**Flow:**
1. Receive webhook payload
2. Verify provider signature (HMAC/RSA per provider)
3. Check `webhook_log` table for idempotency (idempotent key)
4. If already processed: return success (don't re-process)
5. Parse payload and normalize to canonical subscription/payment states
6. Delegate to status processor (e.g., `webhooks::processor::process`)
7. Log webhook_log entry with timestamp

**Rules:**
- All webhooks go through ingress, no shortcuts
- Signature verification first, before any parsing
- Idempotency check prevents duplicate side-effects
- Status normalization maps all provider states to Bridge enums
- NEVER: business logic, direct DB mutation, async spawning

---

## Pattern: Provider Integration

**Location:** `src/services/{provider_name}/`

**Structure:**
```rust
pub async fn call_provider_api(
  auth: ProviderAuth,
  request: ProviderRequest,
) -> Result<ProviderResponse> {
  // Build request with provider-specific format
  // Handle provider auth (headers, signing, etc.)
  // Call provider API
  // Translate response to Bridge domain types
  Ok(normalized_result)
}
```

**Rules:**
- Each provider gets its own module (creem, google_play, coinbase, etc.)
- Translate provider API formats ↔ Bridge domain types
- Handle provider-specific auth, signing, rate limits
- Return normalized results (not raw provider JSON)
- NEVER: DB access, webhook handling, HTTP response building, business logic

---

## Pattern: DB Query Module

**Location:** `src/db/`

**Structure:**
```rust
pub async fn get_subscription_by_id(
  db: &PgPool,
  sub_id: &str,
) -> Result<Subscription> {
  sqlx::query_as::<_, Subscription>(
    "SELECT * FROM pay.subscriptions WHERE id = $1"
  )
  .bind(sub_id)
  .fetch_one(db)
  .await
  .map_err(|e| anyhow!("fetch failed: {}", e))
}
```

**Rules:**
- Pure SQL via SQLx
- Parameterized queries (always `$1`, `$2`, etc.; never string concat)
- Return typed domain structs (not raw JSON)
- Error: convert sqlx errors to Result<T>
- NEVER: business logic, error policy, HTTP concerns, webhook logic

---

## Pattern: Application Orchestrator

**Location:** `src/application/`

**Structure:**
```rust
pub async fn process_subscription_change(
  state: &AppState,
  user_id: &str,
  change: SubscriptionChange,
) -> Result<Subscription> {
  // Fetch current state
  let current = db::get_subscription(state.db, user_id).await?;
  
  // Validate business rules
  validate_transition(&current, &change)?;
  
  // Delegate to service (payment, email, etc.)
  let updated = services::apply_change(state, current, change).await?;
  
  // Persist result
  db::update_subscription(state.db, &updated).await?;
  
  Ok(updated)
}
```

**Rules:**
- Owns domain business rules and state transitions
- Orchestrates calls to services/ and db/ layers
- Validates preconditions and invariants
- Returns domain results, not HTTP responses
- NEVER: HTTP concerns, direct provider calls, duplicate logic from services

---

## Pattern: Background Worker

**Location:** `src/workers/`

**Structure:**
```rust
pub async fn run_reconciliation_worker(pool: &PgPool) -> Result<()> {
  loop {
    // Fetch stale items
    let items = db::get_stale_subscriptions(pool).await?;
    
    for item in items {
      // Verify against provider
      let status = providers::verify_status(item.provider_id).await?;
      
      // Update local state if diverged
      if status != item.local_status {
        db::update_subscription_status(pool, &item.id, status).await?;
      }
    }
    
    sleep(RECONCILIATION_INTERVAL).await;
  }
}
```

**Rules:**
- Long-running async tasks (reconciliation, cleanup, price step-up, pause scheduler)
- Spawn once at startup, run in background
- Fetch work from DB, process idempotently, update results
- Use exponential backoff for retries
- Log state changes and errors
- NEVER: HTTP concerns, handler-style request/response

---

## Pattern: Port / Adapter Trait

**Location:** `src/ports/`

**Structure:**
```rust
pub trait EmailProvider: Send + Sync {
  async fn send_receipt(&self, user_id: &str, amount: i64) -> Result<()>;
}

pub struct SendgridEmail { /* ... */ }

impl EmailProvider for SendgridEmail {
  async fn send_receipt(&self, user_id: &str, amount: i64) -> Result<()> {
    // Sendgrid-specific implementation
    Ok(())
  }
}
```

**Rules:**
- Define external service contracts as traits
- Implementations in services/ (one per provider)
- Injected via State or method params
- Enables testing (mock implementations)
- NEVER: business logic, multiple responsibilities per trait
