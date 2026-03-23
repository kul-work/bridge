use serde::Serialize;
use serde_json::json;
use std::time::Instant;

/// Structured trace for Google Play Billing transactions
/// Provides observability for verify_purchase and webhook flows
#[derive(Serialize, Clone)]
pub struct BpTrace {
    /// Unique request ID for correlation
    pub request_id: String,
    
    /// Authenticated user ID (optional for webhooks)
    pub user_id: Option<String>,
    
    /// Subscription ID from request
    pub subscription_id: Option<String>,
    
    /// Token hash for privacy (first 12 chars of SHA256)
    pub token_hash: String,
    
    /// Flow type: "verify", "webhook", "lifecycle"
    pub flow: String,
    
    /// Current step in flow: "init", "google_res", "db_commit", "finish", etc.
    pub step: String,
    
    /// Latency in milliseconds
    pub latency_ms: u64,
    
    /// Result status: "success", "error", "pending", "linking_required"
    pub result: String,
    
    /// Error message if applicable
    pub error: Option<String>,
    
    /// Flexible metadata for extra context
    pub metadata: serde_json::Value,
    
    #[serde(skip)]
    start_time: Option<Instant>,
}

impl BpTrace {
    /// Create a new trace for a flow
    pub fn new(flow: &str, request_id: &str) -> Self {
        Self {
            request_id: request_id.to_string(),
            user_id: None,
            subscription_id: None,
            token_hash: String::new(),
            flow: flow.to_string(),
            step: "init".to_string(),
            latency_ms: 0,
            result: "pending".to_string(),
            error: None,
            metadata: json!({}),
            start_time: Some(Instant::now()),
        }
    }

    /// Set the user ID
    pub fn set_user_id(&mut self, user_id: &str) -> &mut Self {
        self.user_id = Some(user_id.to_string());
        self
    }

    /// Set the subscription ID
    pub fn set_subscription_id(&mut self, sub_id: &str) -> &mut Self {
        self.subscription_id = Some(sub_id.to_string());
        self
    }

    /// Set the token hash (first 12 chars of SHA256)
    pub fn set_token_hash(&mut self, token: &str) -> &mut Self {
        use sha2::{Sha256, Digest};
        
        let mut hasher = Sha256::new();
        hasher.update(token.as_bytes());
        let hash = hasher.finalize();
        let hash_hex = format!("{:x}", hash);
        self.token_hash = hash_hex.chars().take(12).collect();
        self
    }

    /// Update step in flow
    pub fn set_step(&mut self, step: &str) -> &mut Self {
        self.step = step.to_string();
        self
    }

    /// Set result status
    pub fn set_result(&mut self, result: &str) -> &mut Self {
        self.result = result.to_string();
        self
    }

    /// Set error message
    pub fn set_error(&mut self, error: &str) -> &mut Self {
        self.error = Some(error.to_string());
        self.result = "error".to_string();
        self
    }

    /// Add metadata field
    pub fn add_metadata(&mut self, key: &str, value: serde_json::Value) -> &mut Self {
        if let serde_json::Value::Object(ref mut map) = self.metadata {
            map.insert(key.to_string(), value);
        }
        self
    }

    /// Calculate and set latency
    pub fn update_latency(&mut self) -> &mut Self {
        if let Some(start) = self.start_time {
            self.latency_ms = start.elapsed().as_millis() as u64;
        }
        self
    }

    /// Emit the trace as a structured log (target: BPT-TRACE)
    pub fn emit(&mut self) {
        self.update_latency();
        
        // Serialize to JSON string for logging
        if let Ok(json_str) = serde_json::to_string(&self) {
            tracing::debug!(
                target: "BPT-TRACE",
                trace = json_str,
                "Billing transaction trace"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trace_creation() {
        let trace = BpTrace::new("verify", "test-id-123");
        assert_eq!(trace.request_id, "test-id-123");
        assert_eq!(trace.flow, "verify");
        assert_eq!(trace.step, "init");
        assert_eq!(trace.result, "pending");
    }

    #[test]
    fn test_token_hash() {
        let mut trace = BpTrace::new("verify", "test-id");
        trace.set_token_hash("my-test-token");
        assert_eq!(trace.token_hash.len(), 12);
        // Verify it's deterministic
        let mut trace2 = BpTrace::new("verify", "test-id");
        trace2.set_token_hash("my-test-token");
        assert_eq!(trace.token_hash, trace2.token_hash);
    }

    #[test]
    fn test_builder_pattern() {
        let mut trace = BpTrace::new("webhook", "webhook-123");
        trace
            .set_user_id("user-456")
            .set_subscription_id("sub-789")
            .set_step("db_commit")
            .set_result("success");
        
        assert_eq!(trace.user_id, Some("user-456".to_string()));
        assert_eq!(trace.subscription_id, Some("sub-789".to_string()));
        assert_eq!(trace.step, "db_commit");
        assert_eq!(trace.result, "success");
    }
}
