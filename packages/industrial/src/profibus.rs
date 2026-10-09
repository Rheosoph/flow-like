use crate::{Result, require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfibusMode {
    Stop,
    Clear,
    Operate,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ProfibusPeripheralConfig {
    pub address: u8,
    pub ident_number: u16,
    /// Device-specific bytes derived from its GSD and selected modules.
    pub user_parameters: Vec<u8>,
    pub configuration: Vec<u8>,
    pub input_bytes: usize,
    pub initial_outputs: Vec<u8>,
    pub max_tsdr_bits: u16,
    pub fail_safe: bool,
    #[serde(default)]
    pub groups: u8,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ProfibusConfig {
    /// Serial device connected to an RS-485 adapter with automatic direction control.
    pub port: String,
    pub baud_rate: u32,
    pub master_address: u8,
    pub highest_station_address: u8,
    pub slot_bits: u16,
    pub poll_interval_us: u64,
    pub watchdog_ms: u64,
    pub retry_limit: u8,
    pub mode: ProfibusMode,
    pub peripherals: Vec<ProfibusPeripheralConfig>,
}
impl ProfibusConfig {
    pub fn validate(&self) -> Result<()> {
        require(
            !self.port.is_empty() && self.port.len() <= 4096,
            "A serial device path is required",
        )?;
        require(
            [
                9600, 19200, 31250, 45450, 93750, 187500, 500000, 1500000, 3000000, 6000000,
                12000000,
            ]
            .contains(&self.baud_rate),
            "Unsupported PROFIBUS baud rate",
        )?;
        require(
            self.master_address <= 125
                && self.highest_station_address > self.master_address
                && self.highest_station_address <= 126,
            "PROFIBUS station addresses are outside the configured range",
        )?;
        require(
            (10..=100_000).contains(&self.poll_interval_us),
            "PROFIBUS poll interval must be in 10..100000 microseconds",
        )?;
        let minimum_slot = match self.baud_rate {
            0..=187500 => 100,
            500000 => 200,
            1500000 => 300,
            3000000 => 400,
            6000000 => 600,
            _ => 1000,
        };
        require(
            self.slot_bits >= minimum_slot,
            "PROFIBUS slot time is below the baud-rate minimum",
        )?;
        require(
            u64::from(self.slot_bits) * 1_000_000 / u64::from(self.baud_rate)
                >= self.poll_interval_us * 2,
            "PROFIBUS slot time must allow at least two bus polls",
        )?;
        require(
            (10..=650_000).contains(&self.watchdog_ms),
            "PROFIBUS watchdog must be in 10..650000 ms",
        )?;
        require(
            (1..=15).contains(&self.retry_limit),
            "PROFIBUS retry limit must be in 1..15",
        )?;
        require(
            !self.peripherals.is_empty() && self.peripherals.len() <= 125,
            "Configure 1..125 PROFIBUS peripherals",
        )?;
        let mut addresses = std::collections::HashSet::new();
        for p in &self.peripherals {
            require(
                p.address <= 125 && p.address != self.master_address && addresses.insert(p.address),
                "PROFIBUS peripheral addresses must be unique and differ from the master",
            )?;
            require(
                p.input_bytes <= 244 && p.initial_outputs.len() <= 244,
                "PROFIBUS process images are limited to 244 bytes per direction",
            )?;
            require(
                p.user_parameters.len() <= 237
                    && !p.configuration.is_empty()
                    && p.configuration.len() <= 244,
                "PROFIBUS GSD parameter/configuration bytes exceed telegram bounds",
            )?;
            require(
                u32::from(p.max_tsdr_bits) + 15 <= u32::from(self.slot_bits),
                "PROFIBUS slot time is shorter than a peripheral response time",
            )?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ProfibusPeripheralState {
    pub address: u8,
    pub live: bool,
    pub running: bool,
    pub inputs: Vec<u8>,
    pub outputs: Vec<u8>,
    pub diagnostic_flags: Option<u16>,
    pub extended_diagnostics: Vec<u8>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ProfibusSnapshot {
    pub sequence: u64,
    pub cycle: u64,
    pub timestamp_ms: u64,
    pub mode: ProfibusMode,
    pub peripherals: Vec<ProfibusPeripheralState>,
    pub error: Option<String>,
    pub stopped: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ProfibusOutput {
    pub address: u8,
    pub values: Vec<u8>,
}

#[cfg(feature = "execute")]
pub use execution::ProfibusMaster;

#[cfg(feature = "execute")]
mod execution {
    use super::*;
    use crate::Error;
    use profirust::{Baudrate, dp, fdl, phy::ProfibusPhy};
    use std::{
        io::{Read, Write},
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };
    use tokio::sync::{mpsc, oneshot, watch};

    enum Command {
        Write(ProfibusOutput, oneshot::Sender<Result<()>>),
        Mode(ProfibusMode, oneshot::Sender<Result<()>>),
    }
    pub struct ProfibusMaster {
        commands: mpsc::Sender<Command>,
        state: watch::Receiver<ProfibusSnapshot>,
        stop: Arc<AtomicBool>,
        thread: std::thread::Thread,
        done: watch::Receiver<bool>,
    }
    impl Drop for ProfibusMaster {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            self.thread.unpark();
        }
    }
    impl ProfibusMaster {
        pub async fn connect(config: ProfibusConfig) -> Result<Self> {
            config.validate()?;
            let (commands, receiver) = mpsc::channel(32);
            let (state_sender, state) = watch::channel(empty_snapshot(config.mode));
            let (done_sender, done) = watch::channel(false);
            let (ready_sender, ready) = oneshot::channel();
            let stop = Arc::new(AtomicBool::new(false));
            let thread_stop = stop.clone();
            let thread = std::thread::Builder::new()
                .name("flow-profibus".into())
                .spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        run(config, receiver, &state_sender, thread_stop, ready_sender)
                    }));
                    let error = match result {
                        Ok(Ok(())) => None,
                        Ok(Err(e)) => Some(e.to_string()),
                        Err(_) => {
                            Some("PROFIBUS worker stopped after an internal driver failure".into())
                        }
                    };
                    state_sender.send_modify(|s| {
                        s.stopped = true;
                        s.error = error;
                        s.sequence += 1;
                    });
                    let _ = done_sender.send(true);
                })?;
            let client = Self {
                commands,
                state,
                stop,
                thread: thread.thread().clone(),
                done,
            };
            match tokio::time::timeout(Duration::from_secs(10), ready).await {
                Ok(Ok(result)) => {
                    result?;
                    Ok(client)
                }
                Ok(Err(_)) => Err(Error::Invalid(
                    client
                        .state
                        .borrow()
                        .error
                        .clone()
                        .unwrap_or_else(|| "PROFIBUS worker failed to start".into()),
                )),
                Err(_) => Err(Error::Timeout),
            }
        }
        pub fn snapshot(&self) -> Result<ProfibusSnapshot> {
            let snapshot = self.state.borrow().clone();
            if let Some(error) = &snapshot.error {
                return Err(Error::Invalid(error.clone()));
            }
            require(!snapshot.stopped, "PROFIBUS master is stopped")?;
            Ok(snapshot)
        }
        /// Receivers hold only the newest snapshot, so a slow handler cannot delay the bus.
        pub fn subscribe(&self) -> watch::Receiver<ProfibusSnapshot> {
            self.state.clone()
        }
        pub async fn write(&self, output: ProfibusOutput) -> Result<()> {
            require(
                output.values.len() <= 244,
                "PROFIBUS output exceeds 244 bytes",
            )?;
            self.send(|ack| Command::Write(output, ack)).await
        }
        pub async fn set_mode(&self, mode: ProfibusMode) -> Result<()> {
            self.send(|ack| Command::Mode(mode, ack)).await
        }
        async fn send(
            &self,
            command: impl FnOnce(oneshot::Sender<Result<()>>) -> Command,
        ) -> Result<()> {
            self.snapshot()?;
            let (ack, response) = oneshot::channel();
            self.commands.try_send(command(ack)).map_err(|_| {
                Error::Invalid("PROFIBUS command queue is full or the worker stopped".into())
            })?;
            self.thread.unpark();
            tokio::time::timeout(Duration::from_secs(5), response)
                .await
                .map_err(|_| Error::Timeout)?
                .map_err(|_| {
                    Error::Invalid("PROFIBUS worker stopped before applying the command".into())
                })?
        }
        pub async fn disconnect(&self) -> Result<()> {
            self.stop.store(true, Ordering::Release);
            self.thread.unpark();
            let mut done = self.done.clone();
            tokio::time::timeout(Duration::from_secs(5), async {
                while !*done.borrow_and_update() {
                    done.changed()
                        .await
                        .map_err(|_| Error::Invalid("PROFIBUS worker status unavailable".into()))?;
                }
                self.state
                    .borrow()
                    .error
                    .clone()
                    .map_or(Ok(()), |error| Err(Error::Invalid(error)))
            })
            .await
            .map_err(|_| Error::Timeout)?
        }
    }
    #[cfg(test)]
    #[tokio::test]
    async fn disconnect_propagates_worker_failure() {
        let (commands, _) = mpsc::channel(1);
        let mut snapshot = empty_snapshot(ProfibusMode::Clear);
        snapshot.stopped = true;
        snapshot.error = Some("serial write failed".into());
        let (_, state) = watch::channel(snapshot);
        let (_, done) = watch::channel(true);
        let master = ProfibusMaster {
            commands,
            state,
            done,
            stop: Arc::new(AtomicBool::new(false)),
            thread: std::thread::current(),
        };
        assert!(
            master
                .disconnect()
                .await
                .unwrap_err()
                .to_string()
                .contains("serial write failed")
        );
    }
    fn empty_snapshot(mode: ProfibusMode) -> ProfibusSnapshot {
        ProfibusSnapshot {
            sequence: 0,
            cycle: 0,
            timestamp_ms: 0,
            mode,
            peripherals: vec![],
            error: None,
            stopped: false,
        }
    }
    fn baudrate(rate: u32) -> Baudrate {
        match rate {
            9600 => Baudrate::B9600,
            19200 => Baudrate::B19200,
            31250 => Baudrate::B31250,
            45450 => Baudrate::B45450,
            93750 => Baudrate::B93750,
            187500 => Baudrate::B187500,
            500000 => Baudrate::B500000,
            1500000 => Baudrate::B1500000,
            3000000 => Baudrate::B3000000,
            6000000 => Baudrate::B6000000,
            _ => Baudrate::B12000000,
        }
    }
    pub(super) fn stack(config: &ProfibusConfig) -> (dp::DpMaster<'_>, fdl::FdlActiveStation) {
        let mut master = dp::DpMaster::new(Vec::new());
        for p in &config.peripherals {
            master.add(
                dp::Peripheral::new(
                    p.address,
                    dp::PeripheralOptions {
                        ident_number: p.ident_number,
                        user_parameters: Some(&p.user_parameters),
                        config: Some(&p.configuration),
                        max_tsdr: p.max_tsdr_bits,
                        fail_safe: p.fail_safe,
                        groups: p.groups,
                        ..Default::default()
                    },
                    vec![0; p.input_bytes],
                    p.initial_outputs.clone(),
                )
                .with_diag_buffer(vec![0; 244]),
            );
        }
        let fdl = fdl::FdlActiveStation::new(
            fdl::ParametersBuilder::new(config.master_address, baudrate(config.baud_rate))
                .highest_station_address(config.highest_station_address)
                .slot_bits(config.slot_bits)
                .max_retry_limit(config.retry_limit)
                .watchdog_timeout(profirust::time::Duration::from_millis(config.watchdog_ms))
                .build_verified(&master),
        );
        apply_mode(&mut master, config.mode);
        (master, fdl)
    }
    fn apply_mode(master: &mut dp::DpMaster<'_>, mode: ProfibusMode) {
        match mode {
            ProfibusMode::Stop => master.enter_stop(),
            ProfibusMode::Clear => master.enter_clear(),
            ProfibusMode::Operate => master.enter_operate(),
        }
    }
    pub(super) fn apply_output(
        master: &mut dp::DpMaster<'_>,
        output: ProfibusOutput,
    ) -> Result<()> {
        let (_, peripheral) = master
            .iter_mut()
            .find(|(_, p)| p.address() == output.address)
            .ok_or_else(|| {
                Error::Invalid("PROFIBUS peripheral address is not configured".into())
            })?;
        require(
            peripheral.pi_q().len() == output.values.len(),
            "PROFIBUS output length differs from the configured process image",
        )?;
        peripheral.pi_q_mut().copy_from_slice(&output.values);
        Ok(())
    }
    fn update(state: &watch::Sender<ProfibusSnapshot>, master: &dp::DpMaster<'_>, completed: bool) {
        state.send_modify(|s| {
            s.sequence += 1;
            s.cycle += u64::from(completed);
            s.timestamp_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .min(u64::MAX as u128) as u64;
            s.mode = match master.operating_state() {
                dp::OperatingState::Stop => ProfibusMode::Stop,
                dp::OperatingState::Clear => ProfibusMode::Clear,
                dp::OperatingState::Operate => ProfibusMode::Operate,
            };
            s.peripherals = master
                .iter()
                .map(|(_, p)| {
                    let diagnostics = p.last_diagnostics();
                    ProfibusPeripheralState {
                        address: p.address(),
                        live: p.is_live(),
                        running: p.is_running(),
                        inputs: p.pi_i().to_vec(),
                        outputs: p.pi_q().to_vec(),
                        diagnostic_flags: diagnostics.as_ref().map(|d| d.flags.bits()),
                        extended_diagnostics: diagnostics
                            .and_then(|d| {
                                d.extended_diagnostics.raw_diag_buffer().map(|v| v.to_vec())
                            })
                            .unwrap_or_default(),
                    }
                })
                .collect();
        });
    }
    pub(super) struct ClearShutdown {
        started: std::time::Instant,
        previous_cycle_completed: bool,
        pending: std::collections::BTreeSet<u8>,
    }
    impl ClearShutdown {
        pub(super) fn new(master: &dp::DpMaster<'_>, started: std::time::Instant) -> Self {
            Self {
                started,
                previous_cycle_completed: false,
                pending: master
                    .iter()
                    .map(|(_, peripheral)| peripheral.address())
                    .collect(),
            }
        }
        pub(super) fn advance(
            &mut self,
            events: &dp::DpEvents,
            now: std::time::Instant,
        ) -> Result<bool> {
            if !self.previous_cycle_completed {
                // A reply in this partial cycle may acknowledge an Operate request already sent.
                self.previous_cycle_completed = events.cycle_completed;
            } else if let Some((handle, dp::PeripheralEvent::DataExchanged)) = events.peripheral {
                self.pending.remove(&handle.address());
            }
            if self.pending.is_empty() {
                return Ok(true);
            }
            if now.duration_since(self.started) >= Duration::from_secs(1) {
                return Err(Error::Invalid(format!(
                    "PROFIBUS Clear was not acknowledged by stations {:?} before shutdown",
                    self.pending,
                )));
            }
            Ok(false)
        }
    }
    fn run(
        config: ProfibusConfig,
        mut commands: mpsc::Receiver<Command>,
        state: &watch::Sender<ProfibusSnapshot>,
        stop: Arc<AtomicBool>,
        ready: oneshot::Sender<Result<()>>,
    ) -> Result<()> {
        let mut phy = match SerialPhy::open(&config) {
            Ok(phy) => phy,
            Err(error) => {
                let message = error.to_string();
                let _ = ready.send(Err(error));
                return Err(Error::Invalid(message));
            }
        };
        let (mut master, mut fdl) = stack(&config);
        fdl.set_online();
        update(state, &master, false);
        let _ = ready.send(Ok(()));
        let mut stopping = None;
        loop {
            if stop.load(Ordering::Acquire) && stopping.is_none() {
                master.enter_clear();
                stopping = Some(ClearShutdown::new(&master, std::time::Instant::now()));
            }
            if stopping.is_none() {
                // Bound control work per poll so writers cannot starve fieldbus traffic.
                for _ in 0..32 {
                    match commands.try_recv() {
                        Ok(Command::Write(output, ack)) => {
                            if ack.is_closed() {
                                continue;
                            }
                            let result = apply_output(&mut master, output);
                            update(state, &master, false);
                            let _ = ack.send(result);
                        }
                        Ok(Command::Mode(mode, ack)) => {
                            if ack.is_closed() {
                                continue;
                            }
                            apply_mode(&mut master, mode);
                            update(state, &master, false);
                            let _ = ack.send(Ok(()));
                        }
                        Err(_) => break,
                    }
                }
            }
            fdl.poll(profirust::time::Instant::now(), &mut phy, &mut master);
            if let Some(error) = phy.error.take() {
                return Err(Error::Invalid(format!(
                    "PROFIBUS serial I/O failed: {error}"
                )));
            }
            let events = master.take_last_events();
            if events.cycle_completed || events.peripheral.is_some() {
                update(state, &master, events.cycle_completed);
            }
            if let Some(shutdown) = &mut stopping {
                match shutdown.advance(&events, std::time::Instant::now()) {
                    Ok(true) => break,
                    Ok(false) => {}
                    Err(error) => {
                        fdl.set_offline();
                        return Err(error);
                    }
                }
            }
            std::thread::park_timeout(Duration::from_micros(config.poll_interval_us));
        }
        fdl.set_offline();
        Ok(())
    }

    // The upstream serial PHY panics on OS I/O errors. Record them here and let the worker close.
    struct SerialPhy {
        port: Box<dyn tokio_serial::SerialPort>,
        rx: [u8; 512],
        rx_len: usize,
        tx: [u8; 512],
        tx_len: usize,
        tx_cursor: usize,
        error: Option<String>,
    }
    impl SerialPhy {
        fn open(config: &ProfibusConfig) -> Result<Self> {
            let port = tokio_serial::new(&config.port, config.baud_rate)
                .data_bits(tokio_serial::DataBits::Eight)
                .parity(tokio_serial::Parity::Even)
                .stop_bits(tokio_serial::StopBits::One)
                .flow_control(tokio_serial::FlowControl::None)
                .timeout(Duration::from_millis(5))
                .open()
                .map_err(|e| Error::Invalid(format!("Cannot open PROFIBUS serial device: {e}")))?;
            port.clear(tokio_serial::ClearBuffer::All)
                .map_err(|e| Error::Invalid(format!("Cannot clear PROFIBUS serial device: {e}")))?;
            Ok(Self {
                port,
                rx: [0; 512],
                rx_len: 0,
                tx: [0; 512],
                tx_len: 0,
                tx_cursor: 0,
                error: None,
            })
        }
        fn fail(&mut self, error: impl std::fmt::Display) {
            if self.error.is_none() {
                self.error = Some(error.to_string());
            }
        }
    }
    impl ProfibusPhy for SerialPhy {
        fn poll_transmission(&mut self, _: profirust::time::Instant) -> bool {
            if self.error.is_some() {
                return false;
            }
            if self.tx_cursor < self.tx_len {
                match self.port.write(&self.tx[self.tx_cursor..self.tx_len]) {
                    Ok(n) => self.tx_cursor += n,
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) => {}
                    Err(e) => self.fail(e),
                }
            }
            if self.tx_cursor < self.tx_len {
                return true;
            }
            match self.port.bytes_to_write() {
                Ok(0) => {
                    self.tx_len = 0;
                    self.tx_cursor = 0;
                    false
                }
                Ok(_) => true,
                Err(e) => {
                    self.fail(e);
                    false
                }
            }
        }
        fn transmit_data<F, R>(&mut self, _: profirust::time::Instant, f: F) -> R
        where
            F: FnOnce(&mut [u8]) -> (usize, R),
        {
            self.tx.fill(0);
            let (length, result) = f(&mut self.tx);
            if length > self.tx.len() {
                self.fail("Transmit frame exceeds buffer");
            } else {
                self.tx_len = length;
                self.tx_cursor = 0;
            }
            result
        }
        fn receive_data<F, R>(&mut self, _: profirust::time::Instant, f: F) -> R
        where
            F: FnOnce(&[u8]) -> (usize, R),
        {
            if self.error.is_none() {
                match self.port.bytes_to_read() {
                    Ok(pending) if pending > 0 => {
                        let count = (pending as usize).min(self.rx.len() - self.rx_len);
                        if count == 0 {
                            self.fail("Receive buffer overflow");
                        } else {
                            match self
                                .port
                                .read(&mut self.rx[self.rx_len..self.rx_len + count])
                            {
                                Ok(n) => self.rx_len += n,
                                Err(e)
                                    if matches!(
                                        e.kind(),
                                        std::io::ErrorKind::WouldBlock
                                            | std::io::ErrorKind::TimedOut
                                    ) => {}
                                Err(e) => self.fail(e),
                            }
                        }
                    }
                    Err(e) => self.fail(e),
                    _ => {}
                }
            }
            let (consumed, result) = f(&self.rx[..self.rx_len]);
            if consumed > self.rx_len {
                self.fail("Parser consumed beyond receive buffer");
                self.rx_len = 0;
            } else {
                self.rx.copy_within(consumed..self.rx_len, 0);
                self.rx_len -= consumed;
            }
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> ProfibusConfig {
        ProfibusConfig {
            port: "/dev/flow-profibus-test".into(),
            baud_rate: 19200,
            master_address: 2,
            highest_station_address: 3,
            slot_bits: 576,
            poll_interval_us: 10000,
            watchdog_ms: 2000,
            retry_limit: 2,
            mode: ProfibusMode::Operate,
            peripherals: vec![ProfibusPeripheralConfig {
                address: 7,
                ident_number: 0x1234,
                user_parameters: vec![0x55],
                configuration: vec![0x11, 0x20],
                input_bytes: 2,
                initial_outputs: vec![0x80],
                max_tsdr_bits: 100,
                fail_safe: false,
                groups: 0,
            }],
        }
    }
    #[test]
    fn rejects_duplicate_addresses_and_timing_or_payload_overflow() {
        let mut value = config();
        value.validate().unwrap();
        value.peripherals.push(value.peripherals[0].clone());
        assert!(value.validate().is_err());
        let mut value = config();
        value.peripherals[0].max_tsdr_bits = u16::MAX;
        assert!(value.validate().is_err());
        let mut value = config();
        value.peripherals[0].configuration.resize(245, 0);
        assert!(value.validate().is_err());
        let mut value = config();
        value.poll_interval_us = 100000;
        assert!(value.validate().is_err());
    }
    #[cfg(feature = "execute")]
    #[test]
    fn dp_master_parameterizes_then_exchanges_native_telegrams() {
        exercise_modes(false);
        exercise_modes(true);
    }
    #[cfg(feature = "execute")]
    fn exercise_modes(fail_safe: bool) {
        use profirust::fdl::{
            DataTelegramHeader, FdlApplication, FunctionCode, ResponseState, ResponseStatus,
            ShortConfirmation, Telegram, TelegramTx,
        };
        let mut config = config();
        config.peripherals[0].fail_safe = fail_safe;
        let (mut master, fdl) = execution::stack(&config);
        let mut configured = false;
        let mut parameterized = false;
        let mut exchanged = false;
        for tick in 0..32 {
            let now = profirust::time::Instant::from_millis(tick);
            let mut bytes = [0u8; 512];
            let Some(sent) =
                master.transmit_telegram(now, &fdl, TelegramTx::new(&mut bytes), false)
            else {
                continue;
            };
            let (telegram, decoded) = Telegram::deserialize(&bytes[..sent.bytes_sent()])
                .unwrap()
                .unwrap();
            assert_eq!(decoded, sent.bytes_sent());
            let Telegram::Data(request) = telegram else {
                panic!("Expected a native DP telegram");
            };
            if request.h.da == 127 {
                continue;
            }
            assert_eq!(request.h.da, 7);
            assert_eq!(request.h.sa, 2);
            let mut response = [0u8; 512];
            let count = match request.h.dsap {
                Some(60) => {
                    let flags = if configured { 0 } else { 2 };
                    let payload = [flags, 4, 0, 2, 0x12, 0x34];
                    DataTelegramHeader {
                        da: 2,
                        sa: 7,
                        dsap: request.h.ssap,
                        ssap: request.h.dsap,
                        fc: FunctionCode::Response {
                            state: ResponseState::Slave,
                            status: ResponseStatus::DataLow,
                        },
                    }
                    .serialize(&mut response, payload.len(), |data| {
                        data.copy_from_slice(&payload)
                    })
                }
                Some(61) => {
                    assert_eq!(&request.pdu[4..6], &[0x12, 0x34]);
                    assert_eq!(&request.pdu[7..], &[0x55]);
                    assert_ne!(request.pdu[0] & 8, 0, "Device watchdog must be enabled");
                    parameterized = true;
                    ShortConfirmation.serialize(&mut response)
                }
                Some(62) => {
                    assert!(parameterized);
                    assert_eq!(request.pdu, &[0x11, 0x20]);
                    configured = true;
                    ShortConfirmation.serialize(&mut response)
                }
                None => {
                    assert!(configured);
                    assert_eq!(request.pdu, &[0x80]);
                    exchanged = true;
                    DataTelegramHeader {
                        da: 2,
                        sa: 7,
                        dsap: None,
                        ssap: None,
                        fc: FunctionCode::Response {
                            state: ResponseState::Slave,
                            status: ResponseStatus::DataLow,
                        },
                    }
                    .serialize(&mut response, 2, |data| data.copy_from_slice(&[0x12, 0x34]))
                }
                other => panic!("Unexpected DP SAP {other:?}"),
            };
            let (reply, _) = Telegram::deserialize(&response[..count]).unwrap().unwrap();
            master.receive_reply(now, &fdl, 7, reply);
            if exchanged {
                break;
            }
        }
        assert!(exchanged);
        let (_, peripheral) = master.iter().next().unwrap();
        assert!(peripheral.is_running());
        assert_eq!(peripheral.pi_i(), &[0x12, 0x34]);
        execution::apply_output(
            &mut master,
            ProfibusOutput {
                address: 7,
                values: vec![0x40],
            },
        )
        .unwrap();
        assert_eq!(master.iter().next().unwrap().1.pi_q(), &[0x40]);
        assert!(
            execution::apply_output(
                &mut master,
                ProfibusOutput {
                    address: 7,
                    values: vec![0; 2]
                }
            )
            .is_err()
        );
        assert_eq!(master.iter().next().unwrap().1.pi_q(), &[0x40]);

        for (phase, clear) in [true, false].into_iter().enumerate() {
            if clear {
                master.enter_clear();
            } else {
                master.enter_operate();
            }
            let mut global_control = false;
            let mut data_exchange = false;
            for tick in 0..32 {
                let now = profirust::time::Instant::from_millis(tick + 100 * (phase as i64 + 1));
                // A reused transmit buffer must not leak the previous output image in Clear.
                let mut bytes = [0xa5u8; 512];
                let Some(sent) =
                    master.transmit_telegram(now, &fdl, TelegramTx::new(&mut bytes), false)
                else {
                    continue;
                };
                let (Telegram::Data(request), _) =
                    Telegram::deserialize(&bytes[..sent.bytes_sent()])
                        .unwrap()
                        .unwrap()
                else {
                    panic!("Expected DP data");
                };
                if request.h.da == 127 {
                    assert_eq!(request.pdu, &[if clear { 2 } else { 0 }, 0]);
                    global_control = true;
                    continue;
                }
                assert!(global_control);
                assert_eq!(request.h.dsap, None);
                let expected: &[u8] = if !clear {
                    &[0x40]
                } else if fail_safe {
                    &[]
                } else {
                    &[0]
                };
                assert_eq!(request.pdu, expected);
                assert_eq!(master.iter().next().unwrap().1.pi_q(), &[0x40]);
                let mut response = [0u8; 512];
                let count = DataTelegramHeader {
                    da: 2,
                    sa: 7,
                    dsap: None,
                    ssap: None,
                    fc: FunctionCode::Response {
                        state: ResponseState::Slave,
                        status: ResponseStatus::DataLow,
                    },
                }
                .serialize(&mut response, 2, |data| data.copy_from_slice(&[0x12, 0x34]));
                let (reply, _) = Telegram::deserialize(&response[..count]).unwrap().unwrap();
                // STOP must also accept a reply already in flight without corrupting the cycle.
                master.enter_stop();
                master.receive_reply(now, &fdl, 7, reply);
                assert!(
                    master
                        .transmit_telegram(now, &fdl, TelegramTx::new(&mut bytes), false,)
                        .is_none()
                );
                data_exchange = true;
                break;
            }
            assert!(data_exchange);
        }
    }
    #[cfg(feature = "execute")]
    #[test]
    fn shutdown_does_not_accept_completed_sweeps_of_offline_peripherals() {
        use profirust::fdl::{FdlApplication, TelegramTx};
        let config = config();
        let (mut master, fdl) = execution::stack(&config);
        master.enter_clear();
        let started = std::time::Instant::now();
        let mut shutdown = execution::ClearShutdown::new(&master, started);
        let mut completed = 0;
        for tick in 0..32 {
            let mut bytes = [0; 512];
            let _ = master.transmit_telegram(
                profirust::time::Instant::from_millis(tick),
                &fdl,
                TelegramTx::new(&mut bytes),
                false,
            );
            // No station answers these telegrams. The SDK still completes polling sweeps.
            let events = master.take_last_events();
            completed += usize::from(events.cycle_completed);
            assert!(!shutdown.advance(&events, started).unwrap());
        }
        assert!(completed >= 2);
        let error = shutdown
            .advance(
                &profirust::dp::DpEvents::default(),
                started + std::time::Duration::from_secs(1),
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("stations {7}"));
    }
    #[cfg(feature = "execute")]
    #[test]
    fn shutdown_requires_fresh_data_acknowledgements_from_every_station() {
        use profirust::dp::{DpEvents, PeripheralEvent};
        let mut config = config();
        let mut second = config.peripherals[0].clone();
        second.address = 8;
        config.peripherals.push(second);
        let (master, _) = execution::stack(&config);
        let handles: Vec<_> = master.iter().map(|(handle, _)| handle).collect();
        let started = std::time::Instant::now();
        let mut shutdown = execution::ClearShutdown::new(&master, started);
        let event = |index, kind, cycle_completed| DpEvents {
            cycle_completed,
            peripheral: Some((handles[index], kind)),
        };
        // The last acknowledgement in the partial cycle may still refer to old Operate data.
        assert!(
            !shutdown
                .advance(&event(0, PeripheralEvent::DataExchanged, true), started)
                .unwrap()
        );
        for kind in [
            PeripheralEvent::Diagnostics,
            PeripheralEvent::Configured,
            PeripheralEvent::Offline,
        ] {
            assert!(!shutdown.advance(&event(1, kind, true), started).unwrap());
        }
        assert!(
            !shutdown
                .advance(&event(1, PeripheralEvent::DataExchanged, true), started)
                .unwrap()
        );
        // Repeated successful exchanges from station 8 do not clear the missing station 7.
        assert!(
            !shutdown
                .advance(&event(1, PeripheralEvent::DataExchanged, true), started)
                .unwrap()
        );
        assert!(
            shutdown
                .advance(&event(0, PeripheralEvent::DataExchanged, true), started)
                .unwrap()
        );
    }
    #[cfg(feature = "execute")]
    #[test]
    fn master_can_start_in_clear_or_stop() {
        for mode in [ProfibusMode::Clear, ProfibusMode::Stop] {
            let mut config = config();
            config.mode = mode;
            let (master, _) = execution::stack(&config);
            assert_eq!(
                master.operating_state().is_clear(),
                matches!(mode, ProfibusMode::Clear)
            );
            assert_eq!(
                master.operating_state().is_stop(),
                matches!(mode, ProfibusMode::Stop)
            );
        }
    }
    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn nonexistent_serial_device_returns_error_without_panicking() {
        assert!(ProfibusMaster::connect(config()).await.is_err());
    }
}
