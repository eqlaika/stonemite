//! Generic Stonemite control/state API and its WebSocket transport.
//!
//! [`control`] is transport-independent. [`protocol`] and [`server`] provide
//! the small versioned JSON-over-WebSocket transport named `trushar`.

pub mod control;
pub mod protocol;
pub mod server;
