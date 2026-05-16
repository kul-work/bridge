use super::*;
use super::normalize::normalize_event_type;

#[test]
fn test_normalize_google_play_events() {
    assert_eq!(
        normalize_event_type("google_play", "SUBSCRIPTION_PURCHASED"),
        "subscription.activated"
    );
    assert_eq!(
        normalize_event_type("google_play", "SUBSCRIPTION_RENEWED"),
        "subscription.renewed"
    );
    assert_eq!(
        normalize_event_type("google_play", "SUBSCRIPTION_CANCELED"),
        "subscription.cancelled"
    );
    assert_eq!(
        normalize_event_type("google_play", "SUBSCRIPTION_ON_HOLD"),
        "subscription.on_hold"
    );
    assert_eq!(
        normalize_event_type("google_play", "SUBSCRIPTION_CANCELLATION_SCHEDULED"),
        "subscription.cancellation_scheduled"
    );
    assert_eq!(
        normalize_event_type("google_play", "ONE_TIME_PRODUCT_REFUNDED"),
        "purchase.one_time_refunded"
    );
}

#[test]
fn test_normalize_creem_events() {
    assert_eq!(
        normalize_event_type("creem", "subscription.created"),
        "subscription.created"
    );
    assert_eq!(
        normalize_event_type("creem", "subscription.active"),
        "subscription.activated"
    );
    assert_eq!(
        normalize_event_type("creem", "subscription.trialing"),
        "subscription.trial_started"
    );
    assert_eq!(
        normalize_event_type("creem", "subscription.cancelled"),
        "subscription.cancelled"
    );
    assert_eq!(
        normalize_event_type("creem", "refund.created"),
        "payment.refunded"
    );
}

#[test]
fn test_normalize_creem_checkout_completed_recurring_to_subscription_created() {
    let payload = serde_json::json!({
        "eventType": "checkout.completed",
        "object": {
            "billing_type": "recurring",
            "subscription": {
                "id": "sub_001"
            }
        }
    });

    assert_eq!(
        normalize_event_type_with_payload("creem", "checkout.completed", Some(&payload)),
        "subscription.created"
    );
}

#[test]
fn test_normalize_creem_checkout_completed_one_time_to_purchase_one_time() {
    let payload = serde_json::json!({
        "eventType": "checkout.completed",
        "object": {
            "billing_type": "one_time",
            "checkout_id": "co_001"
        }
    });

    assert_eq!(
        normalize_event_type_with_payload("creem", "checkout.completed", Some(&payload)),
        "purchase.one_time"
    );
}

#[test]
fn test_normalize_creem_refund_created_one_time_to_purchase_one_time_refunded() {
    let payload = serde_json::json!({
        "eventType": "refund.created",
        "object": {
            "id": "refund_001",
            "billing_type": "one_time",
            "order_id": "order_001"
        }
    });

    assert_eq!(
        normalize_event_type_with_payload("creem", "refund.created", Some(&payload)),
        "purchase.one_time_refunded"
    );
}

#[test]
fn test_normalize_creem_refund_created_subscription_stays_payment_refunded() {
    let payload = serde_json::json!({
        "eventType": "refund.created",
        "object": {
            "id": "refund_001",
            "billing_type": "recurring",
            "subscription_id": "sub_001",
            "order_id": "order_001"
        }
    });

    assert_eq!(
        normalize_event_type_with_payload("creem", "refund.created", Some(&payload)),
        "payment.refunded"
    );
}

#[test]
fn test_creem_field_extraction_subscription_active() {
    let payload = serde_json::json!({
        "id": "evt_123",
        "eventType": "subscription.active",
        "createdAt": "2026-04-20T10:00:00Z",
        "object": {
            "id": "sub_456",
            "subscription_id": "sub_789",
            "product_id": "prod_premium",
            "status": "paid",
            "amount": 9999,
            "metadata": {
                "user_id": "user_ext_001"
            }
        }
    });

    let webhook = WebhookProviderSnapshot {
        provider: "creem".to_string(),
        provider_webhook_id: "wh_123".to_string(),
        event_type: "subscription.active".to_string(),
        subscription_id: Some("sub_789".to_string()),
        purchase_token: None,
        payload,
        processed: false,
        timestamp_epoch_ms: Some(1713607200000),
        suppressed: false,
        suppressed_reason: None,
    };

    let fields = extract_webhook_fields(&webhook);
    assert_eq!(fields.subscription_id, Some("sub_789".to_string()));
    assert_eq!(fields.product_id, Some("prod_premium".to_string()));
    assert_eq!(fields.status, Some("paid".to_string()));
    assert_eq!(fields.amount_cents, Some(9999));
}

