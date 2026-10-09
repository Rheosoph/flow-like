//! Everything to do with ADS notifications.

use std::io;
use std::time::Duration;

use byteorder::{ReadBytesExt, LE};

use crate::client::AMS_HEADER_SIZE;
use crate::errors::ErrContext;
use crate::{Error, Result};

/// A handle to the notification; this can be used to delete the notification later.
pub type Handle = u32;

/// Attributes for creating a notification.
pub struct Attributes {
    /// Length of data the notification is interested in.
    pub length: usize,
    /// When notification messages should be transmitted.
    pub trans_mode: TransmissionMode,
    /// The maximum delay between change and transmission.
    pub max_delay: Duration,
    /// The cycle time for checking for changes.
    pub cycle_time: Duration,
}

impl Attributes {
    /// Return new notification attributes.
    pub fn new(length: usize, trans_mode: TransmissionMode,
               max_delay: Duration, cycle_time: Duration) -> Self {
        Self { length, trans_mode, max_delay, cycle_time }
    }
}

/// When notifications should be generated.
#[repr(u32)]
#[derive(Clone, Copy, Debug)]
pub enum TransmissionMode {
    /// No transmission.
    NoTrans = 0,
    /// Notify each server cycle.
    ServerCycle = 3,
    /// Notify when the content changes.
    ServerOnChange = 4,
    // Other constants from the C++ library:
    // ClientCycle = 1,
    // ClientOnChange = 2,
    // ServerCycle2 = 5,
    // ServerOnChange2 = 6,
    // Client1Req = 10,
}

/// A notification message from the ADS server.
pub struct Notification {
    data: Vec<u8>,
    nstamps: u32,
}

impl std::fmt::Debug for Notification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Notification [")?;
        for sample in self.samples() {
            writeln!(f, "    {:?}", sample)?;
        }
        write!(f, "]")
    }
}

impl Notification {
    /// Parse a notification message from an ADS message.
    pub fn new(data: impl Into<Vec<u8>>) -> Result<Self> {
        // Relevant data starts at byte 42 with the number of stamps.
        let data = data.into();
        if data.len() < AMS_HEADER_SIZE + 8 {  // header + length + #stamps
            return Err(Error::Io("parsing notification",
                                 io::ErrorKind::UnexpectedEof.into()));
        }
        let payload_len = data.len() - AMS_HEADER_SIZE;
        let length = (&data[AMS_HEADER_SIZE..]).read_u32::<LE>()
            .ctx("parsing notification")? as usize;
        // Beckhoff's .NET server excludes Length and Stamps from this field.
        // Other ADS peers exclude only Length. Validate either convention, then
        // parse every stamp/sample below and reject truncated or trailing data.
        if length != payload_len - 4 && length != payload_len - 8 {
            return Err(Error::Reply("parsing notification", "invalid stream length", length as _));
        }
        let mut ptr = &data[AMS_HEADER_SIZE + 4..];
        let nstamps = ptr.read_u32::<LE>().ctx("parsing notification")?;
        for _ in 0..nstamps {
            let _timestamp = ptr.read_u64::<LE>().ctx("parsing notification")?;
            let nsamples = ptr.read_u32::<LE>().ctx("parsing notification")?;

            for _ in 0..nsamples {
                let _handle = ptr.read_u32::<LE>().ctx("parsing notification")?;
                let length = ptr.read_u32::<LE>().ctx("parsing notification")? as usize;
                if ptr.len() >= length {
                    ptr = &ptr[length..];
                } else {
                    return Err(Error::Io("parsing notification",
                                         io::ErrorKind::UnexpectedEof.into()));
                }
            }
        }
        if ptr.is_empty() {
            Ok(Self { data, nstamps })
        } else {
            Err(Error::Io("parsing notification",
                          io::ErrorKind::UnexpectedEof.into()))
        }
    }

    /// Return an iterator over all data samples in this notification.
    pub fn samples(&self) -> SampleIter<'_> {
        SampleIter { data: &self.data[46..], cur_timestamp: 0,
                     stamps_left: self.nstamps, samples_left: 0 }
    }
}

