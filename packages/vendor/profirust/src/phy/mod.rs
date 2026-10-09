//! PHY - Physical layer abstraction
//!
//! The PHY layer is an abstraction over the various hardware that `profirust` supports for
//! PROFIBUS communication.  You will need to enable the corresponding crate features for your PHY
//! implementation.  Here is a list:
//!
//! - `phy-serial`: Platform-independent PHY implementation for serial port devices
//! - `phy-linux`: Linux userspace PHY implementation for UART TTY devices
//! - `phy-rp2040`: PHY implementation for UART of the RP2040
//! - `phy-simulator`: Simulator PHY implementation for `profirust` testing with a simulated bus

#[cfg(feature = "phy-linux")]
mod linux;
#[cfg(feature = "phy-linux")]
pub use linux::LinuxRs485Phy;

#[cfg(feature = "phy-serial")]
mod serial;
#[cfg(feature = "phy-serial")]
pub use serial::SerialPortPhy;

#[cfg(feature = "phy-simulator")]
pub mod simulator;
#[cfg(feature = "phy-simulator")]
pub use simulator::SimulatorPhy;

#[cfg(feature = "phy-rp2040")]
mod rp2040;
#[cfg(feature = "phy-rp2040")]
pub use rp2040::Rp2040Phy;

/// Type alias for the message buffer used by some PHY implementations
pub type BufferHandle<'a> = managed::ManagedSlice<'a, u8>;

/// Generic abstraction for `profirust` PHY implementations
pub trait ProfibusPhy {
    /// Poll an ongoing transmission.
    ///
    /// Should return `true` while the transmission is still in progress and `false` once it has
    /// been completed.
    ///
    /// While this function returns `true`, calling any of the `transmit_*()` or `receive_*()`
    /// functions may panic.
    fn poll_transmission(&mut self, now: crate::time::Instant) -> bool;