#[test]
fn test_creem_field_extraction_checkout_completed_recurring() {
    let payload = serde_json::json!({
        "id": "evt_co_123",
        "eventType": "checkout.completed",
        "createdAt": "2026-04-20T10:00:00Z",
        "object": {
            "id": "checkout_abc",
            "product_id": "prod_monthly",
            "billing_type": "recurring",
            "amount": 4999,
            "last_transaction_id": "txn_001",
            "subscription": {
                "id": "sub_new_456",
                "status": "paid",
                "current_period_end_date": "2026-05-20T10:00:00Z",
                "metadata": {
                    "user_id": "user_ext_002"
                }
            }
        }
    });

    let webhook = WebhookProviderSnapshot {
        provider: "creem".to_string(),
        provider_webhook_id: "wh_456".to_string(),
        event_type: "checkout.completed".to_string(),
        subscription_id: Some("sub_new_456".to_string()),
        purchase_token: None,
        payload,
        processed: false,
        timestamp_epoch_ms: Some(1713607200000),
        suppressed: false,
        suppressed_reason: None,
    };

    let fields = extract_webhook_fields(&webhook);
    assert_eq!(fields.subscription_id, Some("sub_new_456".to_string()));
    assert_eq!(fields.product_id, Some("prod_monthly".to_string()));
    assert_eq!(fields.status, Some("paid".to_string()));
    assert_eq!(fields.current_period_end, Some("2026-05-20T10:00:00Z".to_string()));
}

#[test]
fn test_creem_field_extraction_checkout_completed_one_time() {
    let payload = serde_json::json!({
        "id": "evt_co_otp_123",
        "eventType": "checkout.completed",
        "createdAt": "2026-04-20T10:00:00Z",
        "object": {
            "id": "checkout_otp_001",
            "billing_type": "one_time",
            "product_id": "prod_lifetime",
            "checkout_id": "co_otp_001",
            "amount": 9999
        }
    });

    let webhook = WebhookProviderSnapshot {
        provider: "creem".to_string(),
        provider_webhook_id: "wh_otp_001".to_string(),
        event_type: "checkout.completed".to_string(),
        subscription_id: None,
        purchase_token: None,
        payload,
        processed: false,
        timestamp_epoch_ms: Some(1713607200000),
        suppressed: false,
        suppressed_reason: None,
    };

    let fields = extract_webhook_fields(&webhook);
    assert_eq!(fields.subscription_id, None);
    assert_eq!(fields.product_id, Some("prod_lifetime".to_string()));
    assert_eq!(fields.purchase_token, Some("co_otp_001".to_string()));
    assert_eq!(fields.amount_cents, Some(9999));
}

#[test]
fn test_google_play_subscription_field_extraction_does_not_use_purchase_token_as_transaction_id() {
    let payload = serde_json::json!({
        "subscriptionNotification": {
            "subscriptionId": "hiha_monthly",
            "purchaseToken": "shared_purchase_token",
            "notificationType": 2
        }
    });

    let webhook = WebhookProviderSnapshot {
        provider: "google_play".to_string(),
        provider_webhook_id: "19071854013335023".to_string(),
        event_type: "SUBSCRIPTION_RENEWED".to_string(),
        subscription_id: Some("hiha_monthly".to_string()),
        purchase_token: Some("shared_purchase_token".to_string()),
        payload,
        processed: false,
        timestamp_epoch_ms: Some(1778943638046),
        suppressed: false,
        suppressed_reason: None,
    };

    let fields = extract_webhook_fields(&webhook);

    assert_eq!(fields.purchase_token, Some("shared_purchase_token".to_string()));
    assert_eq!(fields.provider_transaction_id, None);
}

