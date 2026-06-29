// `services` is `pub` (from `lib.rs`) so `main.rs` can reach `services::email`.
// The other service modules are internal; `pub(crate)` keeps them reachable
// inside the crate without exposing `pub fn`s that leak `pub(crate)` types.
pub(crate) mod google_play;
pub(crate) mod creem;
pub(crate) mod provider_api;
pub mod email;
pub(crate) mod email_lookup;
