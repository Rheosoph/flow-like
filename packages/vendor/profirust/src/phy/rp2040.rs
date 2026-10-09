use embedded_hal::digital::v2::OutputPin;
use rp2040_hal::uart;

use fugit::RateExtU32;
use rp2040_hal::Clock;

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
        start_tx: crate::time::Instant,
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
            let buffer = core::mem::replace(buffer, (&mut [][..]).into());
            *self = PhyData::Rx { buffer, length: 0 };
        }
    }
}

/// PHY implementation for the [RP2040] microcontroller's UART peripheral
///
/// Available with the `phy-rp2040` feature.
///
/// [RP2040]: https://www.raspberrypi.com/documentation/microcontrollers/rp2040.html
///
/// # Example
/// ```no_run
/// # use rp2040_hal::gpio::{Pin, bank0, FunctionNull, PullNone};
/// # let clocks: rp2040_hal::clocks::ClocksManager = todo!();
/// # let pac: rp2040_hal::pac::Peripherals = todo!();
/// # struct FakePins {
/// #    pub gpio15: Pin<bank0::Gpio15, FunctionNull, PullNone>,
/// #    pub gpio16: Pin<bank0::Gpio16, FunctionNull, PullNone>,
/// #    pub gpio17: Pin<bank0::Gpio17, FunctionNull, PullNone>,
/// # }
/// # let pins: FakePins = todo!();
/// use profirust::{Baudrate, fdl, dp, phy};
/// const BAUDRATE: Baudrate = Baudrate::B19200;
///
/// let uart_pins = (
///     // UART TX (characters sent from RP2040) on pin 1 (GPIO0)
///     pins.gpio16.into_function(),
///     // UART RX (characters received by RP2040) on pin 2 (GPIO1)
///     pins.gpio17.into_function(),
/// );
/// let uart = rp2040_hal::uart::UartPeripheral::new(pac.UART0, uart_pins, &mut pac.RESETS);
///
/// // Pin to toggle the RS485 direction (transmit vs. receive)
/// let dir_pin = pins.gpio15.into_push_pull_output();
///
/// let mut phy_buffer = [0u8; 256];
/// let mut phy = phy::Rp2040Phy::new(
///     uart,
///     dir_pin,
///     &clocks.peripheral_clock,
///     &mut phy_buffer[..],
///     BAUDRATE,
/// )
/// .unwrap();
/// ```
#[derive(Debug)]
pub struct Rp2040Phy<'a, UART, DIR> {
    uart: UART,
    dir_pin: DIR,
    data: PhyData<'a>,
    baudrate: crate::Baudrate,
}

impl<'a, D, P, DIR> Rp2040Phy<'a, uart::UartPeripheral<uart::Enabled, D, P>, DIR>
where
    D: uart::UartDevice,
    P: uart::ValidUartPinout<D>,
    DIR: OutputPin,
{
    pub fn new(
        uart: uart::UartPeripheral<uart::Disabled, D, P>,
        mut dir_pin: DIR,
        per_clock: &rp2040_hal::clocks::PeripheralClock,
        buffer: impl Into<crate::phy::BufferHandle<'a>>,
        baudrate: crate::Baudrate,
    ) -> Result<Self, uart::Error> {
        let uart = uart.enable(
            uart::UartConfig::new(
                u32::try_from(baudrate.to_rate()).unwrap().Hz(),
                uart::DataBits::Eight,
                Some(uart::Parity::Even),
                uart::StopBits::One,
            ),
            per_clock.freq(),
        )?;

        // Go into RX mode.
        dir_pin.set_low().ok().unwrap();

        Ok(Self {
            uart,
            dir_pin,
            data: PhyData::Rx {
                buffer: buffer.into(),
                length: 0,
            },
            baudrate,
        })
    }
}

impl<'a, D, P, DIR> crate::phy::ProfibusPhy
    for Rp2040Phy<'a, uart::UartPeripheral<uart::Enabled, D, P>, DIR>
where
    D: uart::UartDevice,
    P: uart::ValidUartPinout<D>,
    DIR: OutputPin,
{
    fn poll_transmission(&mut self, now: crate::time::Instant) -> bool {
        if let PhyData::Tx {
            buffer,
            length,
            cursor,
            start_tx,
        } = &mut self.data
        {
            if now < *start_tx {
                // We must still wait before beginning transmission (Tset).
                true
            } else if length != cursor {
                let pending = &buffer[*cursor..*length];
                let written = match self.uart.write_raw(pending) {
                    Ok(b) => pending.len() - b.len(),
                    Err(nb::Error::WouldBlock) => 0,
                    Err(nb::Error::Other(_)) => unreachable!(),
                };
                debug_assert!(written <= *length - *cursor);
                *cursor += written;
                true
            } else {
                let busy = self.uart.uart_is_busy();
                if !busy {
                    self.data.make_rx();
                    self.dir_pin.set_low().ok().unwrap();
                }
                busy
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
                }
                let (length, res) = f(&mut buffer[..]);
                if length == 0 {
                    // Don't transmit anything.
                    return res;
                }

                // We enable the transmitter here and then wait for Tset before poll_transmission()
                // will start scheduling bytes for transmission.
                self.dir_pin.set_high().ok().unwrap();
                // TODO: Tset is not always 1 bit time
                let t_set = self.baudrate.bits_to_time(1);

                let buffer = core::mem::replace(buffer, (&mut [][..]).into());
                self.data = PhyData::Tx {
                    buffer,
                    length,
                    cursor: 0,
                    start_tx: now + t_set,
                };
                res
            }
        }
    }

    fn receive_data<F, R>(&mut self, _now: crate::time::Instant, f: F) -> R
    where
        F: FnOnce(&[u8]) -> (usize, R),
    {
        match &mut self.data {
            PhyData::Tx { .. } => panic!("receive_data() while transmitting!"),
            PhyData::Rx { buffer, length } => {
                *length += match self.uart.read_raw(&mut buffer[*length..]) {
                    Ok(l) => l,
                    Err(nb::Error::WouldBlock) => 0,
                    Err(nb::Error::Other(_)) => {
                        // TODO: handle uart errors
                        0
                    }
                };
                debug_assert!(*length <= buffer.len());
                let (drop, res) = f(&buffer[..*length]);
                match drop {
                    0 => (),
                    d if d == *length => *length = 0,
                    d => {
                        // TODO: Properly implement partial buffer drops here as well. It isn't
                        // that important because this shouldn't really ever happen on a
                        // microcontroller, but having it may be needed somewhere someday anyway...
                        log::warn!(
                            "ignoring partial drop of receive buffer ({} of {})",
                            d,
                            *length
                        );
                        *length = 0;
                    }
                }
                res
            }
        }
    }
}
