use crate::db::database::set_local_app_id;
use crate::error::BridgeError;
use crate::ports::UserSubscriptionCancellationSnapshot;
use sqlx::PgPool;
use uuid::Uuid;

pub async fn list_user_subscriptions_to_cancel(
    pool: &PgPool,
    app_id: Uuid,
    external_user_id: &str,
) -> Result<Vec<UserSubscriptionCancellationSnapshot>, BridgeError> {
    let mut tx = pool.begin().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
    set_local_app_id(&mut tx, app_id).await?;

    let rows = sqlx::query_as::<_, (String, String, Option<String>)>(
        "SELECT subscription_id, provider, purchase_token FROM pay.subscriptions
         WHERE app_id = $1 AND external_user_id = $2 AND status IN ('active', 'trial', 'past_due')"
    )
    .bind(app_id)
    .bind(external_user_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(
        rows.into_iter()
            .map(|(subscription_id, provider, purchase_token)| UserSubscriptionCancellationSnapshot {
                subscription_id,
                provider,
                purchase_token,
            })
            .collect()
    )
}

pub async fn anonymize_user_records(
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
    set_local_app_id(&mut tx, app_id).await?;

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
        SET external_user_id = $3
        WHERE app_id = $1
          AND external_user_id = $2
        "#,
    )
    .bind(app_id)
    .bind(external_user_id)
    .bind(&anon_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?
    .rows_affected() as i64;

    let _ = sqlx::query(
        r#"
        UPDATE pay.fraud_prevention
        SET external_user_id = $3,
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
    .bind(&anon_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok((sub_count, pay_count, anon_id))
}

pub async fn cleanup_purged_fraud_prevention(pool: &PgPool) -> Result<(), BridgeError> {
    sqlx::query("SELECT pay.cleanup_purged_fraud_prevention()")
        .execute(pool)
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(())
}
