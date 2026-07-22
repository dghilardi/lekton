//! Learn mode: teach-style personalized lessons grounded on the internal
//! documentation. Server-side only — the frontend consumes lessons through
//! server functions returning [`crate::db::learn_models`] types.

pub mod generator;
pub mod prompt;
pub mod service;
