use std::borrow::Cow;
use std::io;

#[derive(Debug)]
enum PhyData<'a> {
    Rx {
        buffer: crate::phy::BufferHandle<'a>,
        length: usize,
    },
    Tx {
        buffer: crate::phy::BufferHandle<'a>,
        length: usize,
        cursor: usize,
    },
}

impl PhyData<'_> {
    pub fn is_rx(&self) -> bool {
        match self {
            PhyData::Rx { .. } => true,
            _ => false,
        }
    }

    pub fn is_tx(&self) -> bool {
        match self {
            PhyData::Tx { .. } => true,
            _ => false,
        }
    }

    pub fn make_rx(&mut self) {
        if let PhyData::Tx { buffer, .. } = self {
            let buffer = std::mem::replace(buffer, [].into());
            *self = PhyData::Rx { buffer, length: 0 };
        }
    }
}

/// Platform-independent PHY implementation for serial port devices
///
/// Available with the `phy-serial` feature.
///
/// This PHY implementation is mainly meant for use with USB-RS485 converters, for applications
/// running within a general purpose operating system.
///
/// Between USB, the converter device, and the OS, large delays are introduced into the
/// communication path.  The PROFIBUS timing paramters need to be adjusted to accound for this.
/// Here are a few points to consider:
///
/// - Particularly FT232-based devices come with a 16ms latency by default, unless explicitly
///   configured for low-latency (configuration for [Linux][ftdi-latency-linux] or
///   [Windows][ftdi-latency-win]).
/// - You should adjust your bus poll cycle time (called T<sub>SLEEP</sub> here) to be slightly
///   longer than the roundtrip time of an average data-exchange communication.
/// - The T<sub>SL</sub> (slot time) PROFIBUS timing parameter of the bus needs to be much longer
///   than usual.  A value of 2 * T<sub>SLEEP</sub> + 1ms has experimentally proven itself to work
///   quite well.
/// - At least one or two retries should be permitted to cope with the non-realtime nature of the
///   general purpose operating system.  This can be facilitated by setting `max_retry_limit` to 2
///   or 3.
///
/// # Measuring roundtrip time
/// You can measure the roundtrip time using the `debug-measure-roundtrip` crate-feature.  It will
/// debug-log the roundtrip time for each peripheral communication.  The information gained from
/// this measurement can then be used to find an appropriate T<sub>SLEEP</sub> and T<sub>SL</sub>.
///
/// If you are struggling to get any communcation working, try starting with a very high
/// T<sub>SL</sub> (slot time) value (e.g. 20ms = 10000 bits at 500kBaud).
///
/// # Empirical Recommendations
/// The following values were determined to yield stable communication using various popular
/// USB-RS485 converters (all tested devices were either based on the CH341 or FT232R chips).
///
/// | Baudrate | T<sub>SLEEP</sub> | T<sub>SL</sub> (slot time) |
/// | ---: | ---: | ---: |
/// | 19.2 kBaud | 10 ms | 30 ms = 576 bits |
/// | 93.75 kBaud | 3.5 ms | 8 ms = 750 bits |
/// | 187.5 kBaud | 3.5 ms | 8 ms = 1500 bits |
/// | 500 kBaud | 3.5 ms | 8 ms = 4000 bits |
///
/// [ftdi-latency-win]: https://www.ftdichip.com/Support/Knowledgebase/index.html?settingacustomdefaultlaten.htm
/// [ftdi-latency-linux]: https://askubuntu.com/questions/696593/reduce-request-latency-on-an-ftdi-ubs-to-rs-232-adapter
///
/// # Example
/// ```no_run
/// use profirust::{Baudrate, fdl, dp, phy};
/// const BAUDRATE: Baudrate = Baudrate::B500000;
/// # let mut dp_master = dp::DpMaster::new(vec![]);
///
/// let mut fdl = fdl::FdlActiveStation::new(
///     fdl::ParametersBuilder::new(0x02, BAUDRATE)
///         // Increased slot time due to USB latency
///         .slot_bits(4000)
///         .build_verified(&dp_master)
/// );
///
/// let mut phy = phy::SerialPortPhy::new("/dev/ttyUSB0", fdl.parameters().baudrate);
/// // Sleep time for the bus poll loop
/// let sleep_time = std::time::Duration::from_micros(3500);
/// ```
pub struct SerialPortPhy {
    port: Box<dyn serialport::SerialPort>,
    data: PhyData<'static>,
    last_rx: Option<crate::time::Instant>,
}

impl SerialPortPhy {
    pub fn new<'a, P: Into<Cow<'a, str>>>(serial_port: P, baudrate: crate::Baudrate) -> Self {
        Self::new_inner(serial_port.into(), baudrate)
    }

