use crate::{Error, Result, require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, SocketAddrV4};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GenicamCaptureConfig {
    pub device_address: String,
    pub local_interface: Option<String>,
    pub frame_timeout_ms: u64,
    pub command_timeout_ms: u64,
    pub retries: u8,
    pub max_frame_bytes: usize,
    pub pixel_format: Option<String>,
    pub exposure_time_us: Option<f64>,
}

impl GenicamCaptureConfig {
    pub fn validate(&self) -> Result<()> {
        let address: SocketAddrV4 = self.device_address.parse().map_err(|_| {
            Error::Invalid("GigE Vision address must be an IPv4 address and port".into())
        })?;
        require(
            !address.ip().is_unspecified() && !address.ip().is_multicast() && address.port() > 0,
            "GigE Vision requires a unicast device address",
        )?;
        if let Some(ip) = &self.local_interface {
            let ip: Ipv4Addr = ip.parse().map_err(|_| {
                Error::Invalid("GigE Vision local interface must be an IPv4 address".into())
            })?;
            require(
                !ip.is_unspecified() && !ip.is_multicast(),
                "Invalid local interface",
            )?;
        }
        require(
            (1..=300_000).contains(&self.frame_timeout_ms)
                && (1..=30_000).contains(&self.command_timeout_ms)
                && self.retries <= 8,
            "Invalid capture or command timeout",
        )?;
        require(
            (1..=64 * 1024 * 1024).contains(&self.max_frame_bytes),
            "Frame limit must be between 1 byte and 64 MiB",
        )?;
        if let Some(value) = &self.pixel_format {
            require(
                !value.is_empty()
                    && value.len() <= 128
                    && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
                "Invalid GenICam pixel format name",
            )?;
        }
        if let Some(value) = self.exposure_time_us {
            require(
                value.is_finite() && value > 0.0,
                "Exposure must be finite and positive",
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GenicamFrame {
    pub width: u32,
    pub height: u32,
    pub pixel_format: u32,
    pub pixel_format_name: String,
    pub x_offset: u32,
    pub y_offset: u32,
    pub x_padding: u16,
    pub y_padding: u16,
    pub has_chunks: bool,
    pub frame_id: u64,
    pub timestamp_ticks: u64,
    pub timestamp_ns: Option<u64>,
    pub system_timestamp_ns: u64,
    pub bytes: Vec<u8>,
}

impl GenicamFrame {
    /// Convert supported unpacked 8-bit formats to tightly packed RGB.
    pub fn to_rgb8(&self) -> Result<Vec<u8>> {
        let channels = match self.pixel_format {
            0x0108_0001 => 1,
            0x0218_0014 | 0x0218_0015 => 3,
            _ => {
                return Err(Error::Invalid(
                    "RGB conversion supports Mono8, RGB8 and BGR8".into(),
                ));
            }
        };
        require(
            self.width > 0 && self.height > 0 && self.y_padding == 0,
            "Invalid or unsupported frame geometry",
        )?;
        let pixels = (self.width as usize)
            .checked_mul(self.height as usize)
            .ok_or_else(|| Error::Invalid("Frame geometry overflow".into()))?;
        let output_size = pixels
            .checked_mul(3)
            .filter(|size| *size <= 192 * 1024 * 1024)
            .ok_or_else(|| Error::Invalid("RGB frame exceeds limit".into()))?;
        let row_bytes = (self.width as usize)
            .checked_mul(channels)
            .ok_or_else(|| Error::Invalid("Frame row overflow".into()))?;
        let stride = row_bytes
            .checked_add(self.x_padding as usize)
            .ok_or_else(|| Error::Invalid("Frame stride overflow".into()))?;
        let image_size = stride
            .checked_mul(self.height as usize)
            .ok_or_else(|| Error::Invalid("Frame byte count overflow".into()))?;
        require(
            self.bytes.len() >= image_size && (self.has_chunks || self.bytes.len() == image_size),
            "Frame byte count does not match geometry",
        )?;
        let mut rgb = Vec::with_capacity(output_size);
        for row in self.bytes[..image_size].chunks_exact(stride) {
            if channels == 1 {
                for &value in &row[..row_bytes] {
                    rgb.extend_from_slice(&[value; 3]);
                }
            } else if self.pixel_format == 0x0218_0015 {
                for pixel in row[..row_bytes].as_chunks::<3>().0 {
                    rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
                }
            } else {
                rgb.extend_from_slice(&row[..row_bytes]);
            }
        }
        Ok(rgb)
    }
}

/// Connect, acquire one complete frame, and release camera control. This blocks;
/// async callers should run it on their blocking worker pool.
#[cfg(feature = "genicam")]
pub fn capture_genicam(config: &GenicamCaptureConfig) -> Result<GenicamFrame> {
    use std::net::SocketAddr;
    use std::time::Duration;
    use telegenic::GenICamera;
    use telegenic::gige::{
        GigeConfig,
        stream::{FrameStatus, PacketSize, PayloadKind, StreamConfig},
    };

    config.validate()?;
    let address: SocketAddrV4 = config
        .device_address
        .parse()
        .map_err(|e| Error::Invalid(format!("Invalid device address: {e}")))?;
    let mut transport = GigeConfig::new(*address.ip());
    transport.addr = SocketAddr::V4(address);
    transport.gvcp_timeout = Duration::from_millis(config.command_timeout_ms);
    transport.retries = config.retries;
    if let Some(ip) = &config.local_interface {
        let ip: Ipv4Addr = ip
            .parse()
            .map_err(|e| Error::Invalid(format!("Invalid interface: {e}")))?;
        transport.local_addr = Some(SocketAddr::V4(SocketAddrV4::new(ip, 0)));
    }
    let map_error = |error| Error::Invalid(format!("GigE Vision: {error}"));
    let mut camera = GenICamera::with_config(transport);
    let result = (|| {
        camera.connect().map_err(map_error)?;
        if let Some(format) = &config.pixel_format {
            camera.set_enum("PixelFormat", format).map_err(map_error)?;
        }
        if let Some(exposure) = config.exposure_time_us {
            camera
                .set_float("ExposureTime", exposure)
                .map_err(map_error)?;
        }
        let payload = camera.get_integer("PayloadSize").map_err(map_error)?;
        require(
            payload > 0 && payload as u64 <= config.max_frame_bytes as u64,
            "Camera payload exceeds configured frame limit",
        )?;
        let mut stream = StreamConfig::new();
        stream.payload_size = Some(payload as usize);
        stream.n_buffers = 2;
        stream.packet_size = PacketSize::Fixed(1500);
        let frame = camera
            .snap(stream, Duration::from_millis(config.frame_timeout_ms))
            .map_err(map_error)?;
        require(
            frame.status == FrameStatus::Complete,
            "Camera frame is incomplete",
        )?;
        let has_chunks = match frame.payload {
            PayloadKind::Image { has_chunks } => has_chunks,
            _ => return Err(Error::Invalid("Camera payload is not an image".into())),
        };
        require(
            frame.width > 0 && frame.height > 0 && frame.data().len() <= config.max_frame_bytes,
            "Camera frame geometry or size is invalid",
        )?;
        Ok(GenicamFrame {
            width: frame.width,
            height: frame.height,
            pixel_format: frame.pixel_format.0,
            pixel_format_name: frame.pixel_format.to_string(),
            x_offset: frame.x_offset,
            y_offset: frame.y_offset,
            x_padding: frame.x_padding,
            y_padding: frame.y_padding,
            has_chunks,
            frame_id: frame.frame_id,
            timestamp_ticks: frame.timestamp_ticks,
            timestamp_ns: (frame.timestamp_ns != 0).then_some(frame.timestamp_ns),
            system_timestamp_ns: frame.system_timestamp_ns,
            bytes: frame.data().to_vec(),
        })
    })();
    camera.disconnect(Duration::from_millis(config.command_timeout_ms));
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> GenicamFrame {
        GenicamFrame {
            width: 2,
            height: 2,
            pixel_format: 0x0108_0001,
            pixel_format_name: "Mono8".into(),
            x_offset: 0,
            y_offset: 0,
            x_padding: 1,
            y_padding: 0,
            has_chunks: false,
            frame_id: 1,
            timestamp_ticks: 0,
            timestamp_ns: None,
            system_timestamp_ns: 0,
            bytes: vec![10, 20, 255, 30, 40, 255],
        }
    }

    #[test]
    fn converts_padding_and_bgr_without_reading_chunks() {
        assert_eq!(
            frame().to_rgb8().unwrap(),
            vec![10, 10, 10, 20, 20, 20, 30, 30, 30, 40, 40, 40]
        );
        let mut frame = frame();
        frame.height = 1;
        frame.x_padding = 0;
        frame.pixel_format = 0x0218_0015;
        frame.has_chunks = true;
        frame.bytes = vec![1, 2, 3, 4, 5, 6, 255];
        assert_eq!(frame.to_rgb8().unwrap(), vec![3, 2, 1, 6, 5, 4]);
        frame.has_chunks = false;
        assert!(frame.to_rgb8().is_err());
        frame.bytes.truncate(5);
        assert!(frame.to_rgb8().is_err());
    }

    #[test]
    #[cfg(feature = "genicam-emulator")]
    fn captures_from_gvcp_gvsp_emulator() {
        use std::net::UdpSocket;
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        use std::time::Duration;
        use telegenic::emulator::{DeviceConfig, GigeDevice};
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let address = socket.local_addr().unwrap();
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let worker = std::thread::spawn(move || {
            let mut device = GigeDevice::new(
                Ipv4Addr::LOCALHOST,
                &DeviceConfig {
                    width: 16,
                    height: 8,
                    ..Default::default()
                },
            );
            let mut data = [0; 2048];
            while !stop.load(Ordering::Acquire) {
                let Ok((len, source)) = socket.recv_from(&mut data) else {
                    continue;
                };
                let reaction = device.handle_datagram(&data[..len], source);
                if let Some(reply) = reaction.reply {
                    socket.send_to(&reply, source).unwrap();
                }
                if reaction.acquisition_started {
                    let destination = device.stream_dest().unwrap();
                    for packet in device.frame_packets(7, &(0..128).collect::<Vec<u8>>()) {
                        socket.send_to(&packet, destination).unwrap();
                    }
                    device.clear_acquisition();
                }
            }
        });
        let config = GenicamCaptureConfig {
            device_address: address.to_string(),
            local_interface: Some("127.0.0.1".into()),
            frame_timeout_ms: 2_000,
            command_timeout_ms: 200,
            retries: 1,
            max_frame_bytes: 1024,
            pixel_format: None,
            exposure_time_us: None,
        };
        let result = capture_genicam(&config);
        stopped.store(true, Ordering::Release);
        worker.join().unwrap();
        let frame = result.unwrap();
        assert_eq!((frame.width, frame.height, frame.frame_id), (16, 8, 7));
        assert_eq!(frame.bytes, (0..128).collect::<Vec<u8>>());
        assert_eq!(frame.to_rgb8().unwrap().len(), 384);
    }
}
