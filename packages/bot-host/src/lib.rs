//! The generic host of account bots: claim a turn from a hub, keep its lease, run it through a
//! [`TurnRunner`] and answer as the bot. The desktop and the device agent each bring a hub and
//! a runner.

pub mod account;
pub mod authorization;
pub mod bridge;
pub mod e2e;
pub mod event;
pub mod gate;
pub mod host;
pub mod hub;
pub mod latency;
pub mod live_reply;
pub mod memory;
pub mod notes;
pub mod recovery;
pub mod room_updates;
pub mod rule;
pub mod secrets;
pub mod taint;
pub mod tasks;
pub mod transcript;

pub use account::{AccountConfig, AccountHost};
pub use gate::ApprovalGate;

#[cfg(test)]
mod tests;
