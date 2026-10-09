//! Pure-Rust GenICam camera library.
//!
//! [`GenICamera`] exposes a device's GenICam feature tree (Width,
//! ExposureTime, PixelFormat, AcquisitionStart, ...) by name, evaluated
//! through the node graph of the device's own description XML, plus
//! acquisition helpers ([`start_acquisition`](GenICamera::start_acquisition),
//! [`snap`](GenICamera::snap), [`snapshot_session`](GenICamera::snapshot_session)).
//!
//! Transports live in backend modules. The only backend today is [`gige`]
//! (GigE Vision: GVCP control and GVSP streaming spoken directly, no vendor
//! SDK); the feature layer is an enum over the transport so USB3 Vision can
//! slot in later. Each channel runs a dedicated I/O thread; handles are
//! cheap to clone and share it.

#![deny(
    arithmetic_overflow,
    missing_debug_implementations,
    unused_unsafe,
    unreachable_code,
    clippy::panicking_unwrap,
    clippy::missing_safety_doc
)]
#![allow(clippy::module_name_repetitions, clippy::option_if_let_else)]
#![cfg_attr(
    not(test),
    forbid(
        clippy::panic,
        clippy::todo,
        clippy::unimplemented,
        clippy::expect_used,
        clippy::unwrap_used
    )
)]

#[cfg(feature = "valuable")]
#[macro_use]
mod valuable_error;

#[cfg(feature = "emulator")]
pub mod emulator;
pub mod error;
pub mod genicam;
pub mod gige;
pub mod handle;
#[cfg(feature = "py")]
mod py;
mod thread_util;
#[cfg(feature = "valuable")]
mod valuable_impls;

pub use error::{CameraError, GenicamError, Result};
pub use genicam::{AccessMode, Acquisition, Features, GenICamera, NodeKind, SnapshotSession};
pub use gige::PixelFormat;
pub use gige::stream::{
    Frame, FrameChannel, FrameStatus, PacketSize, PayloadKind, ResendPolicy, StreamChannel,
    StreamConfig, StreamStats,
};
pub use handle::ResponseHandle;
pub use thread_util::ThreadConfig;