/// A single sample in a notification message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sample<'a> {
    /// The notification handle associated with the data.
    pub handle: Handle,
    /// Timestamp of generation (100-nanosecond intervals since 01/01/1601 UTC).
    pub timestamp: u64, // TODO: better dtype?
    /// Data of the handle at the specified time.
    pub data: &'a [u8],
}

/// An iterator over all samples within a notification message.
pub struct SampleIter<'a> {
    data: &'a [u8],
    cur_timestamp: u64,
    stamps_left: u32,
    samples_left: u32,
}

impl<'a> Iterator for SampleIter<'a> {
    type Item = Sample<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        // Empty stamp batches are valid and must not consume the thread stack.
        while self.samples_left == 0 {
            if self.stamps_left == 0 {
                return None;
            }
            self.cur_timestamp = self.data.read_u64::<LE>().expect("size");
            self.samples_left = self.data.read_u32::<LE>().expect("size");
            self.stamps_left -= 1;
        }
        let handle = self.data.read_u32::<LE>().expect("size");
        let length = self.data.read_u32::<LE>().expect("size") as usize;
        let (data, rest) = self.data.split_at(length);
        self.data = rest;
        self.samples_left -= 1;
        Some(Sample { handle, data, timestamp: self.cur_timestamp })
    }
}

#[cfg(test)]
mod compatibility_tests {
    use super::*;

    fn vendor_frame() -> Vec<u8> {
        // Captured from Beckhoff AdsSymbolicServer 6.2.521: one INT sample.
        let payload = [
            0x16, 0, 0, 0, 1, 0, 0, 0,
            0x85, 0xb4, 0xf2, 0x0d, 0xc7, 0x57, 0xdd, 1,
            1, 0, 0, 0, 0, 0x40, 0, 0, 2, 0, 0, 0, 0, 0,
        ];
        let mut frame = vec![0; AMS_HEADER_SIZE];
        frame.extend_from_slice(&payload);
        frame
    }

    #[test]
    fn captured_beckhoff_sample_and_legacy_length_decode_identically() {
        for length in [22u32, 26] {
            let mut frame = vendor_frame();
            frame[AMS_HEADER_SIZE..AMS_HEADER_SIZE + 4].copy_from_slice(&length.to_le_bytes());
            let notification = Notification::new(frame).unwrap();
            let samples: Vec<_> = notification.samples().collect();
            assert_eq!(samples.len(), 1);
            assert_eq!(samples[0].handle, 0x4000);
            assert_eq!(samples[0].timestamp, 0x01dd57c70df2b485);
            assert_eq!(samples[0].data, [0, 0]);
        }
    }

    #[test]
    fn empty_stamp_batches_do_not_recurse() {
        let count = 50_000u32;
        let mut frame = vec![0; AMS_HEADER_SIZE];
        frame.extend_from_slice(&(12 * count).to_le_bytes());
        frame.extend_from_slice(&count.to_le_bytes());
        frame.resize(AMS_HEADER_SIZE + 8 + 12 * count as usize, 0);
        let notification = Notification::new(frame).unwrap();
        assert_eq!(notification.samples().count(), 0);
    }

    #[test]
    fn compatibility_does_not_accept_bad_lengths_counts_or_truncation() {
        for length in [0u32, 1, 21, 23, 25, 27, 31] {
            let mut frame = vendor_frame();
            frame[AMS_HEADER_SIZE..AMS_HEADER_SIZE + 4].copy_from_slice(&length.to_le_bytes());
            assert!(Notification::new(frame).is_err());
        }
        for (offset, value) in [(4, 2u32), (16, 2), (24, 3)] {
            let mut frame = vendor_frame();
            frame[AMS_HEADER_SIZE + offset..AMS_HEADER_SIZE + offset + 4]
                .copy_from_slice(&value.to_le_bytes());
            assert!(Notification::new(frame).is_err());
        }
        let original = vendor_frame();
        for end in 0..original.len() {
            assert!(Notification::new(original[..end].to_vec()).is_err());
        }
        for trailing in [false, true] {
            let mut frame = vendor_frame();
            if trailing { frame.push(0); } else { frame.pop(); }
            let length = (frame.len() - AMS_HEADER_SIZE - 8) as u32;
            frame[AMS_HEADER_SIZE..AMS_HEADER_SIZE + 4].copy_from_slice(&length.to_le_bytes());
            assert!(Notification::new(frame).is_err());
        }
    }
}
