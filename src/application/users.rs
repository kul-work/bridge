use uuid::Uuid;

use crate::{
    error::BridgeError,
    ports::{ProviderConfigLookupRepository, UserRepository, UserSubscriptionCancellationSnapshot},
    services::provider_api,
};

pub struct AnonymizeUserInput<'a> {
    pub app_id: Uuid,
    pub external_user_id: &'a str,
    pub reason: Option<&'a str>,
}

pub struct AnonymizeUserResult {
    pub subscriptions_cancelled: i64,
    pub payments_anonymized: i64,
    pub new_anonymous_id: String,
}

pub async fn anonymize_user<R>(
    repo: &R,
    input: AnonymizeUserInput<'_>,
) -> Result<AnonymizeUserResult, BridgeError>
where
    R: UserRepository + ProviderConfigLookupRepository + ?Sized,
{
    let subscriptions = repo
        .list_user_subscriptions_to_cancel(input.app_id, input.external_user_id)
        .await?;

    for subscription in subscriptions {
        cancel_subscription_at_provider(repo, input.app_id, input.external_user_id, &subscription)
            .await;
    }

    let (subscriptions_cancelled, payments_anonymized, new_anonymous_id) = repo
        .anonymize_user_records(input.app_id, input.external_user_id, input.reason)
        .await?;

    Ok(AnonymizeUserResult {
        subscriptions_cancelled,
        payments_anonymized,
        new_anonymous_id,
    })
}

async fn cancel_subscription_at_provider<R>(
    repo: &R,
    app_id: Uuid,
    external_user_id: &str,
    subscription: &UserSubscriptionCancellationSnapshot,
) where
    R: ProviderConfigLookupRepository + ?Sized,
{
    if subscription.provider == "google_play" && subscription.purchase_token.is_none() {
        tracing::warn!(
            app_id = %app_id,
            external_user_id = external_user_id,
            subscription_id = subscription.subscription_id,
            provider = subscription.provider,
            "Skipping Google Play provider cancellation during user anonymization because purchase_token is missing"
        );
        return;
    }

    let provider_config = match repo.get_provider_config(app_id, &subscription.provider).await {
        Ok(provider_config) => provider_config,
        Err(e) => {
            tracing::warn!(
                app_id = %app_id,
                external_user_id = external_user_id,
                subscription_id = subscription.subscription_id,
                provider = subscription.provider,
                error = %e,
                "Failed to load provider config during user anonymization"
            );
            return;
        }
    };

    if let Err(e) = provider_api::cancel_subscription(
        &subscription.provider,
        &subscription.subscription_id,
        subscription.purchase_token.as_deref(),
        Some("scheduled"),
        &provider_config.config,
    )
    .await
    {
        tracing::warn!(
            app_id = %app_id,
            external_user_id = external_user_id,
            subscription_id = subscription.subscription_id,
            provider = subscription.provider,
            error = %e,
            "Failed to cancel subscription at provider during user anonymization"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use serde_json::json;

    use super::*;
    use crate::application::app_context::ProviderConfigSnapshot;

    struct FakeRepo {
        subscriptions: Vec<UserSubscriptionCancellationSnapshot>,
        provider_config_requests: Mutex<Vec<String>>,
        anonymized: Mutex<bool>,
    }

    #[async_trait]
    impl UserRepository for FakeRepo {
        async fn list_user_subscriptions_to_cancel(
            &self,
            _app_id: Uuid,
            _external_user_id: &str,
        ) -> Result<Vec<UserSubscriptionCancellationSnapshot>, BridgeError> {
            Ok(self.subscriptions.clone())
        }

        async fn anonymize_user_records(
            &self,
            _app_id: Uuid,
            _external_user_id: &str,
            _reason: Option<&str>,
        ) -> Result<(i64, i64, String), BridgeError> {
            *self.anonymized.lock().unwrap() = true;
            Ok((2, 3, "deleted_test".to_string()))
        }
    }

    #[async_trait]
    impl ProviderConfigLookupRepository for FakeRepo {
        async fn get_provider_config(
            &self,
            _app_id: Uuid,
            provider: &str,
        ) -> Result<ProviderConfigSnapshot, BridgeError> {
            self.provider_config_requests
                .lock()
                .unwrap()
                .push(provider.to_string());
            Ok(ProviderConfigSnapshot { config: json!({}) })
        }
    }

    #[tokio::test]
    async fn anonymize_continues_when_provider_cancellation_is_skipped_or_fails() {
        let repo = FakeRepo {
            subscriptions: vec![
                UserSubscriptionCancellationSnapshot {
                    subscription_id: "legacy-sub".to_string(),
                    provider: "legacy".to_string(),
                    purchase_token: None,
                },
                UserSubscriptionCancellationSnapshot {
                    subscription_id: "google-sub".to_string(),
                    provider: "google_play".to_string(),
                    purchase_token: None,
                },
                UserSubscriptionCancellationSnapshot {
                    subscription_id: "unsupported-sub".to_string(),
                    provider: "unsupported".to_string(),
                    purchase_token: None,
                },
            ],
            provider_config_requests: Mutex::new(Vec::new()),
            anonymized: Mutex::new(false),
        };

        let result = anonymize_user(
            &repo,
            AnonymizeUserInput {
                app_id: Uuid::new_v4(),
                external_user_id: "user-1",
                reason: Some("user_requested_deletion"),
            },
        )
        .await
        .unwrap();

        assert_eq!(result.subscriptions_cancelled, 2);
        assert_eq!(result.payments_anonymized, 3);
        assert_eq!(result.new_anonymous_id, "deleted_test");
        assert!(*repo.anonymized.lock().unwrap());
        assert_eq!(
            *repo.provider_config_requests.lock().unwrap(),
            vec!["legacy".to_string(), "unsupported".to_string()]
        );
    }
}
