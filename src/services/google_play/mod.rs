/// Google Play billing service implementation
#[allow(dead_code)]
pub mod client;
#[allow(dead_code)]
pub mod models;

// These modules are from the monolith and need adaptation for Bridge's PaymentProvider trait
// They can be integrated later when Bridge has equivalent database/handler structures
// pub mod provider;
// pub mod validation;
// pub mod trace;
// pub mod notifications;
// pub mod product_lifecycle;
// pub mod subscription_lifecycle;
