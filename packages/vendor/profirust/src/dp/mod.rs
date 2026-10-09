//! DP - Decentralized peripherals
//!
//! This module implements the DP application layer of PROFIBUS.  The main component is the
//! [`DpMaster`] type which manages the DP cyclic communication and peripheral lifecycle.
//!
//! Peripherals are represented as [`Peripheral`] objects which you need to construct using
//! [`PeripheralOptions`].  These options are best generated from the peripheral's GSD file using
//! the `gsdtool` that is part of the `profirust` project.
mod diagnostics;
mod master;
mod peripheral;
mod peripheral_set;
pub mod scan;

pub use diagnostics::{
    ChannelDataType, ChannelDiagnostics, ChannelError, ExtDiagBlock, ExtDiagBlockIter,
    ExtendedDiagnostics,
};
pub(crate) use master::DpMasterState;
pub use master::{DpEvents, DpMaster, OperatingState};
pub(crate) use peripheral::DiagnosticsInfo;
pub use peripheral::{
    DiagnosticFlags, Peripheral, PeripheralDiagnostics, PeripheralEvent, PeripheralOptions,
};
pub(crate) use peripheral_set::PeripheralSet;
pub use peripheral_set::{PeripheralHandle, PeripheralStorage};
