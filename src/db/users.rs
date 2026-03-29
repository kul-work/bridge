use crate::error::BridgeError;
use sqlx::PgPool;
use uuid::Uuid;

pub async fn anonymize_user(
    pool: &PgPool,
    app_id: Uuid,
    external_user_id: &str,
    reason: Option<&str>,
) -> Result<(i64, i64, String), BridgeError> {
    use sha2::Digest;
    
    let reason_val = reason.unwrap_or("user_requested_deletion");
    let anon_id = format!(
        "deleted_{}",
        hex::encode(sha2::Sha256::digest(
            format!("{}:{}:{}", app_id, external_user_id, reason_val).as_bytes()
        ))
    );

    let mut tx = pool.begin().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    // Fetch active subscriptions BEFORE cancelling them via provider APIs
    let active_subs: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT subscription_id, provider, purchase_token FROM pay.subscriptions
         WHERE app_id = $1 AND external_user_id = $2 AND status IN ('active', 'trial', 'past_due')"
    )
    .bind(app_id)
    .bind(external_user_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    // Cancel each subscription at the provider level
    for (subscription_id, provider, purchase_token) in active_subs {
        // Attempt provider cancellation, but don't fail the entire operation if one fails
        // (user data should still be anonymized even if a provider cancellation fails)
        if let Err(e) = cancel_subscription_at_provider(
            pool,
            app_id,
            &provider,
            &subscription_id,
            purchase_token.as_deref(),
        )
        .await
        {
            // Log the error but continue with other subscriptions and DB updates
            tracing::warn!(
                app_id = %app_id,
                external_user_id = external_user_id,
                subscription_id = subscription_id,
                provider = provider,
                error = %e,
                "Failed to cancel subscription at provider during user anonymization"
            );
        }
    }

    let sub_count = sqlx::query(
        r#"
        UPDATE pay.subscriptions
        SET external_user_id = $4,
            status = CASE WHEN status IN ('active', 'trial', 'past_due') THEN 'cancelled' ELSE status END,
            google_obfuscated_account_id = NULL,
            google_obfuscated_profile_id = NULL,
            google_linked_purchase_token = NULL,
            google_prepaid_linked_purchase_token = NULL,
            updated_at = NOW(),
            revocation_reason = COALESCE(revocation_reason, $3)
        WHERE app_id = $1
          AND external_user_id = $2
        "#,
    )
    .bind(app_id)
    .bind(external_user_id)
    .bind(reason_val)
    .bind(&anon_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?
    .rows_affected() as i64;

    let pay_count = sqlx::query(
        r#"
        UPDATE pay.payments
        SET external_user_id = $4
        WHERE app_id = $1
          AND external_user_id = $2
        "#,
    )
    .bind(app_id)
    .bind(external_user_id)
    .bind(reason_val)
    .bind(&anon_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?
    .rows_affected() as i64;

    let _ = sqlx::query(
        r#"
        UPDATE pay.fraud_prevention
        SET external_user_id = $4,
            is_anonymized = true,
            anonymized_at = NOW(),
            should_purge_at = NOW() + INTERVAL '90 days',
            updated_at = NOW()
        WHERE app_id = $1
          AND external_user_id = $2
        "#,
    )
    .bind(app_id)
    .bind(external_user_id)
    .bind(reason_val)
    .bind(&anon_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok((sub_count, pay_count, anon_id))
}

/// Helper function to cancel a subscription at the provider level via API
/// Logs errors but doesn't fail the anonymization operation
async fn cancel_subscription_at_provider(
    pool: &PgPool,
    app_id: Uuid,
    provider: &str,
    subscription_id: &str,
    purchase_token: Option<&str>,
) -> Result<(), BridgeError> {
    match provider {
        "creem" => {
            // Get Creem config and call cancel API
            if let Ok(config) = crate::db::provider_configs::get_provider_config(pool, app_id, "creem").await {
                if let Ok(creem_config) = serde_json::from_value::<serde_json::Value>(config.config) {
                    cancel_creem_subscription(subscription_id, &creem_config).await?;
                }
            }
        }
        "lemonsqueezy" => {
            // Get LemonSqueezy config and call cancel API
            if let Ok(config) = crate::db::provider_configs::get_provider_config(pool, app_id, "lemonsqueezy").await {
                if let Ok(ls_config) = serde_json::from_value::<serde_json::Value>(config.config) {
                    cancel_lemonsqueezy_subscription(subscription_id, &ls_config).await?;
                }
            }
        }
        "google_play" => {
            // Get Google Play config and cancel if we have purchase token
            if let Some(token) = purchase_token {
                if let Ok(config) = crate::db::provider_configs::get_provider_config(pool, app_id, "google_play").await {
                    if let Ok(gp_config) = serde_json::from_value::<serde_json::Value>(config.config) {
                        cancel_google_play_subscription(subscription_id, token, &gp_config).await?;
                    }
                }
            }
        }
        "coinbase" => {
            // Coinbase subscriptions are managed via webhooks, no direct API cancellation
            tracing::info!("Coinbase subscription {} - no direct API cancellation needed", subscription_id);
        }
        _ => {
            tracing::warn!("Unknown provider: {}, skipping provider cancellation", provider);
        }
    }

    Ok(())
}

async fn cancel_creem_subscription(
    subscription_id: &str,
    config: &serde_json::Value,
) -> Result<(), BridgeError> {
    let api_key = config.get("api_key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BridgeError::ConfigError("Missing Creem api_key".to_string()))?;
    
    let api_url = config.get("api_url")
        .and_then(|v| v.as_str())
        .unwrap_or("https://api.creem.com");

    let client = reqwest::Client::new();
    let url = format!(
        "{}/subscriptions/{}/cancel",
        api_url.trim_end_matches('/'),
        subscription_id
    );

    let response = client
        .post(&url)
        .header("x-api-key", api_key)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({}))
        .send()
        .await
        .map_err(|e| BridgeError::ProviderError(format!("Creem API call failed: {}", e)))?;

    let status = response.status();
    if !status.is_success() {
        let error_msg = response.text().await.unwrap_or_default();
        return Err(BridgeError::ProviderError(format!(
            "Creem cancel failed: {} - {}",
            status,
            error_msg
        )));
    }

    tracing::info!("Creem subscription {} cancelled via API", subscription_id);
    Ok(())
}

async fn cancel_lemonsqueezy_subscription(
    subscription_id: &str,
    config: &serde_json::Value,
) -> Result<(), BridgeError> {
    let api_key = config.get("api_key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BridgeError::ConfigError("Missing LemonSqueezy api_key".to_string()))?;

    let client = reqwest::Client::new();
    let url = format!(
        "https://api.lemonsqueezy.com/v1/subscriptions/{}",
        subscription_id
    );

    let response = client
        .patch(&url)
        .bearer_auth(api_key)
        .header("Content-Type", "application/vnd.api+json")
        .json(&serde_json::json!({
            "data": {
                "type": "subscriptions",
                "id": subscription_id,
                "attributes": {
                    "cancelled_at": chrono::Utc::now().to_rfc3339()
                }
            }
        }))
        .send()
        .await
        .map_err(|e| BridgeError::ProviderError(format!("LemonSqueezy API call failed: {}", e)))?;

    let status = response.status();
    if !status.is_success() {
        let error_msg = response.text().await.unwrap_or_default();
        return Err(BridgeError::ProviderError(format!(
            "LemonSqueezy cancel failed: {} - {}",
            status,
            error_msg
        )));
    }

    tracing::info!("LemonSqueezy subscription {} cancelled via API", subscription_id);
    Ok(())
}

async fn cancel_google_play_subscription(
    subscription_id: &str,
    purchase_token: &str,
    config: &serde_json::Value,
) -> Result<(), BridgeError> {
    let package_name = config.get("package_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BridgeError::ConfigError("Missing Google Play package_name".to_string()))?;

    let sa_path = config.get("service_account_json")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BridgeError::ConfigError("Missing Google Play service_account_json path".to_string()))?;

    let sa_path_owned = sa_path.to_string();
    let client = tokio::task::spawn_blocking(move || {
        crate::services::google_play::client::GooglePlayClient::new(&sa_path_owned)
    })
    .await
    .map_err(|e| BridgeError::ProviderError(format!("Failed to spawn blocking task: {}", e)))?
    .map_err(|e| BridgeError::ConfigError(format!("Failed to init Google Play client: {}", e)))?;

    client.cancel_subscription(package_name, subscription_id, purchase_token)
        .await
        .map_err(|e| BridgeError::ProviderError(format!("Google Play cancel failed: {}", e)))?;

    tracing::info!("Google Play subscription {} cancelled", subscription_id);
    Ok(())
}
