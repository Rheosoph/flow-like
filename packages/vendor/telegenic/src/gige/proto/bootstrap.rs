//! The GigE Vision bootstrap register map and the device description block
//! returned by discovery (the first 0xf8 bytes of register space).

use std::net::Ipv4Addr;

use zerocopy::FromBytes;
use zerocopy::byteorder::network_endian::U32;

pub const VERSION: u32 = 0x0000;
pub const DEVICE_MODE: u32 = 0x0004;
pub const DEVICE_MAC_HIGH: u32 = 0x0008;
pub const DEVICE_MAC_LOW: u32 = 0x000c;
pub const SUPPORTED_IP_CONFIG: u32 = 0x0010;
pub const CURRENT_IP_CONFIG: u32 = 0x0014;
pub const CURRENT_IP_ADDRESS: u32 = 0x0024;
pub const CURRENT_SUBNET_MASK: u32 = 0x0034;
pub const CURRENT_GATEWAY: u32 = 0x0044;
pub const MANUFACTURER_NAME: u32 = 0x0048;
pub const MANUFACTURER_NAME_SIZE: usize = 32;
pub const MODEL_NAME: u32 = 0x0068;
pub const MODEL_NAME_SIZE: usize = 32;
pub const DEVICE_VERSION: u32 = 0x0088;
pub const DEVICE_VERSION_SIZE: usize = 32;
pub const MANUFACTURER_INFO: u32 = 0x00a8;
pub const MANUFACTURER_INFO_SIZE: usize = 48;
pub const SERIAL_NUMBER: u32 = 0x00d8;
pub const SERIAL_NUMBER_SIZE: usize = 16;
pub const USER_DEFINED_NAME: u32 = 0x00e8;
pub const USER_DEFINED_NAME_SIZE: usize = 16;
/// Size of the device description block carried by a discovery acknowledge.
pub const DISCOVERY_DATA_SIZE: usize = 0xf8;

pub const XML_URL_0: u32 = 0x0200;
pub const XML_URL_1: u32 = 0x0400;
pub const XML_URL_SIZE: usize = 512;

pub const N_NETWORK_INTERFACES: u32 = 0x0600;
pub const N_MESSAGE_CHANNELS: u32 = 0x0900;
pub const N_STREAM_CHANNELS: u32 = 0x0904;

pub const GVCP_CAPABILITY: u32 = 0x0934;
pub const CAP_CONCATENATION: u32 = 1 << 0;
pub const CAP_WRITE_MEMORY: u32 = 1 << 1;
pub const CAP_PACKET_RESEND: u32 = 1 << 2;
pub const CAP_EVENT: u32 = 1 << 3;
pub const CAP_EVENT_DATA: u32 = 1 << 4;
pub const CAP_PENDING_ACK: u32 = 1 << 5;
pub const CAP_ACTION: u32 = 1 << 6;
pub const CAP_EXTENDED_STATUS_CODES: u32 = 1 << 22;
pub const CAP_HEARTBEAT_DISABLE: u32 = 1 << 29;

pub const HEARTBEAT_TIMEOUT: u32 = 0x0938;
pub const TIMESTAMP_TICK_FREQUENCY_HIGH: u32 = 0x093c;
pub const TIMESTAMP_TICK_FREQUENCY_LOW: u32 = 0x0940;
pub const TIMESTAMP_CONTROL: u32 = 0x0944;
pub const TIMESTAMP_LATCHED_HIGH: u32 = 0x0948;
pub const TIMESTAMP_LATCHED_LOW: u32 = 0x094c;

pub const CONTROL_CHANNEL_PRIVILEGE: u32 = 0x0a00;
pub const CCP_CONTROL: u32 = 1 << 1;
pub const CCP_EXCLUSIVE: u32 = 1 << 0;

pub const MESSAGE_CHANNEL_PORT: u32 = 0x0b00;
pub const MESSAGE_CHANNEL_DEST_ADDRESS: u32 = 0x0b10;
pub const MESSAGE_CHANNEL_TRANSMISSION_TIMEOUT: u32 = 0x0b14;
pub const MESSAGE_CHANNEL_RETRY_COUNT: u32 = 0x0b18;
pub const MESSAGE_CHANNEL_SOURCE_PORT: u32 = 0x0b1c;

/// Stream channel 0 registers; channel `n` lives at `+ n * STREAM_CHANNEL_STRIDE`.
pub const STREAM_CHANNEL_PORT: u32 = 0x0d00;
pub const STREAM_CHANNEL_PACKET_SIZE: u32 = 0x0d04;
pub const STREAM_CHANNEL_PACKET_DELAY: u32 = 0x0d08;
pub const STREAM_CHANNEL_DEST_ADDRESS: u32 = 0x0d18;
pub const STREAM_CHANNEL_SOURCE_PORT: u32 = 0x0d1c;
pub const STREAM_CHANNEL_STRIDE: u32 = 0x40;

pub const SCPS_PACKET_SIZE_MASK: u32 = 0x0000_ffff;
pub const SCPS_BIG_ENDIAN: u32 = 1 << 29;
pub const SCPS_DO_NOT_FRAGMENT: u32 = 1 << 30;
pub const SCPS_FIRE_TEST_PACKET: u32 = 1 << 31;

