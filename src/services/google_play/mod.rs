/// Google Play billing service implementation
pub mod client;
pub mod models;
pub mod provider;
pub mod validation;
pub mod trace;

// These modules depend on hiha-specific structures and are not needed for Bridge core
// They can be integrated later when Bridge has equivalent database/handler structures
// pub mod notifications;
// pub mod product_lifecycle;
// pub mod subscription_lifecycle;

pub use provider::GooglePlayProvider;
pub use validation::TokenValidationMode;
#[allow(unused_imports)]
pub use trace::BpTrace;
