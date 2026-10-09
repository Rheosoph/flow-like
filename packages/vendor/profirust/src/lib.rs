//! # `profirust` - A PROFIBUS-DP compatible communication stack
//!
//! _profirust_ is structured according to the layered model of PROFIBUS:
//!
//! - The [`phy`] module abstracts physical interfaces for RS-485 communication.
//! - The [`fdl`] module implements the _Fieldbus Data Link_ layer of basic bus communication and
//!   token passing between multiple master stations.
//! - The [`dp`] module implements the PROFIBUS-DP (Decentralized Peripherals) application layer.
//!   This is where peripherals are managed and cyclic data exchange is facilitated.
//!
//! # Example
//! To successfully communicate with a peripheral, you need to initialize and parameterize all
//! layers.  Here is an example:
//!
//! ```no_run
//! use profirust::{Baudrate, fdl, dp, phy};
//!
//! // Initialize the DP master:
//! // =========================
//! let buffer: [profirust::dp::PeripheralStorage; 4] = Default::default();
//! let mut dp_master = profirust::dp::DpMaster::new(buffer);
//! // or with `std`:
//! // let mut dp_master = dp::DpMaster::new(Vec::new());
//!
//! // Let's add a peripheral:
//! // =======================
//! let remoteio_address = 7;
//! let remoteio_options = dp::PeripheralOptions {
//!     // ...
//!     // best generated using `gsdtool`
//!     // ...
//!     ..Default::default()
//! };
//! let mut buffer_inputs = [0u8; 8];
//! let mut buffer_outputs = [0u8; 4];
//!
//! let remoteio_handle = dp_master.add(dp::Peripheral::new(
//!     remoteio_address, remoteio_options, &mut buffer_inputs[..], &mut buffer_outputs[..]
//! ));
//!
//! // Set up the FDL master and parameterize it:
//! // ==========================================
//! let master_address = 2;
//! let mut fdl = fdl::FdlActiveStation::new(
//!     fdl::ParametersBuilder::new(master_address, Baudrate::B19200)
//!         .slot_bits(300)
//!         .build_verified(&dp_master)
//! );
//!
//! // Initialize the PHY layer:
//! // =========================
//! let mut phy = phy::SerialPortPhy::new("/dev/ttyUSB0", fdl.parameters().baudrate);
//!
//! // Now let's go live:
//! // ==================
//! fdl.set_online();
//! dp_master.enter_operate();
//!
//! // Main Application Cycle
//! // ======================
//! loop {
//!     let now = profirust::time::Instant::now();
//!     fdl.poll(now, &mut phy, &mut dp_master);
//!     let events = dp_master.take_last_events();
//!
//!     // Do something whenever new the DP cycle (for all peripherals) completes:
//!     if events.cycle_completed {
//!         let remoteio = dp_master.get_mut(remoteio_handle);
//!         println!("Inputs: {:?}", remoteio.pi_i());
//!
//!         // Set some output bits
//!         let pi_q = remoteio.pi_q_mut();
//!         pi_q[0] = 0x80;
//!     }
//! }
//! ```
// TODO: Remove this once the crate has matured.
#![allow(dead_code)]
#![allow(unused_variables)]
#![cfg_attr(not(any(feature = "std", test)), no_std)]

mod consts;
pub mod dp;
pub mod fdl;
pub mod phy;
pub mod time;

#[cfg(all(test, feature = "std"))]
pub mod test_utils;

/// Baudrate for fieldbus communication
///
/// - PROFIBUS DP networks can run at any of the available baudrates given that all stations
///   support the selected speed.
/// - PROFIBUS PA networks must use `B31250` (31.25 kbit/s).
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum Baudrate {
    /// 9.6 kbit/s
    B9600,
    /// 19.2 kbit/s
    B19200,
    /// 31.25 kbit/s
    B31250,
    /// 45.45 kbit/s
    B45450,
    /// 93.75 kbit/s
    B93750,
    /// 187.5 kbit/s
    B187500,
    /// 500 kbit/s
    B500000,
    /// 1.5 Mbit/s
    B1500000,
    /// 3 Mbit/s
    B3000000,
    /// 6 Mbit/s
    B6000000,
    /// 12 Mbit/s
    B12000000,
}

impl Baudrate {
    /// Convert baudrate into its numeric value in bit/s.
    pub fn to_rate(self) -> u64 {
        match self {
            Baudrate::B9600 => 9600,
            Baudrate::B19200 => 19200,
            Baudrate::B31250 => 31250,
            Baudrate::B45450 => 45450,
            Baudrate::B93750 => 93750,
            Baudrate::B187500 => 187500,
            Baudrate::B500000 => 500000,
            Baudrate::B1500000 => 1500000,
            Baudrate::B3000000 => 3000000,
            Baudrate::B6000000 => 6000000,
            Baudrate::B12000000 => 12000000,
        }
    }

    /// At this baudrate, return how long a given number of bits take to transmit.
    pub fn bits_to_time(self, bits: u32) -> crate::time::Duration {
        crate::time::Duration::from_micros(u64::from(bits) * 1000000 / self.to_rate())
    }

    /// At this baudrate, return how many bits could be transmitted in the given time.
    pub fn time_to_bits(self, time: crate::time::Duration) -> u64 {
        time.total_micros() * self.to_rate() / 1000000
    }
}

pub type Address = u8;

#[inline(always)]
#[track_caller]
pub(crate) fn debug_assert_address(addr: Address) {
    debug_assert!(
        addr <= 127,
        "PROFIBUS address cannot be bigger than 127, got {addr}!"
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn baudrate_time_conversions() {
        let all_bauds = &[
            crate::Baudrate::B9600,
            crate::Baudrate::B19200,
            crate::Baudrate::B31250,
            crate::Baudrate::B45450,
            crate::Baudrate::B93750,
            crate::Baudrate::B187500,
            crate::Baudrate::B500000,
            crate::Baudrate::B1500000,
            crate::Baudrate::B3000000,
            crate::Baudrate::B6000000,
            crate::Baudrate::B12000000,
        ];
        let test_values = &[0, 1, 10, 100, 2000, 65536, u32::MAX];

        for baud in all_bauds.iter().copied() {
            for bits in test_values.iter().copied() {
                let time = baud.bits_to_time(bits);
                let micros = time.total_micros();
                let bits2 = baud.time_to_bits(time);

                let max_difference = match baud {
                    crate::Baudrate::B9600 => 1,
                    crate::Baudrate::B19200 => 1,
                    crate::Baudrate::B31250 => 1,
                    crate::Baudrate::B45450 => 1,
                    crate::Baudrate::B93750 => 1,
                    crate::Baudrate::B187500 => 1,
                    crate::Baudrate::B500000 => 1,
                    crate::Baudrate::B1500000 => 1,
                    crate::Baudrate::B3000000 => 2,
                    crate::Baudrate::B6000000 => 4,
                    crate::Baudrate::B12000000 => 10,
                };
                assert!(
                    u64::from(bits) - bits2 <= max_difference,
                    "{bits} (={micros}us) was converted to {bits2} (at {baud:?})"
                );
            }
        }
    }
}
