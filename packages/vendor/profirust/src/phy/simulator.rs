use std::sync;

#[derive(Debug)]
struct CapturedTelegram {
    sender: &'static str,
    timestamp: crate::time::Instant,
    index: usize,
    length: usize,
}

#[derive(Debug)]
struct SimulatorBus {
    baudrate: crate::Baudrate,
    telegrams: Vec<CapturedTelegram>,
    stream: Vec<u8>,
    bus_time: crate::time::Instant,
    /// Which master is currently holding the token.  We use this to verify correct timing.
    token_master: Option<u8>,
}

impl SimulatorBus {
    pub fn new(baudrate: crate::Baudrate) -> Self {
        Self {
            baudrate,
            telegrams: Vec::new(),
            stream: Vec::new(),
            bus_time: crate::time::Instant::ZERO,
            token_master: None,
        }
    }

    pub fn current_cursor(&self) -> usize {
        if self.telegrams.len() == 0 {
            return 0;
        }

        let last_telegram = self.telegrams.last().unwrap();

        let last_telegram_tx_duration = self.bus_time - last_telegram.timestamp;
        let last_telegram_tx_bytes =
            usize::try_from(self.baudrate.time_to_bits(last_telegram_tx_duration) / 11).unwrap();

        self.stream.len() - last_telegram.length + last_telegram_tx_bytes.min(last_telegram.length)
    }

    pub fn pending_bytes(&self, cursor: usize) -> &[u8] {
        &self.stream[cursor..self.current_cursor()]
    }

    pub fn is_active(&self) -> Option<&'static str> {
        if self.telegrams.len() == 0 {
            return None;
        }

        let last_telegram = self.telegrams.last().unwrap();

        let last_telegram_tx_duration = self.bus_time - last_telegram.timestamp;
        let last_telegram_tx_bytes =
            usize::try_from(self.baudrate.time_to_bits(last_telegram_tx_duration) / 11).unwrap();

        if last_telegram_tx_bytes < last_telegram.length {
            Some(last_telegram.sender)
        } else {
            None
        }
    }

    pub fn enqueue_telegram(&mut self, name: &'static str, mut data: Vec<u8>) {
        if let Some(active_sender) = self.is_active() {
            panic!(
                "\"{}\" attempted transmission while \"{}\" is still sending!",
                name, active_sender
            );
        }

        // Drop out early if nothing needs to be sent.
        if data.len() == 0 {
            return;
        }

        let sa = if let Some(Ok((decoded, length))) = crate::fdl::Telegram::deserialize(&data) {
            if length != data.len() {
                panic!("Enqueued more than one deserializable telegram? {data:?}");
            }
            match decoded {
                crate::fdl::Telegram::Token(crate::fdl::TokenTelegram { da, sa }) => {
                    self.token_master = Some(da);
                    Some(sa)
                }
                crate::fdl::Telegram::Data(crate::fdl::DataTelegram { h, pdu }) => Some(h.sa),
                _ => None,
            }
        } else {
            None
        };

        #[derive(Debug)]
        enum DelayType {
            Tid1,
            Tid2,
            Tsdr,
        }

        let mut delay_type: Option<DelayType> = None;
        // Ensure that at least 11 bit times were left between two consecutive transmissions.
        if let Some(last_telegram_and_pause) = self.telegrams.last().map(|t| {
            let min_delay = if let Some(t) = self.get_telegram(t) {
                if sa == self.token_master && t.source_address() == sa {
                    delay_type = Some(DelayType::Tid2);
                    // TODO: Update this to be a value calculated from FDL parameters
                    33
                } else if sa == self.token_master && t.source_address() != sa {
                    delay_type = Some(DelayType::Tid1);
                    // TODO: Update this to be a value calculated from FDL parameters
                    33
                } else {
                    /* sa != self.token_master */
                    delay_type = Some(DelayType::Tsdr);
                    // TODO: Update this to be a value calculated from FDL parameters
                    11
                }
            } else {
                log::debug!(
                    "Received undeciperable transmission: {:?}",
                    self.get_telegram_data(t)
                );
                // We don't know what is being transmitted, but at least one symbol time should be
                // required anyway.
                11
            };
            t.timestamp
                + self
                    .baudrate
                    .bits_to_time(u32::try_from(t.length).unwrap() * 11 + min_delay)
        }) {
            if self.bus_time < last_telegram_and_pause {
                panic!(
                    "\"{}\" did not leave appropriate {:?} delay time before transmission!",
                    name, delay_type
                );
            }
        }

        if let Some(Ok(decoded)) = crate::fdl::Telegram::deserialize(&data) {
            log::trace!("{:8} {}: {:?}", self.bus_time.total_micros(), name, decoded);
        } else {
            let data_fmt = data
                .iter()
                .map(|b| format!("0x{:02x}", b))
                .collect::<Vec<_>>()
                .join(" ");
            log::trace!("{:8} {}: {}", self.bus_time.total_micros(), name, data_fmt);
        }

        let telegram = CapturedTelegram {
            sender: name,
            timestamp: self.bus_time,
            index: self.stream.len(),
            length: data.len(),
        };
        self.stream.append(&mut data);
        self.telegrams.push(telegram);
    }

    pub fn print_log(&self) {
        for t in &self.telegrams {
            print!("{:16} {:>12}:", t.timestamp.total_micros(), t.sender);
            for b in &self.stream[t.index..t.index + t.length] {
                print!(" 0x{:02x}", b);
            }
            println!();
        }
    }

    pub fn get_telegram_data(&self, t: &CapturedTelegram) -> &[u8] {
        &self.stream[t.index..t.index + t.length]
    }

    pub fn get_telegram(&self, t: &CapturedTelegram) -> Option<crate::fdl::Telegram> {
        crate::fdl::Telegram::deserialize(self.get_telegram_data(t))
            .map(Result::ok)
            .flatten()
            .map(|(t, _)| t)
    }

    // phy needs to find out what data is still pending for it
    // phy needs to find out whether it is still transmitting
    // phy needs to be able to submit new data
    //
    // bytes should become available with correct timing (i.e. no artificial framing)
    // bus should immediately panic on collision (at least for now)
}

