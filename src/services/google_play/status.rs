#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GoogleSubscriptionStateStatus<'a> {
    Known(&'static str),
    Unknown(&'a str),
    Missing,
}

pub(crate) fn subscription_state_to_canonical_status(
    subscription_state: Option<&str>,
) -> GoogleSubscriptionStateStatus<'_> {
    match subscription_state {
        Some("SUBSCRIPTION_STATE_ACTIVE") => GoogleSubscriptionStateStatus::Known("active"),
        Some("SUBSCRIPTION_STATE_CANCELED") => GoogleSubscriptionStateStatus::Known("cancelled"),
        Some("SUBSCRIPTION_STATE_IN_GRACE_PERIOD") => GoogleSubscriptionStateStatus::Known("past_due"),
        Some("SUBSCRIPTION_STATE_ON_HOLD") => GoogleSubscriptionStateStatus::Known("on_hold"),
        Some("SUBSCRIPTION_STATE_PAUSED") => GoogleSubscriptionStateStatus::Known("paused"),
        Some("SUBSCRIPTION_STATE_PENDING") => GoogleSubscriptionStateStatus::Known("pending"),
        Some("SUBSCRIPTION_STATE_EXPIRED") => GoogleSubscriptionStateStatus::Known("expired"),
        Some(other) => GoogleSubscriptionStateStatus::Unknown(other),
        None => GoogleSubscriptionStateStatus::Missing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn google_subscription_state_mapping_preserves_unknowns() {
        assert_eq!(
            subscription_state_to_canonical_status(Some("SUBSCRIPTION_STATE_ACTIVE")),
            GoogleSubscriptionStateStatus::Known("active")
        );
        assert_eq!(
            subscription_state_to_canonical_status(Some("SUBSCRIPTION_STATE_FUTURE")),
            GoogleSubscriptionStateStatus::Unknown("SUBSCRIPTION_STATE_FUTURE")
        );
        assert_eq!(subscription_state_to_canonical_status(None), GoogleSubscriptionStateStatus::Missing);
    }
}