#[test]
fn test_google_subscription_expiry_prefers_line_item_expiry() {
    let resource = crate::services::google_play::models::SubscriptionPurchaseV2 {
        expiry_time: Some("2026-05-16T14:30:34Z".to_string()),
        line_items: vec![crate::services::google_play::models::SubscriptionLineItem {
            product_id: "hiha_monthly".to_string(),
            expiry_time: Some("2026-05-16T14:35:34Z".to_string()),
            auto_renewing_plan: None,
            offer_details: None,
        }],
        ..Default::default()
    };

    assert_eq!(
        google_subscription_expiry_time(&resource),
        Some("2026-05-16T14:35:34Z".to_string())
    );
}

#[test]
fn test_google_subscription_transaction_id_prefers_latest_order_id() {
    let resource = crate::services::google_play::models::SubscriptionPurchaseV2 {
        latest_order_id: Some("GPA.1234-5678-9012-34567".to_string()),
        ..Default::default()
    };

    assert_eq!(
        google_subscription_transaction_id(&resource, "19071854013335023"),
        "GPA.1234-5678-9012-34567"
    );
}

#[test]
fn test_google_subscription_transaction_id_falls_back_to_rtdn_message_id() {
    let resource = crate::services::google_play::models::SubscriptionPurchaseV2::default();

    assert_eq!(
        google_subscription_transaction_id(&resource, "19071854013335023"),
        "google_play_rtdn:19071854013335023"
    );
}

#[test]
fn test_google_subscription_recurring_amount_uses_integer_cents() {
    let resource = crate::services::google_play::models::SubscriptionPurchaseV2 {
        line_items: vec![crate::services::google_play::models::SubscriptionLineItem {
            product_id: "hiha_monthly".to_string(),
            expiry_time: None,
            auto_renewing_plan: Some(crate::services::google_play::models::AutoRenewingPlan {
                auto_renew_enabled: Some(true),
                recurring_price: Some(crate::services::google_play::models::Money {
                    currency_code: Some("USD".to_string()),
                    units: Some("5".to_string()),
                    nanos: Some(490_000_000),
                }),
            }),
            offer_details: None,
        }],
        ..Default::default()
    };

    assert_eq!(google_subscription_recurring_amount_cents(&resource), Some(549));
}

#[test]
fn test_creem_field_extraction_refund_with_amount_fallback() {
    let payload = serde_json::json!({
        "id": "evt_ref_123",
        "eventType": "refund.created",
        "createdAt": "2026-04-20T10:00:00Z",
        "object": {
            "id": "refund_789",
            "order_id": "order_original",
            "subscription_id": "sub_refunded",
            "last_transaction": {
                "amount": 2999
            }
        }
    });

    let webhook = WebhookProviderSnapshot {
        provider: "creem".to_string(),
        provider_webhook_id: "wh_789".to_string(),
        event_type: "refund.created".to_string(),
        subscription_id: Some("sub_refunded".to_string()),
        purchase_token: None,
        payload,
        processed: false,
        timestamp_epoch_ms: Some(1713607200000),
        suppressed: false,
        suppressed_reason: None,
    };

    let fields = extract_webhook_fields(&webhook);
    assert_eq!(fields.subscription_id, Some("sub_refunded".to_string()));
    assert_eq!(fields.purchase_token, Some("order_original".to_string()));
    assert_eq!(fields.amount_cents, Some(2999));
}

#[test]
fn test_creem_field_extraction_one_time_refund() {
    let payload = serde_json::json!({
        "id": "evt_ref_otp_123",
        "eventType": "refund.created",
        "createdAt": "2026-04-20T10:00:00Z",
        "object": {
            "id": "refund_otp_789",
            "billing_type": "one_time",
            "order_id": "order_original",
            "product_id": "prod_lifetime",
            "last_transaction": {
                "amount": 9999
            }
        }
    });

    let webhook = WebhookProviderSnapshot {
        provider: "creem".to_string(),
        provider_webhook_id: "wh_otp_refund_789".to_string(),
        event_type: "refund.created".to_string(),
        subscription_id: None,
        purchase_token: None,
        payload,
        processed: false,
        timestamp_epoch_ms: Some(1713607200000),
        suppressed: false,
        suppressed_reason: None,
    };

    let fields = extract_webhook_fields(&webhook);
    assert_eq!(fields.subscription_id, None);
    assert_eq!(fields.product_id, Some("prod_lifetime".to_string()));
    assert_eq!(fields.purchase_token, Some("order_original".to_string()));
    assert_eq!(fields.amount_cents, Some(9999));
}