#[derive(Debug)]
pub struct SimulatorPhy {
    bus: sync::Arc<sync::Mutex<SimulatorBus>>,
    cursor: usize,
    name: &'static str,
}

impl SimulatorPhy {
    pub fn new(baudrate: crate::Baudrate, name: &'static str) -> Self {
        Self {
            bus: sync::Arc::new(sync::Mutex::new(SimulatorBus::new(baudrate))),
            cursor: 0,
            name,
        }
    }

    pub fn duplicate(&self, name: &'static str) -> Self {
        Self {
            bus: self.bus.clone(),
            cursor: 0,
            name,
        }
    }

    pub fn set_bus_time(&self, time: crate::time::Instant) {
        self.bus.lock().unwrap().bus_time = time;
    }

    pub fn advance_bus_time(&self, dur: crate::time::Duration) {
        self.bus.lock().unwrap().bus_time += dur;
    }

    pub fn bus_time(&self) -> crate::time::Instant {
        self.bus.lock().unwrap().bus_time
    }

    pub fn print_bus_log(&self) {
        self.bus.lock().unwrap().print_log();
    }

    pub fn iter_until_matching<'a, F>(
        &'a mut self,
        timestep: crate::time::Duration,
        f: F,
    ) -> SimulationIterator<'a, F>
    where
        F: FnMut(crate::fdl::Telegram) -> bool,
    {
        SimulationIterator {
            timeout: self.bus_time() + crate::time::Duration::from_secs(10),
            phy: self,
            timestep,
            matcher: f,
        }
    }

    pub fn advance_bus_time_min_tsdr(&self) {
        let mut bus = self.bus.lock().unwrap();
        let min_tsdr = bus.baudrate.bits_to_time(11);
        bus.bus_time += min_tsdr;
    }
}

impl crate::phy::ProfibusPhy for SimulatorPhy {
    fn poll_transmission(&mut self, _now: crate::time::Instant) -> bool {
        let bus = self.bus.lock().unwrap();
        bus.is_active() == Some(self.name)
    }

    fn transmit_data<F, R>(&mut self, _now: crate::time::Instant, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        let mut bus = self.bus.lock().unwrap();

        let mut buffer = vec![0u8; 256];
        let (length, res) = f(&mut buffer);
        buffer.truncate(length);

        bus.enqueue_telegram(self.name, buffer);
        self.cursor += length;

        res
    }

    fn receive_data<F, R>(&mut self, now: crate::time::Instant, f: F) -> R
    where
        F: FnOnce(&[u8]) -> (usize, R),
    {
        if self.poll_transmission(now) {
            panic!(
                "\"{}\" attempted to receive while it was still transmitting!",
                self.name
            );
        }

        let bus = self.bus.lock().unwrap();
        let pending = bus.pending_bytes(self.cursor);

        let (drop, res) = f(pending);
        assert!(
            drop <= pending.len(),
            "\"{}\" attempted to drop more pending bytes than it has!",
            self.name,
        );
        self.cursor += drop;

        res
    }
}

pub struct SimulationIterator<'a, F> {
    phy: &'a mut SimulatorPhy,
    timestep: crate::time::Duration,
    timeout: crate::time::Instant,
    matcher: F,
}

impl<'a, F> Iterator for SimulationIterator<'a, F>
where
    F: FnMut(crate::fdl::Telegram) -> bool,
{
    type Item = crate::time::Instant;

    fn next(&mut self) -> Option<Self::Item> {
        use crate::phy::ProfibusPhy;

        self.phy.advance_bus_time(self.timestep);
        let now = self.phy.bus_time();
        if now >= self.timeout {
            panic!("Timeout while waiting for a certain telegram to show up!");
        }
        if !self.phy.poll_transmission(now) {
            let is_matching = self
                .phy
                .receive_telegram(now, |t| (self.matcher)(t))
                .unwrap_or(false);
            if is_matching {
                return None;
            }
        }
        Some(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phy::ProfibusPhy;

    #[test]
    fn send_and_receive() {
        let mut phy1 = SimulatorPhy::new(crate::Baudrate::B19200, "phy1");
        let mut phy2 = phy1.duplicate("phy2");

        let mut now = crate::time::Instant::ZERO;

        let data = &[0xde, 0xad, 0xbe, 0xef, 0x12, 0x34];
        phy1.transmit_data(now, |buf| {
            buf[..data.len()].copy_from_slice(data);
            (data.len(), ())
        });

        phy2.receive_data(now, |buf| {
            assert_eq!(buf.len(), 0);
            (0, ())
        });

        now += crate::time::Duration::from_millis(100);
        phy1.set_bus_time(now);

        phy2.receive_data(now, |buf| {
            assert_eq!(buf, data);
            (4, ())
        });
        phy2.receive_data(now, |buf| {
            assert_eq!(buf.len(), data.len() - 4);
            (buf.len(), ())
        });

        let data = &[0xc0, 0xff, 0xee];
        phy2.transmit_data(now, |buf| {
            buf[..data.len()].copy_from_slice(data);
            (data.len(), ())
        });

        now += crate::time::Duration::from_millis(100);
        phy1.set_bus_time(now);

        phy1.receive_data(now, |buf| {
            assert_eq!(buf, data);
            (buf.len(), ())
        });

        phy1.print_bus_log();
    }
}
