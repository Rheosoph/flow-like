//! Shared visibility-review pipeline for apps and suites (app groups).
//!
//! Both entity kinds move to a public visibility through the *same*
//! `PublicationRequest` rows, statuses, logs and admin decision path. The only
//! thing that differs is what is being reviewed, which is captured by
//! [`PublicationTarget`].

pub mod gate;
pub mod target;

pub use target::PublicationTarget;
