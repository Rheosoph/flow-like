//! The device port the GenICam node graph performs its register access
//! through.

use std::time::Duration;

use crate::error::{CameraError, Result};
use crate::gige::ControlPort;
use crate::handle::unwrap_arc;

pub trait PortIo {
    fn read(&self, address: u64, buf: &mut [u8]) -> Result<()>;
    fn write(&self, address: u64, data: &[u8]) -> Result<()>;
}

fn budget_for(port: &ControlPort, len: usize) -> Duration {
    // Chunked memory transfers take one transaction per 512 bytes.
    port.budget() * (len as u32 / 512 + 1)
}

impl PortIo for ControlPort {
    fn read(&self, address: u64, buf: &mut [u8]) -> Result<()> {
        let address = u32::try_from(address)
            .map_err(|_| CameraError::Protocol(format!("address {address:#x} beyond 32 bit")))?;
        if buf.len() == 4 && address.is_multiple_of(4) {
            let value = self
                .read_register(address)
                .wait_timeout(budget_for(self, 4))
                .map_err(unwrap_arc)?;
            buf.copy_from_slice(&value.to_be_bytes());
            return Ok(());
        }
        let data = self
            .read_memory(address, buf.len() as u32)
            .wait_timeout(budget_for(self, buf.len()))
            .map_err(unwrap_arc)?;
        if data.len() != buf.len() {
            return Err(CameraError::Protocol("short memory read".into()));
        }
        buf.copy_from_slice(&data);
        Ok(())
    }

    fn write(&self, address: u64, data: &[u8]) -> Result<()> {
        let address = u32::try_from(address)
            .map_err(|_| CameraError::Protocol(format!("address {address:#x} beyond 32 bit")))?;
        if data.len() == 4 && address.is_multiple_of(4) {
            let value = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
            self.write_register(address, value)
                .wait_timeout(budget_for(self, 4))
                .map_err(unwrap_arc)?;
            return Ok(());
        }
        self.write_memory(address, data.to_vec())
            .wait_timeout(budget_for(self, data.len()))
            .map_err(unwrap_arc)?;
        Ok(())
    }
}

/// A flat in-memory port for tests: a byte array addressed from 0.
#[derive(Debug)]
pub struct MockPort {
    pub mem: parking_lot::Mutex<Vec<u8>>,
}

impl MockPort {
    pub fn new(size: usize) -> Self {
        Self {
            mem: parking_lot::Mutex::new(vec![0u8; size]),
        }
    }

    pub fn set_u32_be(&self, address: u64, value: u32) {
        let a = address as usize;
        self.mem.lock()[a..a + 4].copy_from_slice(&value.to_be_bytes());
    }

    pub fn u32_be(&self, address: u64) -> u32 {
        let a = address as usize;
        let mem = self.mem.lock();
        u32::from_be_bytes([mem[a], mem[a + 1], mem[a + 2], mem[a + 3]])
    }
}

impl PortIo for MockPort {
    fn read(&self, address: u64, buf: &mut [u8]) -> Result<()> {
        let a = address as usize;
        let mem = self.mem.lock();
        let slice = mem
            .get(a..a + buf.len())
            .ok_or_else(|| CameraError::Protocol(format!("mock read out of range {address:#x}")))?;
        buf.copy_from_slice(slice);
        Ok(())
    }

    fn write(&self, address: u64, data: &[u8]) -> Result<()> {
        let a = address as usize;
        let mut mem = self.mem.lock();
        let slice = mem.get_mut(a..a + data.len()).ok_or_else(|| {
            CameraError::Protocol(format!("mock write out of range {address:#x}"))
        })?;
        slice.copy_from_slice(data);
        Ok(())
    }
}