    fn new_inner(serial_port: Cow<'_, str>, baudrate: crate::Baudrate) -> Self {
        use serialport::SerialPort;

        #[allow(unused_mut)]
        let mut port = serialport::new(serial_port, u32::try_from(baudrate.to_rate()).unwrap())
            .data_bits(serialport::DataBits::Eight)
            .flow_control(serialport::FlowControl::None)
            .parity(serialport::Parity::Even)
            .stop_bits(serialport::StopBits::One)
            .open_native()
            .unwrap();

        assert_eq!(
            u64::from(port.baud_rate().unwrap()),
            baudrate.to_rate(),
            "baudrate not configured correctly"
        );

        #[cfg(target_os = "linux")]
        serialport_low_latency::enable_low_latency(&mut port).unwrap();

        let buffer = crate::phy::BufferHandle::from(vec![0u8; 512]);

        Self {
            port: Box::new(port),
            data: PhyData::Rx { buffer, length: 0 },
            last_rx: None,
        }
    }

    fn write(port: &mut dyn serialport::SerialPort, buffer: &[u8]) -> io::Result<usize> {
        // TODO: Technically we need to ensure this never blocks
        port.write(buffer)
    }

    fn get_output_queue(&mut self) -> io::Result<usize> {
        Ok(usize::try_from(self.port.bytes_to_write().unwrap()).unwrap())
    }

    fn read(port: &mut dyn serialport::SerialPort, buffer: &mut [u8]) -> io::Result<usize> {
        let bytes_to_read = port.bytes_to_read().unwrap();
        if bytes_to_read == 0 {
            Ok(0)
        } else {
            // TODO: Check that this will always return the available bytes immediately
            port.read(buffer)
        }
    }
}

impl crate::phy::ProfibusPhy for SerialPortPhy {
    fn poll_transmission(&mut self, _now: crate::time::Instant) -> bool {
        if let PhyData::Tx {
            buffer,
            length,
            cursor,
        } = &mut self.data
        {
            if length != cursor {
                // Need to submit more data.
                let written = Self::write(&mut *self.port, &buffer[*cursor..*length]).unwrap();
                debug_assert!(written <= *length - *cursor);
                *cursor += written;
                true
            } else {
                // Everything was submitted already.
                let queued = self.get_output_queue().unwrap();
                if queued == 0 {
                    // All data was sent.
                    self.data.make_rx();
                    false
                } else {
                    // Still sending.
                    true
                }
            }
        } else {
            false
        }
    }

    fn transmit_data<F, R>(&mut self, now: crate::time::Instant, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        match &mut self.data {
            PhyData::Tx { .. } => panic!("transmit_data() while already transmitting!"),
            PhyData::Rx {
                buffer,
                length: receive_length,
            } => {
                if *receive_length != 0 {
                    log::warn!(
                        "{} bytes in the receive buffer and we go into transmission?",
                        receive_length
                    );
                    #[cfg(feature = "std")]
                    {
                        let buffer_string = buffer[..*receive_length]
                            .iter()
                            .map(|b| format!("{b:02X}"))
                            .collect::<Vec<_>>()
                            .join(" ");
                        if let Some(last_rx) = self.last_rx {
                            log::warn!(
                                "Last data was received {} us ago",
                                (now - last_rx).total_micros()
                            );
                        }
                        log::warn!("Receive buffer content: {buffer_string}");
                    }
                }
                let (length, res) = f(&mut buffer[..]);
                if length == 0 {
                    // Don't transmit anything.
                    return res;
                }
                let cursor = Self::write(&mut *self.port, &buffer[..length]).unwrap();
                debug_assert!(cursor <= length);
                let buffer = std::mem::replace(buffer, [].into());
                self.data = PhyData::Tx {
                    buffer,
                    length,
                    cursor,
                };
                res
            }
        }
    }

    fn receive_data<F, R>(&mut self, now: crate::time::Instant, f: F) -> R
    where
        F: FnOnce(&[u8]) -> (usize, R),
    {
        match &mut self.data {
            PhyData::Tx { .. } => panic!("receive_data() while transmitting!"),
            PhyData::Rx { buffer, length } => {
                let last_length = *length;
                *length += Self::read(&mut *self.port, &mut buffer[*length..]).unwrap();
                if last_length != *length {
                    self.last_rx = Some(now);
                }
                debug_assert!(*length <= buffer.len());
                let (drop, res) = f(&buffer[..*length]);
                match drop {
                    0 => (),
                    d if d == *length => *length = 0,
                    d => {
                        assert!(d < *length);
                        for i in 0..(*length - d) {
                            buffer[i] = buffer[i + d];
                        }
                        *length -= d;
                    }
                }
                res
            }
        }
    }
}
