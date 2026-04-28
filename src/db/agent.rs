use crate::db::database::set_local_app_id;
use crate::error::BridgeError;
use sqlx::{PgPool, FromRow};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc, Duration};

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct AgentCredit {
    pub id: Uuid,
    pub app_id: Uuid,
    pub external_user_id: String,
    pub balance_cents: i32,
    pub lifetime_spent_cents: i32,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct AgentPaymentToken {
    pub id: Uuid,
    pub app_id: Uuid,
    pub external_user_id: String,
    pub endpoint: String,
    pub amount_cents: i32,
    pub nonce: String,
    pub used: bool,
    pub used_at: Option<DateTime<Utc>>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct AgentTransaction {
    pub request_type: String,
    pub amount_cents: i32,
    pub charge_id: Option<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

async fn begin_app_tx<'a>(
    pool: &'a PgPool,
    app_id: Uuid,
) -> Result<sqlx::Transaction<'a, sqlx::Postgres>, BridgeError> {
    let mut tx = pool.begin().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
    set_local_app_id(&mut tx, app_id).await?;
    Ok(tx)
}

pub async fn get_agent_credit(
    pool: &PgPool,
    app_id: Uuid,
    external_user_id: &str,
) -> Result<Option<AgentCredit>, BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let credit = sqlx::query_as::<_, AgentCredit>(
        "SELECT * FROM pay.agent_credits WHERE app_id = $1 AND external_user_id = $2 LIMIT 1"
    )
    .bind(app_id)
    .bind(external_user_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(credit)
}

pub async fn list_agent_transactions(
    pool: &PgPool,
    app_id: Uuid,
    external_user_id: &str,
) -> Result<Vec<AgentTransaction>, BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let transactions = sqlx::query_as::<_, AgentTransaction>(
        "SELECT request_type, amount_cents, charge_id, status, created_at
         FROM pay.agent_transactions
         WHERE app_id = $1 AND external_user_id = $2
         ORDER BY created_at DESC"
    )
    .bind(app_id)
    .bind(external_user_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(transactions)
}

pub async fn cleanup_expired_agent_tokens(pool: &PgPool) -> Result<(), BridgeError> {
    sqlx::query("SELECT pay.cleanup_expired_agent_tokens()")
        .execute(pool)
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(())
}

pub async fn topup_agent(
    pool: &PgPool,
    app_id: Uuid,
    external_user_id: &str,
    amount_cents: i32,
    charge_id: Option<&str>,
) -> Result<AgentCredit, BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;

    let credit = sqlx::query_as::<_, AgentCredit>(
        r#"
        INSERT INTO pay.agent_credits (app_id, external_user_id, balance_cents, lifetime_spent_cents, updated_at)
        VALUES ($1, $2, $3, $4, NOW())
        ON CONFLICT (app_id, external_user_id)
        DO UPDATE SET
            balance_cents = pay.agent_credits.balance_cents + EXCLUDED.balance_cents,
            lifetime_spent_cents = pay.agent_credits.lifetime_spent_cents + EXCLUDED.lifetime_spent_cents,
            updated_at = NOW()
        RETURNING *
        "#,
    )
    .bind(app_id)
    .bind(external_user_id)
    .bind(amount_cents)
    .bind(0)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    record_agent_transaction(
        &mut tx,
        app_id,
        external_user_id,
        "topup",
        amount_cents,
        charge_id,
    )
    .await?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(credit)
}

pub async fn upsert_agent_credit(
    pool: &PgPool,
    app_id: Uuid,
    external_user_id: &str,
    balance_delta: i32,
    spent_delta: i32,
) -> Result<AgentCredit, BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let credit = sqlx::query_as::<_, AgentCredit>(
        r#"
        INSERT INTO pay.agent_credits (app_id, external_user_id, balance_cents, lifetime_spent_cents, updated_at)
        VALUES ($1, $2, $3, $4, NOW())
        ON CONFLICT (app_id, external_user_id)
        DO UPDATE SET
            balance_cents = pay.agent_credits.balance_cents + EXCLUDED.balance_cents,
            lifetime_spent_cents = pay.agent_credits.lifetime_spent_cents + EXCLUDED.lifetime_spent_cents,
            updated_at = NOW()
        RETURNING *
        "#
    )
    .bind(app_id)
    .bind(external_user_id)
    .bind(balance_delta)
    .bind(spent_delta)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(credit)
}

