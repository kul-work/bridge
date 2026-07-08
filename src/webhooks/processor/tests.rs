use super::*;
use crate::webhooks::provider_adapter::{NormalizedProviderStatus, ProviderWebhookAdapter};

#[test]
fn test_normalize_google_play_events() {
    let adapter = ProviderWebhookAdapter::GooglePlay;
    let payload = serde_json::Value::Null;
    assert_eq!(
        adapter.canonical_event_type("SUBSCRIPTION_PURCHASED", &payload),
        "subscription.activated"
    );
    assert_eq!(
        adapter.canonical_event_type("SUBSCRIPTION_RENEWED", &payload),
        "subscription.renewed"
    );
    assert_eq!(
        adapter.canonical_event_type("SUBSCRIPTION_CANCELED", &payload),
        "subscription.cancelled"
    );
    assert_eq!(
        adapter.canonical_event_type("SUBSCRIPTION_ON_HOLD", &payload),
        "subscription.on_hold"
    );
    assert_eq!(
        adapter.canonical_event_type("SUBSCRIPTION_CANCELLATION_SCHEDULED", &payload),
        "subscription.cancellation_scheduled"
    );
    assert_eq!(
        adapter.canonical_event_type("ONE_TIME_PRODUCT_REFUNDED", &payload),
        "purchase.one_time_refunded"
    );
}