/// Identity and addressing of a device, parsed from the first
/// [`DISCOVERY_DATA_SIZE`] bytes of bootstrap register space (the discovery
/// acknowledge payload, or the same block read via READMEM).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "py", pyo3::pyclass(skip_from_py_object))]
pub struct DeviceInfo {
    pub spec_version: (u16, u16),
    pub device_mode: u32,
    pub mac: [u8; 6],
    pub supported_ip_config: u32,
    pub current_ip_config: u32,
    pub ip: Ipv4Addr,
    pub subnet_mask: Ipv4Addr,
    pub gateway: Ipv4Addr,
    pub manufacturer: String,
    pub model: String,
    pub device_version: String,
    pub manufacturer_info: String,
    pub serial: String,
    pub user_defined_name: String,
}

impl DeviceInfo {
    pub fn parse(block: &[u8]) -> Option<Self> {
        if block.len() < DISCOVERY_DATA_SIZE {
            return None;
        }
        let version = reg(block, VERSION)?;
        let mut mac = [0u8; 6];
        mac.copy_from_slice(&block[0x0a..0x10]);
        Some(Self {
            spec_version: ((version >> 16) as u16, version as u16),
            device_mode: reg(block, DEVICE_MODE)?,
            mac,
            supported_ip_config: reg(block, SUPPORTED_IP_CONFIG)?,
            current_ip_config: reg(block, CURRENT_IP_CONFIG)?,
            ip: ip_reg(block, CURRENT_IP_ADDRESS)?,
            subnet_mask: ip_reg(block, CURRENT_SUBNET_MASK)?,
            gateway: ip_reg(block, CURRENT_GATEWAY)?,
            manufacturer: string_reg(block, MANUFACTURER_NAME, MANUFACTURER_NAME_SIZE),
            model: string_reg(block, MODEL_NAME, MODEL_NAME_SIZE),
            device_version: string_reg(block, DEVICE_VERSION, DEVICE_VERSION_SIZE),
            manufacturer_info: string_reg(block, MANUFACTURER_INFO, MANUFACTURER_INFO_SIZE),
            serial: string_reg(block, SERIAL_NUMBER, SERIAL_NUMBER_SIZE),
            user_defined_name: string_reg(block, USER_DEFINED_NAME, USER_DEFINED_NAME_SIZE),
        })
    }
}

fn reg(block: &[u8], offset: u32) -> Option<u32> {
    let start = offset as usize;
    U32::read_from_bytes(block.get(start..start + 4)?)
        .ok()
        .map(|v| v.get())
}

fn ip_reg(block: &[u8], offset: u32) -> Option<Ipv4Addr> {
    reg(block, offset).map(Ipv4Addr::from)
}

/// NUL-padded fixed-size string register, trimmed and lossily decoded.
fn string_reg(block: &[u8], offset: u32, size: usize) -> String {
    let start = offset as usize;
    let raw = &block[start..start + size];
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    String::from_utf8_lossy(&raw[..end]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_block() -> Vec<u8> {
        let mut b = vec![0u8; DISCOVERY_DATA_SIZE];
        b[..4].copy_from_slice(&[0x00, 0x02, 0x00, 0x00]); // GEV 2.0
        b[0x0a..0x10].copy_from_slice(&[0xaa, 0xbb, 0xcc, 0x01, 0x02, 0x03]);
        b[0x14..0x18].copy_from_slice(&[0, 0, 0, 0b101]);
        b[0x24..0x28].copy_from_slice(&[192, 168, 1, 50]);
        b[0x34..0x38].copy_from_slice(&[255, 255, 255, 0]);
        b[0x44..0x48].copy_from_slice(&[192, 168, 1, 1]);
        b[0x48..0x48 + 4].copy_from_slice(b"Acme");
        b[0x68..0x68 + 5].copy_from_slice(b"Cam2k");
        b[0xd8..0xd8 + 6].copy_from_slice(b"SN0042");
        b
    }

    #[test]
    fn parse_device_info() {
        let info = DeviceInfo::parse(&sample_block()).unwrap();
        assert_eq!(info.spec_version, (2, 0));
        assert_eq!(info.mac, [0xaa, 0xbb, 0xcc, 0x01, 0x02, 0x03]);
        assert_eq!(info.current_ip_config, 0b101);
        assert_eq!(info.ip, Ipv4Addr::new(192, 168, 1, 50));
        assert_eq!(info.subnet_mask, Ipv4Addr::new(255, 255, 255, 0));
        assert_eq!(info.gateway, Ipv4Addr::new(192, 168, 1, 1));
        assert_eq!(info.manufacturer, "Acme");
        assert_eq!(info.model, "Cam2k");
        assert_eq!(info.serial, "SN0042");
        assert_eq!(info.user_defined_name, "");
    }

    #[test]
    fn parse_rejects_short_block() {
        assert!(DeviceInfo::parse(&[0u8; 0x40]).is_none());
    }
}