pub async fn insert_agent_token(
    pool: &PgPool,
    app_id: Uuid,
    external_user_id: &str,
    endpoint: &str,
    amount_cents: i32,
    nonce: &str,
) -> Result<AgentPaymentToken, BridgeError> {
    let expires_at = Utc::now() + Duration::minutes(10);
    let mut tx = begin_app_tx(pool, app_id).await?;
    let token = sqlx::query_as::<_, AgentPaymentToken>(
        r#"
        INSERT INTO pay.agent_payment_tokens (
            app_id, external_user_id, endpoint, amount_cents, nonce, expires_at
        )
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (app_id, external_user_id, endpoint, nonce)
        DO UPDATE SET
            amount_cents = EXCLUDED.amount_cents,
            expires_at = EXCLUDED.expires_at
        RETURNING *
        "#
    )
    .bind(app_id)
    .bind(external_user_id)
    .bind(endpoint)
    .bind(amount_cents)
    .bind(nonce)
    .bind(expires_at)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(token)
}

pub async fn use_agent_token(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: Uuid,
    external_user_id: &str,
    token_id: Uuid,
    endpoint: &str,
) -> Result<Option<AgentPaymentToken>, BridgeError> {
    sqlx::query_as::<_, AgentPaymentToken>(
        r#"
        UPDATE pay.agent_payment_tokens
        SET used = true, used_at = NOW()
        WHERE id = $1
          AND app_id = $2
          AND external_user_id = $3
          AND endpoint = $4
          AND used = false
          AND expires_at > NOW()
        RETURNING *
        "#
    )
    .bind(token_id)
    .bind(app_id)
    .bind(external_user_id)
    .bind(endpoint)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))
}

pub async fn record_agent_transaction(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: Uuid,
    external_user_id: &str,
    request_type: &str,
    amount_cents: i32,
    charge_id: Option<&str>,
) -> Result<(), BridgeError> {
    sqlx::query(
        r#"
        INSERT INTO pay.agent_transactions (app_id, external_user_id, request_type, amount_cents, charge_id, status)
        VALUES ($1, $2, $3, $4, $5, 'completed')
        "#,
    )
    .bind(app_id)
    .bind(external_user_id)
    .bind(request_type)
    .bind(amount_cents)
    .bind(charge_id)
    .execute(&mut **tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;
    Ok(())
}

pub async fn charge_agent(
    pool: &PgPool,
    app_id: Uuid,
    external_user_id: &str,
    token_id: Uuid,
    endpoint: &str,
) -> Result<(i32, i32), BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;

    let token_opt = use_agent_token(&mut tx, app_id, external_user_id, token_id, endpoint).await?;
    let token = match token_opt {
        Some(token) => token,
        None => {
            let _ = tx.rollback().await;
            return Err(BridgeError::ValidationError(
                "Token is invalid, expired, already used, or does not belong to this user"
                    .into(),
            ));
        }
    };

    let credit_opt = sqlx::query_as::<_, AgentCredit>(
        r#"
        UPDATE pay.agent_credits
        SET balance_cents = balance_cents - $3,
            lifetime_spent_cents = lifetime_spent_cents + $3,
            updated_at = NOW()
        WHERE app_id = $1 AND external_user_id = $2 AND balance_cents >= $3
        RETURNING *
        "#
    )
    .bind(app_id)
    .bind(external_user_id)
    .bind(token.amount_cents)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    let credit = match credit_opt {
        Some(credit) => credit,
        None => {
            let _ = tx.rollback().await;
            return Err(BridgeError::ValidationError("Insufficient funds".to_string()));
        }
    };

    record_agent_transaction(
        &mut tx,
        app_id,
        external_user_id,
        &token.endpoint,
        -token.amount_cents,
        None,
    ).await?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
    
    Ok((credit.balance_cents, token.amount_cents))
}