#[test]
fn test_normalize_creem_events() {
    let adapter = ProviderWebhookAdapter::Creem;
    let payload = serde_json::Value::Null;
    assert_eq!(
        adapter.canonical_event_type("subscription.created", &payload),
        "subscription.created"
    );
    assert_eq!(
        adapter.canonical_event_type("subscription.active", &payload),
        "subscription.activated"
    );
    assert_eq!(
        adapter.canonical_event_type("subscription.trialing", &payload),
        "subscription.trial_started"
    );
    assert_eq!(
        adapter.canonical_event_type("subscription.cancelled", &payload),
        "subscription.cancelled"
    );
    assert_eq!(
        adapter.canonical_event_type("refund.created", &payload),
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
        ProviderWebhookAdapter::Creem.canonical_event_type("checkout.completed", &payload),
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
        ProviderWebhookAdapter::Creem.canonical_event_type("checkout.completed", &payload),
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
        ProviderWebhookAdapter::Creem.canonical_event_type("refund.created", &payload),
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
        ProviderWebhookAdapter::Creem.canonical_event_type("refund.created", &payload),
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

    let fields = extract_webhook_fields(&webhook).unwrap();
    assert_eq!(fields.subscription_id, Some("sub_789".to_string()));
    assert_eq!(fields.product_id, Some("prod_premium".to_string()));
    assert_eq!(fields.status, Some("paid".to_string()));
    assert_eq!(fields.amount_cents, Some(9999));
    assert_eq!(fields.provider_transaction_id, None);
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

    let fields = extract_webhook_fields(&webhook).unwrap();
    assert_eq!(fields.subscription_id, Some("sub_new_456".to_string()));
    assert_eq!(fields.product_id, Some("prod_monthly".to_string()));
    assert_eq!(fields.status, Some("paid".to_string()));
    assert_eq!(fields.current_period_end, Some("2026-05-20T10:00:00Z".to_string()));
    assert_eq!(fields.provider_transaction_id, Some("txn_001".to_string()));
}

#[test]
fn test_creem_recurring_checkout_without_last_transaction_is_state_only() {
    let payload = serde_json::json!({
        "id": "evt_checkout_recurring_002",
        "eventType": "checkout.completed",
        "object": {
            "id": "ch_4fXOCBlh6QuOjqdyOhfkWf",
            "billing_type": "recurring",
            "order": {
                "id": "ord_5fBLcubXQKPHJHBA7KbuVr",
                "amount": 450
            },
            "product": {
                "id": "prod_monthly"
            },
            "subscription": {
                "id": "sub_3UJmiDyIzY1uQJsH4a2jpQ",
                "status": "active",
                "current_period_end_date": "2026-06-26T14:51:01Z"
            }
        }
    });

    let webhook = WebhookProviderSnapshot {
        provider: "creem".to_string(),
        provider_webhook_id: "evt_checkout_recurring_002".to_string(),
        event_type: "checkout.completed".to_string(),
        subscription_id: Some("sub_3UJmiDyIzY1uQJsH4a2jpQ".to_string()),
        purchase_token: Some("ch_4fXOCBlh6QuOjqdyOhfkWf".to_string()),
        payload,
        processed: false,
        timestamp_epoch_ms: None,
        suppressed: false,
        suppressed_reason: None,
    };

    let fields = extract_webhook_fields(&webhook).unwrap();
    assert_eq!(fields.subscription_id, Some("sub_3UJmiDyIzY1uQJsH4a2jpQ".to_string()));
    assert_eq!(fields.amount_cents, Some(450));
    assert_eq!(fields.provider_transaction_id, None);
}

#[test]
fn test_creem_subscription_paid_uses_last_transaction_id() {
    let payload = serde_json::json!({
        "id": "evt_paid_001",
        "eventType": "subscription.paid",
        "object": {
            "id": "sub_3UJmiDyIzY1uQJsH4a2jpQ",
            "last_transaction_id": "tran_5kxqVwXF85IN29I42TDHon",
            "last_transaction": {
                "id": "tran_5kxqVwXF85IN29I42TDHon",
                "amount": 450
            },
            "product": {
                "id": "prod_monthly",
                "currency": "USD"
            },
            "status": "active"
        }
    });

    let webhook = WebhookProviderSnapshot {
        provider: "creem".to_string(),
        provider_webhook_id: "evt_paid_001".to_string(),
        event_type: "subscription.paid".to_string(),
        subscription_id: Some("sub_3UJmiDyIzY1uQJsH4a2jpQ".to_string()),
        purchase_token: None,
        payload,
        processed: false,
        timestamp_epoch_ms: None,
        suppressed: false,
        suppressed_reason: None,
    };

    let fields = extract_webhook_fields(&webhook).unwrap();
    assert_eq!(fields.subscription_id, Some("sub_3UJmiDyIzY1uQJsH4a2jpQ".to_string()));
    assert_eq!(fields.amount_cents, Some(450));
    assert_eq!(fields.currency, Some("USD".to_string()));
    assert_eq!(fields.provider_transaction_id, Some("tran_5kxqVwXF85IN29I42TDHon".to_string()));
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

    let fields = extract_webhook_fields(&webhook).unwrap();
    assert_eq!(fields.subscription_id, None);
    assert_eq!(fields.product_id, Some("prod_lifetime".to_string()));
    assert_eq!(fields.purchase_token, Some("co_otp_001".to_string()));
    assert_eq!(fields.amount_cents, Some(9999));
    assert_eq!(fields.provider_transaction_id, Some("co_otp_001".to_string()));
}

#[test]
fn test_google_play_subscription_field_extraction_does_not_use_purchase_token_as_transaction_id() {
    let payload = serde_json::json!({
        "subscriptionNotification": {
            "subscriptionId": "premium_monthly",
            "purchaseToken": "shared_purchase_token",
            "notificationType": 2
        }
    });

    let webhook = WebhookProviderSnapshot {
        provider: "google_play".to_string(),
        provider_webhook_id: "19071854013335023".to_string(),
        event_type: "SUBSCRIPTION_RENEWED".to_string(),
        subscription_id: Some("premium_monthly".to_string()),
        purchase_token: Some("shared_purchase_token".to_string()),
        payload,
        processed: false,
        timestamp_epoch_ms: Some(1778943638046),
        suppressed: false,
        suppressed_reason: None,
    };

    let fields = extract_webhook_fields(&webhook).unwrap();

    assert_eq!(fields.purchase_token, Some("shared_purchase_token".to_string()));
    assert_eq!(fields.provider_transaction_id, None);
}

#[test]
fn test_normalize_google_play_voided_otp_to_one_time_refund() {
    let payload = serde_json::json!({
        "voidedPurchaseNotification": {
            "purchaseToken": "otp_purchase_token",
            "orderId": "GPA.3346-7932-0960-90782",
            "productType": 2
        }
    });

    assert_eq!(
        ProviderWebhookAdapter::GooglePlay.canonical_event_type("VOIDED_PURCHASE", &payload),
        "purchase.one_time_refunded"
    );
}

#[test]
fn test_google_play_one_time_field_extraction_keeps_token_out_of_transaction_id() {
    let payload = serde_json::json!({
        "oneTimeProductNotification": {
            "productId": "hiha_one_time",
            "purchaseToken": "otp_purchase_token",
            "notificationType": 1
        }
    });

    let webhook = WebhookProviderSnapshot {
        provider: "google_play".to_string(),
        provider_webhook_id: "19082919261635860".to_string(),
        event_type: "ONE_TIME_PRODUCT_PURCHASED".to_string(),
        subscription_id: None,
        purchase_token: Some("otp_purchase_token".to_string()),
        payload,
        processed: false,
        timestamp_epoch_ms: Some(1779043946000),
        suppressed: false,
        suppressed_reason: None,
    };

    let fields = extract_webhook_fields(&webhook).unwrap();

    assert_eq!(fields.subscription_id, None);
    assert_eq!(fields.product_id, Some("hiha_one_time".to_string()));
    assert_eq!(fields.purchase_token, Some("otp_purchase_token".to_string()));
    assert_eq!(fields.provider_transaction_id, None);
}

#[test]
fn test_google_play_voided_otp_field_extraction_uses_order_id_not_token() {
    let payload = serde_json::json!({
        "voidedPurchaseNotification": {
            "purchaseToken": "otp_purchase_token",
            "orderId": "GPA.3346-7932-0960-90782",
            "productType": 2
        }
    });

    let webhook = WebhookProviderSnapshot {
        provider: "google_play".to_string(),
        provider_webhook_id: "19545006170252135".to_string(),
        event_type: "VOIDED_PURCHASE".to_string(),
        subscription_id: None,
        purchase_token: Some("otp_purchase_token".to_string()),
        payload,
        processed: false,
        timestamp_epoch_ms: Some(1779044094000),
        suppressed: false,
        suppressed_reason: None,
    };

    let fields = extract_webhook_fields(&webhook).unwrap();

    assert_eq!(fields.subscription_id, None);
    assert_eq!(fields.purchase_token, Some("otp_purchase_token".to_string()));
    assert_eq!(fields.provider_transaction_id, Some("GPA.3346-7932-0960-90782".to_string()));
}

#[test]
fn test_google_subscription_expiry_prefers_line_item_expiry() {
    let resource = crate::services::google_play::models::SubscriptionPurchaseV2 {
        expiry_time: Some("2026-05-16T14:30:34Z".to_string()),
        line_items: vec![crate::services::google_play::models::SubscriptionLineItem {
            product_id: "premium_monthly".to_string(),
            expiry_time: Some("2026-05-16T14:35:34Z".to_string()),
            latest_successful_order_id: None,
            auto_renewing_plan: None,
            offer_details: None,
            offer_phase: None,
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
        google_subscription_transaction_id(&resource),
        Some("GPA.1234-5678-9012-34567".to_string())
    );
}

#[test]
fn test_google_subscription_transaction_id_does_not_fall_back_to_rtdn_message_id() {
    let resource = crate::services::google_play::models::SubscriptionPurchaseV2::default();

    assert_eq!(google_subscription_transaction_id(&resource), None);
}

#[test]
fn test_google_subscription_recurring_amount_uses_integer_cents() {
    let resource = crate::services::google_play::models::SubscriptionPurchaseV2 {
        line_items: vec![crate::services::google_play::models::SubscriptionLineItem {
            product_id: "premium_monthly".to_string(),
            expiry_time: None,
            latest_successful_order_id: None,
            auto_renewing_plan: Some(crate::services::google_play::models::AutoRenewingPlan {
                auto_renew_enabled: Some(true),
                recurring_price: Some(crate::services::google_play::models::Money {
                    currency_code: Some("RON".to_string()),
                    units: Some("5".to_string()),
                    nanos: Some(490_000_000),
                }),
                price_change_details: None,
            }),
            offer_details: None,
            offer_phase: None,
        }],
        ..Default::default()
    };

    assert_eq!(google_subscription_current_amount_cents(&resource), Some(549));
    assert_eq!(google_subscription_recurring_currency(&resource), Some("RON".to_string()));
}

#[test]
fn test_google_subscription_recurring_amount_uses_zero_for_free_trial_phase() {
    let resource = crate::services::google_play::models::SubscriptionPurchaseV2 {
        line_items: vec![crate::services::google_play::models::SubscriptionLineItem {
            product_id: "premium_monthly".to_string(),
            expiry_time: None,
            latest_successful_order_id: Some("GPA.3393-5701-2992-95414".to_string()),
            auto_renewing_plan: Some(crate::services::google_play::models::AutoRenewingPlan {
                auto_renew_enabled: Some(true),
                recurring_price: Some(crate::services::google_play::models::Money {
                    currency_code: Some("RON".to_string()),
                    units: Some("5".to_string()),
                    nanos: Some(490_000_000),
                }),
                price_change_details: None,
            }),
            offer_details: None,
            offer_phase: Some(crate::services::google_play::models::OfferPhase {
                free_trial: Some(serde_json::json!({})),
                base_price: None,
            }),
        }],
        ..Default::default()
    };

    assert_eq!(google_subscription_current_amount_cents(&resource), Some(0));
    assert_eq!(google_subscription_recurring_currency(&resource), Some("RON".to_string()));
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

    let fields = extract_webhook_fields(&webhook).unwrap();
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

    let fields = extract_webhook_fields(&webhook).unwrap();
    assert_eq!(fields.subscription_id, None);
    assert_eq!(fields.product_id, Some("prod_lifetime".to_string()));
    assert_eq!(fields.purchase_token, Some("order_original".to_string()));
    assert_eq!(fields.amount_cents, Some(9999));
}

#[test]
fn test_creem_field_extraction_refund_created_real_schema() {
    // Real Creem refund.created payload shape (from docs.creem.io/code/webhooks):
    // currency lives at object.refund_currency / object.transaction.currency,
    // amount lives at object.refund_amount / object.transaction.amount_paid.
    // object.product.currency is absent — the pre-fix single-path extraction
    // returned currency=null and amount_cents=null for this event.
    let payload = serde_json::json!({
        "id": "evt_refund_real",
        "eventType": "refund.created",
        "created_at": 1728734351631_i64,
        "object": {
            "id": "ref_3DB9NQFvk18TJwSqd0N6bd",
            "object": "refund",
            "status": "succeeded",
            "refund_amount": 1210,
            "refund_currency": "EUR",
            "reason": "requested_by_customer",
            "subscription": { "id": "sub_6pC2lNB6joCRQIZ1aMrTpi" },
            "transaction": {
                "id": "tran_5yMaWzAl3jxuGJMCOrYWwk",
                "object": "transaction",
                "amount": 1000,
                "amount_paid": 1210,
                "currency": "EUR",
                "status": "refunded",
                "order": "ord_4aDwWXjMLpes4Kj4XqNnUA",
                "subscription": "sub_6pC2lNB6joCRQIZ1aMrTpi"
            }
        }
    });

    let webhook = WebhookProviderSnapshot {
        provider: "creem".to_string(),
        provider_webhook_id: "wh_refund_real".to_string(),
        event_type: "refund.created".to_string(),
        subscription_id: Some("sub_6pC2lNB6joCRQIZ1aMrTpi".to_string()),
        purchase_token: None,
        payload,
        processed: false,
        timestamp_epoch_ms: Some(1728734351631),
        suppressed: false,
        suppressed_reason: None,
    };

    let fields = extract_webhook_fields(&webhook).unwrap();
    assert_eq!(fields.amount_cents, Some(1210), "refund_amount must be extracted");
    assert_eq!(fields.currency, Some("EUR".to_string()), "refund_currency must be extracted");
}

#[test]
fn test_creem_field_extraction_dispute_created_real_schema() {
    // Real Creem dispute.created payload shape (from docs.creem.io/code/webhooks):
    // currency lives at object.currency / object.transaction.currency,
    // amount lives at object.amount. object.product.currency is absent.
    let payload = serde_json::json!({
        "id": "evt_dispute_real",
        "eventType": "dispute.created",
        "created_at": 1750941264812_i64,
        "object": {
            "id": "disp_6vSsOdTANP5PhOzuDlUuXE",
            "object": "dispute",
            "amount": 1331,
            "currency": "EUR",
            "transaction": {
                "id": "tran_4Dk8CxWFdceRUQgMFhCCXX",
                "object": "transaction",
                "amount": 1100,
                "amount_paid": 1331,
                "currency": "EUR",
                "status": "chargeback",
                "order": "ord_57bf8042UmG8fFypxZrfnj",
                "subscription": "sub_5sD6zM482uwOaEoyEUDDJs"
            },
            "subscription": { "id": "sub_5sD6zM482uwOaEoyEUDDJs" },
            "order": {
                "id": "ord_57bf8042UmG8fFypxZrfnj",
                "amount": 1100,
                "currency": "EUR",
                "status": "paid"
            }
        }
    });

    let webhook = WebhookProviderSnapshot {
        provider: "creem".to_string(),
        provider_webhook_id: "wh_dispute_real".to_string(),
        event_type: "dispute.created".to_string(),
        subscription_id: Some("sub_5sD6zM482uwOaEoyEUDDJs".to_string()),
        purchase_token: None,
        payload,
        processed: false,
        timestamp_epoch_ms: Some(1750941264812),
        suppressed: false,
        suppressed_reason: None,
    };

    let fields = extract_webhook_fields(&webhook).unwrap();
    assert_eq!(fields.amount_cents, Some(1331), "top-level amount must be extracted");
    assert_eq!(fields.currency, Some("EUR".to_string()), "top-level currency must be extracted");
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
    let gp = ProviderWebhookAdapter::GooglePlay;
    let cr = ProviderWebhookAdapter::Creem;

    for adapter in [&gp, &cr] {
        assert_eq!(adapter.normalize_status(Some("trialing")), NormalizedProviderStatus::Known("trial".to_string()));
        assert_eq!(adapter.normalize_status(Some("trial")), NormalizedProviderStatus::Known("trial".to_string()));
        assert_eq!(adapter.normalize_status(Some("active")), NormalizedProviderStatus::Known("active".to_string()));
        assert_eq!(adapter.normalize_status(Some("paid")), NormalizedProviderStatus::Known("active".to_string()));
        assert_eq!(adapter.normalize_status(Some("completed")), NormalizedProviderStatus::Known("active".to_string()));
        assert_eq!(adapter.normalize_status(Some("success")), NormalizedProviderStatus::Known("active".to_string()));
        assert_eq!(adapter.normalize_status(Some("past_due")), NormalizedProviderStatus::Known("past_due".to_string()));
        assert_eq!(adapter.normalize_status(Some("grace_period")), NormalizedProviderStatus::Known("past_due".to_string()));
        assert_eq!(adapter.normalize_status(Some("cancelled")), NormalizedProviderStatus::Known("cancelled".to_string()));
        assert_eq!(adapter.normalize_status(Some("canceled")), NormalizedProviderStatus::Known("cancelled".to_string()));
        assert_eq!(adapter.normalize_status(Some("expired")), NormalizedProviderStatus::Known("expired".to_string()));
        assert_eq!(adapter.normalize_status(Some("on_hold")), NormalizedProviderStatus::Known("on_hold".to_string()));
        assert_eq!(adapter.normalize_status(Some("on-hold")), NormalizedProviderStatus::Known("on_hold".to_string()));
        assert_eq!(adapter.normalize_status(Some("paused")), NormalizedProviderStatus::Known("paused".to_string()));
        assert_eq!(adapter.normalize_status(Some("revoked")), NormalizedProviderStatus::Known("revoked".to_string()));
        assert_eq!(adapter.normalize_status(Some("pending")), NormalizedProviderStatus::Known("pending".to_string()));
        assert_eq!(adapter.normalize_status(Some("unpaid")), NormalizedProviderStatus::Unknown("unpaid".to_string()));
        assert_eq!(adapter.normalize_status(Some("unknown_status")), NormalizedProviderStatus::Unknown("unknown_status".to_string()));
        assert_eq!(adapter.normalize_status(None), NormalizedProviderStatus::Missing);
    }
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
        google_pending_price_change_new_price_cents: None,
        google_pending_price_change_currency: None,
        google_pending_price_change_mode: None,
        google_pending_price_change_state: None,
        google_pending_price_change_expected_at: None,
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