    /// Schedule transmission of some data.
    ///
    /// The data is written by the closure `f` into the buffer passed to it.  `f` then returns how
    /// many bytes were written.  Only this many bytes must be transmitted.
    ///
    /// **Important**: This function must not block on the actual transmission!
    ///
    /// # Panics
    /// This function may panic when a transmission is already ongoing.
    fn transmit_data<F, R>(&mut self, now: crate::time::Instant, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> (usize, R);

    /// Schedule transmission of a telegram.
    ///
    /// The closure `f` may (or may not) call one of the methods of
    /// [`fdl::TelegramTx`][`crate::fdl::TelegramTx`] to schedule transmission of a telegram.  This
    /// function returns `Some(n)` (`n` = number of bytes for transmission) when a telegram was
    /// scheduled and `None` otherwise.
    ///
    /// **Important**: This function must not block on the actual transmission!
    ///
    /// # Panics
    /// This function may panic when a transmission is already ongoing.
    fn transmit_telegram<F>(
        &mut self,
        now: crate::time::Instant,
        f: F,
    ) -> Option<crate::fdl::TelegramTxResponse>
    where
        F: FnOnce(crate::fdl::TelegramTx) -> Option<crate::fdl::TelegramTxResponse>,
    {
        self.transmit_data(now, |buffer| {
            let ttx = crate::fdl::TelegramTx::new(buffer);
            let response = f(ttx);
            if let Some(response) = response {
                let bytes_sent = response.bytes_sent();

                if let Some(Ok(t)) = crate::fdl::Telegram::deserialize(buffer) {
                    log::trace!("PHY TX {:?}", t);
                } else {
                    log::trace!("PHY TX {:?} (invalid!)", &buffer[..bytes_sent]);
                }

                (bytes_sent, Some(response))
            } else {
                (0, None)
            }
        })
    }

    /// Try receiving some data.
    ///
    /// The closure `f` will process all received data and return how many bytes should be dropped
    /// from the receive buffer.
    ///
    /// **Important**: This function must not block on the actually receiving data and should
    /// instead return an empty buffer if no data is available!
    ///
    /// # Panics
    /// This function may panic when a transmission is ongoing.
    fn receive_data<F, R>(&mut self, now: crate::time::Instant, f: F) -> R
    where
        F: FnOnce(&[u8]) -> (usize, R);

    /// Try receiving a telegram.
    ///
    /// When a full and correct telegram was received, the closure `f` is called to process it.
    ///
    /// When `f()` is called, there is no guarantee that no more telegrams are pending after this
    /// one in the receive buffer.  Call `receive_telegram()` multiple times to process them all or
    /// use `receive_all_telegrams()` instead.
    ///
    /// **Important**: This function must not block on the actually receiving a telegram and should
    /// return `None` in case no full telegram was received yet!
    ///
    /// # Panics
    /// This function may panic when a transmission is ongoing.
    fn receive_telegram<F, R>(&mut self, now: crate::time::Instant, f: F) -> Option<R>
    where
        F: FnOnce(crate::fdl::Telegram) -> R,
    {
        self.receive_data(now, |buffer| {
            match crate::fdl::Telegram::deserialize(buffer) {
                // Discard all received data on error.
                Some(Err(_)) => (buffer.len(), None),
                Some(Ok((telegram, length))) => {
                    log::trace!("PHY RX {:?}", telegram);
                    if length != buffer.len() {
                        log::trace!("Received more than one telegram at once!");
                    }
                    (length, Some(f(telegram)))
                }
                // Don't drop any bytes yet if the telegram isn't complete.
                None => (0, None),
            }
        })
    }

    /// Try receiving all pending telegrams.
    ///
    /// This function calls `f()` for each valid telegram in the receive buffer.  The second
    /// parameter to `f()` is a boolean indicating whether this is the latest telegram that was
    /// received (`is_last_telegram`).  There are a few caveats:
    ///
    /// - The return value of `f()` is only forwarded for the last call of `f()` where
    ///   `is_last_telegram` was `true`.
    /// - It may be possible that no call of `f()` has `is_last_telegram==true`.  This happens when
    ///   the final data in the receive buffer is not (yet) a valid telegram. `None` is returned in this
    ///   case.
    ///
    /// **Important**: This function must not block on the actually receiving a telegram and should
    /// return `None` in case no full telegram was received yet!
    ///
    /// # Panics
    /// This function may panic when a transmission is ongoing.
    fn receive_all_telegrams<F, R>(&mut self, now: crate::time::Instant, mut f: F) -> Option<R>
    where
        F: FnMut(crate::fdl::Telegram, bool) -> R,
    {
        // TODO: Limit this loop in some way?  Or is it enough to rely on the receive-buffer being
        // finite?
        loop {
            let (is_last, res) = self.receive_data(now, |buffer| {
                match crate::fdl::Telegram::deserialize(buffer) {
                    // Discard all received data on error.
                    Some(Err(_)) => (buffer.len(), (true, None)),
                    Some(Ok((telegram, length))) => {
                        log::trace!("PHY RX {:?}", telegram);
                        let telegram_is_last = length == buffer.len();
                        let res = f(telegram, telegram_is_last);
                        (length, (telegram_is_last, Some(res)))
                    }
                    // Don't drop any bytes yet if the telegram isn't complete.
                    None => (0, (true, None)),
                }
            });
            if is_last {
                return res;
            } else {
                log::trace!("Received more than one telegram at once, trying to keep up!");
            }
        }
    }

    /// Poll for the current amount of bytes waiting in the receive buffer.
    ///
    /// The receive buffer is not emptied by this call.
    ///
    /// **Important**: This function must not block on the actually receiving data and should
    /// instead return 0 if no data is available!
    ///
    /// # Panics
    /// This function may panic when a transmission is ongoing.
    fn poll_pending_received_bytes(&mut self, now: crate::time::Instant) -> usize {
        self.receive_data(now, |buf| (0, buf.len()))
    }
}