#[test]
fn test_creem_metadata_user_id_from_checkout_path() {
    let payload = serde_json::json!({
        "id": "evt_123",
        "eventType": "checkout.completed",
        "createdAt": "2026-04-20T10:00:00Z",
        "object": {
            "id": "checkout_123",
            "checkout": {
                "metadata": {
                    "user_id": "user_from_checkout"
                }
            }
        }
    });

    assert_eq!(
        extract_metadata_user_id(&payload).as_deref(),
        Some("user_from_checkout")
    );
}

#[test]
fn test_normalize_status() {
    assert_eq!(normalize_status(Some("Trialing")), "trial");
    assert_eq!(normalize_status(Some(" PAID ")), "active");
    assert_eq!(normalize_status(Some("canceled")), "cancelled");
    assert_eq!(normalize_status(None), "pending");
}

#[test]
fn test_status_to_canonical_event() {
    assert_eq!(status_to_canonical_event("active"), Some("subscription.activated".to_string()));
    assert_eq!(status_to_canonical_event("trial"), Some("subscription.activated".to_string()));
    assert_eq!(status_to_canonical_event("expired"), Some("subscription.expired".to_string()));
    assert_eq!(status_to_canonical_event("unknown"), None);
}

#[test]
fn test_callback_status_for_pause_lifecycle_events() {
    assert_eq!(
        callback_status_for_event("subscription.pause_scheduled"),
        Some("active".to_string())
    );
    assert_eq!(
        callback_status_for_event("subscription.resumed"),
        Some("active".to_string())
    );
    assert_eq!(
        callback_status_for_event("subscription.paused"),
        Some("paused".to_string())
    );
    assert_eq!(callback_status_for_event("subscription.updated"), None);
}

#[test]
fn test_canonical_payload_serializes_google_lifecycle_fields() {
    let payload = CanonicalWebhookPayload {
        event_id: "evt_1".to_string(),
        event_type: "subscription.pause_scheduled".to_string(),
        timestamp: "2026-05-07T12:00:00Z".to_string(),
        timestamp_epoch_ms: 1778155200000,
        app_slug: "hiha".to_string(),
        product_id: None,
        subscription_id: Some("sub_1".to_string()),
        external_user_id: Some("user_1".to_string()),
        amount_cents: None,
        new_price_cents: None,
        auto_renewing: Some(true),
        purchase_token: Some("token_1".to_string()),
        current_period_end: None,
        status: Some("active".to_string()),
        provider: "google_play".to_string(),
        provider_event_id: "provider_evt_1".to_string(),
        previous_status: None,
        corrected_status: None,
        reconciliation_source: None,
        revocation_reason: None,
        cancellation_mode: None,
        google_price_step_up_consent_deadline: Some(1778760000000),
        google_pause_scheduled_at: Some(1778846400000),
        google_deferred_until: Some(1781438400000),
    };

    let value = serde_json::to_value(payload).unwrap();

    assert_eq!(value["google_price_step_up_consent_deadline"], 1778760000000i64);
    assert_eq!(value["google_pause_scheduled_at"], 1778846400000i64);
    assert_eq!(value["google_deferred_until"], 1781438400000i64);
}

#[test]
fn test_mock_google_play_renewal_period_end_extends_existing_period() {
    let existing = chrono::DateTime::parse_from_rfc3339("2026-05-10T18:44:10Z")
        .unwrap()
        .with_timezone(&chrono::Utc);

    assert_eq!(
        mock_google_play_renewal_period_end(Some(existing)),
        chrono::DateTime::parse_from_rfc3339("2026-06-09T18:44:10Z")
            .unwrap()
            .with_timezone(&chrono::Utc)
    );
}

#[test]
fn test_extract_metadata_user_id_supports_nested_paths() {
    let payload = serde_json::json!({
        "event": {
            "data": {
                "metadata": {
                    "external_user_id": "nested-user"
                }
            }
        }
    });

    assert_eq!(extract_metadata_user_id(&payload).as_deref(), Some("nested-user"));
}
