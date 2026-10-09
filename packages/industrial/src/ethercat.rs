//! Native EtherCAT cyclic process data on a dedicated network interface.

use crate::{Result, require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const MAX_DEVICES: usize = 64;
pub const MAX_PROCESS_DATA: usize = 16_384;
// A 1500-byte Ethernet payload includes the EtherCAT header, LRW header and working counter.
const LRW_DATA_BYTES: usize = 1500 - 2 - 10 - 2;
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum SdoValue {
    U8(u8),
    U16(u16),
    U32(u32),
    I8(i8),
    I16(i16),
    I32(i32),
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StartupSdo {
    pub device: usize,
    pub index: u16,
    pub sub_index: u8,
    pub value: SdoValue,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExpectedDevice {
    pub vendor_id: u32,
    pub product_id: u32,
    pub revision: Option<u32>,
    pub serial: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct EthercatConfig {
    pub interface: String,
    /// The worker uses a dedicated thread. OS scheduling still determines actual timing.
    pub cycle_us: u64,
    pub startup_timeout_ms: u64,
    pub response_timeout_ms: u64,
    /// When present, require the bus to match this ordered device list before entering OP.
    pub expected_devices: Vec<ExpectedDevice>,
    /// Expedited CoE writes applied in PRE-OP before the PDO mapping is read.
    pub startup_sdos: Vec<StartupSdo>,
}
impl Default for EthercatConfig {
    fn default() -> Self {
        Self {
            interface: String::new(),
            cycle_us: 10_000,
            startup_timeout_ms: 30_000,
            response_timeout_ms: 100,
            expected_devices: vec![],
            startup_sdos: vec![],
        }
    }
}
impl EthercatConfig {
    pub fn validate(&self) -> Result<()> {
        require(
            !self.interface.trim().is_empty() && !self.interface.contains('\0'),
            "Choose a dedicated EtherCAT network interface",
        )?;
        require(
            (1000..=1_000_000).contains(&self.cycle_us),
            "EtherCAT cycle must be between 1000 and 1000000 microseconds",
        )?;
        require(
            (100..=300_000).contains(&self.startup_timeout_ms),
            "EtherCAT startup timeout must be between 100 and 300000 ms",
        )?;
        require(
            (1..=10_000).contains(&self.response_timeout_ms),
            "EtherCAT response timeout must be between 1 and 10000 ms",
        )?;
        require(
            self.expected_devices.len() <= MAX_DEVICES,
            "EtherCAT supports up to 64 devices per session",
        )?;
        require(
            self.startup_sdos.len() <= 4096,
            "EtherCAT startup configuration exceeds 4096 SDO writes",
        )?;
        require(
            self.startup_sdos.iter().all(|sdo| sdo.device < MAX_DEVICES),
            "EtherCAT SDO device position must be below 64",
        )
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EthercatDevice {
    pub position: usize,
    pub station_address: u16,
    pub name: String,
    pub vendor_id: u32,
    pub product_id: u32,
    pub revision: u32,
    pub serial: u32,
    pub input_bytes: usize,
    pub output_bytes: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DeviceProcessData {
    pub position: usize,
    pub inputs: Vec<u8>,
    pub outputs: Vec<u8>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct EthercatSnapshot {
    pub cycle: u64,
    pub working_counter: u16,
    pub expected_working_counter: u16,
    pub operational: bool,
    pub stopped: bool,
    pub error: Option<String>,
    pub cycle_elapsed_us: u64,
    pub overruns: u64,
    pub devices: Vec<DeviceProcessData>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EthercatOutput {
    pub device: usize,
    pub offset: usize,
    pub data: Vec<u8>,
}
fn apply_output(output: &mut [u8], offset: usize, data: &[u8]) -> Result<()> {
    require(!data.is_empty(), "EtherCAT output update must not be empty")?;
    let end = offset
        .checked_add(data.len())
        .ok_or_else(|| crate::Error::Invalid("EtherCAT output range overflow".into()))?;
    require(
        end <= output.len(),
        "EtherCAT output range exceeds the device's mapped PDO bytes",
    )?;
    output[offset..end].copy_from_slice(data);
    Ok(())
}
fn expected_working_counter(devices: &[EthercatDevice]) -> u16 {
    // EtherCrab maps all input ranges first, then all output ranges, in device order. tx_rx
    // starts each frame with one LRW filling its available data capacity. Count a direction
    // again whenever its mapped range crosses into another LRW datagram.
    let mut offset = 0;
    let mut counter = 0;
    for (length, weight) in devices
        .iter()
        .map(|device| (device.input_bytes, 1))
        .chain(devices.iter().map(|device| (device.output_bytes, 2)))
    {
        if length > 0 {
            let datagrams = (offset + length - 1) / LRW_DATA_BYTES - offset / LRW_DATA_BYTES + 1;
            counter += datagrams as u16 * weight;
            offset += length;
        }
    }
    counter
}
fn validate_inventory(expected: &[ExpectedDevice], actual: &[EthercatDevice]) -> Result<()> {
    if expected.is_empty() {
        return Ok(());
    }
    require(
        expected.len() == actual.len(),
        "EtherCAT device count does not match the configured topology",
    )?;
    for (expected, actual) in expected.iter().zip(actual) {
        require(
            expected.vendor_id == actual.vendor_id
                && expected.product_id == actual.product_id
                && expected.revision.is_none_or(|v| v == actual.revision)
                && expected.serial.is_none_or(|v| v == actual.serial),
            "EtherCAT device identity does not match the configured topology",
        )?;
    }
    Ok(())
}

#[cfg(feature = "execute")]
mod runtime {
    use super::*;
    use crate::Error;
    use std::time::Duration;
    use tokio::sync::{mpsc, oneshot, watch};
    use tokio_util::sync::CancellationToken;

    struct OutputCommand {
        update: EthercatOutput,
        result: oneshot::Sender<Result<()>>,
    }
    pub struct EthercatMaster {
        pub devices: Vec<EthercatDevice>,
        commands: mpsc::Sender<OutputCommand>,
        state: watch::Receiver<EthercatSnapshot>,
        stop: CancellationToken,
    }
    fn failure(error: impl std::fmt::Display) -> Error {
        Error::Other(anyhow::anyhow!(error.to_string()))
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    struct InterfaceLease(String);
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn interfaces() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
        static INTERFACES: std::sync::OnceLock<
            std::sync::Mutex<std::collections::HashSet<String>>,
        > = std::sync::OnceLock::new();
        INTERFACES.get_or_init(Default::default)
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    impl InterfaceLease {
        fn acquire(interface: &str) -> Result<Self> {
            require(
                interfaces()
                    .lock()
                    .map_err(failure)?
                    .insert(interface.into()),
                "An EtherCAT master already owns this interface",
            )?;
            Ok(Self(interface.into()))
        }
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    impl Drop for InterfaceLease {
        fn drop(&mut self) {
            if let Ok(mut interfaces) = interfaces().lock() {
                interfaces.remove(&self.0);
            }
        }
    }
    impl EthercatMaster {
        pub async fn start(config: EthercatConfig) -> Result<Self> {
            config.validate()?;
            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            {
                let _ = config;
                return Err(failure(
                    "Native EtherCAT currently requires a Linux or macOS executor and raw access to a dedicated network interface",
                ));
            }
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            {
                let lease = InterfaceLease::acquire(&config.interface)?;
                let (commands, receiver) = mpsc::channel(64);
                let (state_tx, state) = watch::channel(EthercatSnapshot::default());
                let (ready_tx, ready) = oneshot::channel();
                let stop = CancellationToken::new();
                let worker_stop = stop.clone();
                let timeout = Duration::from_millis(config.startup_timeout_ms);
                std::thread::Builder::new()
                    .name("flow-like-ethercat".into())
                    .stack_size(8 * 1024 * 1024)
                    .spawn(move || {
                        let _lease = lease;
                        let mut ready_tx = Some(ready_tx);
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                            || -> Result<()> {
                                let runtime = tokio::runtime::Builder::new_current_thread()
                                    .enable_all()
                                    .build()?;
                                runtime.block_on(run_bus(
                                    config,
                                    receiver,
                                    &state_tx,
                                    &mut ready_tx,
                                    worker_stop,
                                ))
                            },
                        ))
                        .unwrap_or_else(|_| Err(failure("EtherCAT worker panicked and stopped")));
                        let error = result.err().map(|error| error.to_string());
                        if let Some(ready_tx) = ready_tx.take() {
                            let _ = ready_tx.send(Err(failure(
                                error.as_deref().unwrap_or("EtherCAT startup stopped"),
                            )));
                        }
                        state_tx.send_modify(|state| {
                            state.operational = false;
                            state.stopped = true;
                            state.error = error;
                        });
                    })?;
                let mut master = Self {
                    devices: vec![],
                    commands,
                    state,
                    stop,
                };
                master.devices = tokio::time::timeout(timeout, ready)
                    .await
                    .map_err(|_| Error::Timeout)?
                    .map_err(failure)??;
                Ok(master)
            }
        }
        pub fn snapshot(&self) -> Result<EthercatSnapshot> {
            let state = self.state.borrow().clone();
            if let Some(error) = &state.error {
                return Err(failure(error));
            }
            Ok(state)
        }
        pub fn subscribe(&self) -> watch::Receiver<EthercatSnapshot> {
            self.state.clone()
        }
        pub async fn write(&self, update: EthercatOutput) -> Result<()> {
            require(
                update.device < self.devices.len(),
                "EtherCAT device position is not present",
            )?;
            let size = self.devices[update.device].output_bytes;
            let end = update
                .offset
                .checked_add(update.data.len())
                .ok_or_else(|| failure("EtherCAT output range overflow"))?;
            require(
                !update.data.is_empty() && end <= size,
                "EtherCAT output update exceeds the mapped device output bytes",
            )?;
            let (result, response) = oneshot::channel();
            tokio::select! {
                _ = self.stop.cancelled() => return Err(failure("EtherCAT master is stopping")),
                result = self.commands.send(OutputCommand { update, result }) => result.map_err(failure)?,
            }
            response
                .await
                .map_err(|_| failure("EtherCAT master stopped before applying the output update"))?
        }
        pub async fn close(&self) -> Result<()> {
            self.stop.cancel();
            let mut state = self.state.clone();
            tokio::time::timeout(Duration::from_secs(15), async {
                loop {
                    let snapshot = state.borrow().clone();
                    if snapshot.stopped {
                        return snapshot.error.map_or(Ok(()), |error| Err(failure(error)));
                    }
                    state.changed().await.map_err(failure)?;
                }
            })
            .await
            .map_err(|_| Error::Timeout)?
        }
    }
    impl Drop for EthercatMaster {
        fn drop(&mut self) {
            self.stop.cancel();
        }
    }

    async fn startup_step<T, E: std::fmt::Display>(
        stop: &CancellationToken,
        operation: impl std::future::Future<Output = std::result::Result<T, E>>,
    ) -> Result<T> {
        // Prefer cancellation even when the next phase is immediately ready. Dropping an
        // EtherCrab request also releases its pending PDU, so it cannot be retried afterward.
        tokio::select! {
            biased;
            _ = stop.cancelled() => Err(failure("EtherCAT startup cancelled")),
            result = operation => result.map_err(failure),
        }
    }
    async fn with_bus_shutdown<T>(
        operation: impl std::future::Future<Output = Result<T>>,
        shutdown: impl std::future::Future<Output = Result<()>>,
    ) -> Result<T> {
        let result = operation.await;
        match (result, shutdown.await) {
            (result, Ok(())) => result,
            (Ok(_), Err(error)) => Err(error),
            (Err(error), Err(shutdown)) => Err(failure(format!(
                "{error}; EtherCAT shutdown failed: {shutdown}"
            ))),
        }
    }
    fn protocol_timeouts(response: Duration) -> ethercrab::Timeouts {
        let defaults = ethercrab::Timeouts::default();
        // These operations await PDUs internally. A shorter outer deadline drops a
        // permitted response and leaves its late frame referring to a released PDU.
        ethercrab::Timeouts {
            pdu: response,
            eeprom: defaults.eeprom.max(response),
            mailbox_echo: defaults.mailbox_echo.max(response),
            mailbox_response: defaults.mailbox_response.max(response),
            state_transition: defaults.state_transition.max(response),
            wait_loop_delay: Duration::from_millis(1),
        }
    }
    #[cfg(test)]
    mod cancellation_tests {
        use super::*;
        use std::cell::RefCell;

        #[test]
        fn operation_deadlines_allow_configured_responses_and_preserve_longer_defaults() {
            let short = protocol_timeouts(Duration::from_millis(1));
            assert_eq!(short.pdu, Duration::from_millis(1));
            assert_eq!(short.eeprom, Duration::from_millis(10));
            assert_eq!(short.mailbox_echo, Duration::from_millis(100));
            assert_eq!(short.mailbox_response, Duration::from_secs(1));
            assert_eq!(short.state_transition, Duration::from_secs(5));

            let delayed = protocol_timeouts(Duration::from_millis(250));
            assert_eq!(delayed.eeprom, Duration::from_millis(250));
            assert_eq!(delayed.mailbox_echo, Duration::from_millis(250));
            assert_eq!(delayed.mailbox_response, Duration::from_secs(1));

            let long = protocol_timeouts(Duration::from_secs(10));
            for deadline in [
                long.eeprom,
                long.mailbox_echo,
                long.mailbox_response,
                long.state_transition,
            ] {
                assert_eq!(deadline, Duration::from_secs(10));
            }
        }

        #[tokio::test]
        async fn cancellation_drops_the_active_phase_skips_later_writes_and_runs_shutdown() {
            // Two SDO writes, mapping, and the OP request share the same cancellation boundary.
            for cancel_at in 0..4 {
                let stop = CancellationToken::new();
                let issued = RefCell::new(Vec::new());
                let result = with_bus_shutdown(
                    async {
                        for phase in 0..4 {
                            startup_step(&stop, async {
                                issued.borrow_mut().push(phase);
                                if phase == cancel_at {
                                    stop.cancel();
                                    std::future::pending::<()>().await;
                                }
                                Ok::<_, Error>(())
                            })
                            .await?;
                        }
                        Ok(())
                    },
                    async {
                        issued.borrow_mut().push(4);
                        Ok(())
                    },
                )
                .await;
                assert!(result.unwrap_err().to_string().contains("cancelled"));
                assert_eq!(
                    *issued.borrow(),
                    (0..=cancel_at).chain([4]).collect::<Vec<_>>()
                );
            }
        }

        #[tokio::test]
        async fn an_already_cancelled_phase_never_starts() {
            let stop = CancellationToken::new();
            stop.cancel();
            assert!(
                startup_step(&stop, async {
                    panic!("Cancelled startup issued another device operation");
                    #[allow(unreachable_code)]
                    Ok::<_, Error>(())
                })
                .await
                .is_err()
            );
        }

        #[tokio::test]
        async fn startup_and_shutdown_errors_are_both_reported() {
            let error =
                with_bus_shutdown(async { Err::<(), _>(failure("mapping failed")) }, async {
                    Err(failure("INIT failed"))
                })
                .await
                .unwrap_err()
                .to_string();
            assert!(error.contains("mapping failed"));
            assert!(error.contains("INIT failed"));
        }
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    async fn return_to_init(device: &ethercrab::MainDevice<'_>) -> Result<()> {
        // State-transition methods consume the group. A cancelled mapping or OP transition
        // may have dropped it, so reset all devices through the dedicated bus instead.
        ethercrab::Command::bwr(ethercrab::RegisterAddress::AlControl.into())
            .ignore_wkc()
            // INIT plus error acknowledgement, as used by EtherCrab's discovery reset.
            .send(device, 0x0011u16)
            .await
            .map_err(failure)?;
        if device.num_subdevices() > 0 {
            device
                .wait_for_state(ethercrab::SubDeviceState::Init)
                .await
                .map_err(failure)?;
        }
        Ok(())
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    async fn run_bus(
        config: EthercatConfig,
        mut commands: mpsc::Receiver<OutputCommand>,
        state: &watch::Sender<EthercatSnapshot>,
        ready: &mut Option<oneshot::Sender<Result<Vec<EthercatDevice>>>>,
        stop: CancellationToken,
    ) -> Result<()> {
        use ethercrab::{
            MainDevice, MainDeviceConfig, PduStorage,
            std::{ethercat_now, tx_rx_task},
        };
        let storage = PduStorage::<16, { PduStorage::element_size(LRW_DATA_BYTES) }>::new();
        let (tx, rx, pdu) = storage
            .try_split()
            .map_err(|_| failure("EtherCAT PDU storage is already in use"))?;
        let device = MainDevice::new(
            pdu,
            protocol_timeouts(Duration::from_millis(config.response_timeout_ms)),
            MainDeviceConfig::default(),
        );
        let io = tx_rx_task(&config.interface, tx, rx)?;
        let control = with_bus_shutdown(
            async {
                let group = startup_step(
                    &stop,
                    device.init_single_group::<MAX_DEVICES, MAX_PROCESS_DATA>(ethercat_now),
                )
                .await?;
                require(
                    !group.is_empty(),
                    "No EtherCAT devices found on this interface",
                )?;
                let mut inventory: Vec<_> = group
                    .iter(&device)
                    .enumerate()
                    .map(|(position, subdevice)| {
                        let identity = subdevice.identity();
                        EthercatDevice {
                            position,
                            station_address: subdevice.configured_address(),
                            name: subdevice.name().to_string(),
                            vendor_id: identity.vendor_id,
                            product_id: identity.product_id,
                            revision: identity.revision,
                            serial: identity.serial,
                            input_bytes: 0,
                            output_bytes: 0,
                        }
                    })
                    .collect();
                validate_inventory(&config.expected_devices, &inventory)?;
                for startup in &config.startup_sdos {
                    let subdevice = group.subdevice(&device, startup.device).map_err(failure)?;
                    startup_step(&stop, async {
                        match startup.value {
                            SdoValue::U8(v) => {
                                subdevice
                                    .sdo_write(startup.index, startup.sub_index, v)
                                    .await
                            }
                            SdoValue::U16(v) => {
                                subdevice
                                    .sdo_write(startup.index, startup.sub_index, v)
                                    .await
                            }
                            SdoValue::U32(v) => {
                                subdevice
                                    .sdo_write(startup.index, startup.sub_index, v)
                                    .await
                            }
                            SdoValue::I8(v) => {
                                subdevice
                                    .sdo_write(startup.index, startup.sub_index, v)
                                    .await
                            }
                            SdoValue::I16(v) => {
                                subdevice
                                    .sdo_write(startup.index, startup.sub_index, v)
                                    .await
                            }
                            SdoValue::I32(v) => {
                                subdevice
                                    .sdo_write(startup.index, startup.sub_index, v)
                                    .await
                            }
                        }
                    })
                    .await?;
                }
                let group = startup_step(&stop, group.into_pre_op_pdi(&device)).await?;
                for (entry, subdevice) in inventory.iter_mut().zip(group.iter(&device)) {
                    let io = subdevice.io_raw();
                    entry.input_bytes = io.inputs().len();
                    entry.output_bytes = io.outputs().len();
                }
                let expected_wkc = expected_working_counter(&inventory);
                let group = startup_step(&stop, group.request_into_op(&device)).await?;
                let period = Duration::from_micros(config.cycle_us);
                let mut interval = tokio::time::interval(period);
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                let op_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
                let mut active = false;
                let mut snapshot = EthercatSnapshot {
                    expected_working_counter: expected_wkc,
                    ..Default::default()
                };
                loop {
                    tokio::select! {
                        biased;
                        _ = stop.cancelled() => break,
                        _ = interval.tick() => {}
                    }
                    let began = std::time::Instant::now();
                    let mut acknowledgements = Vec::new();
                    for _ in 0..64 {
                        let Ok(command) = commands.try_recv() else {
                            break;
                        };
                        if command.result.is_closed() || stop.is_cancelled() {
                            continue;
                        }
                        let result = (|| {
                            let subdevice = group
                                .subdevice(&device, command.update.device)
                                .map_err(failure)?;
                            apply_output(
                                &mut subdevice.outputs_raw_mut(),
                                command.update.offset,
                                &command.update.data,
                            )
                        })();
                        match result {
                            Ok(()) => acknowledgements.push(command.result),
                            Err(error) => {
                                let _ = command.result.send(Err(error));
                            }
                        }
                    }
                    let response = tokio::select! {
                        biased;
                        _ = stop.cancelled() => break,
                        response = group.tx_rx(&device) => response.map_err(failure)?,
                    };
                    let operational = response.all_op() && response.working_counter == expected_wkc;
                    if !operational && (active || tokio::time::Instant::now() >= op_deadline) {
                        return Err(failure(format!(
                            "EtherCAT process data invalid: working counter {}/{}, all devices OP: {}",
                            response.working_counter,
                            expected_wkc,
                            response.all_op()
                        )));
                    }
                    let newly_active = operational && !active;
                    if newly_active {
                        active = true;
                    }
                    for result in acknowledgements {
                        let _ = result.send(Ok(()));
                    }
                    snapshot.cycle += 1;
                    snapshot.working_counter = response.working_counter;
                    snapshot.operational = operational;
                    snapshot.cycle_elapsed_us =
                        began.elapsed().as_micros().min(u64::MAX as u128) as u64;
                    if began.elapsed() > period {
                        snapshot.overruns += 1;
                    }
                    snapshot.devices = group
                        .iter(&device)
                        .enumerate()
                        .map(|(position, subdevice)| {
                            let io = subdevice.io_raw();
                            DeviceProcessData {
                                position,
                                inputs: io.inputs().to_vec(),
                                outputs: io.outputs().to_vec(),
                            }
                        })
                        .collect();
                    state.send_replace(snapshot.clone());
                    if newly_active && let Some(ready) = ready.take() {
                        let _ = ready.send(Ok(inventory.clone()));
                    }
                }
                Ok(())
            },
            return_to_init(&device),
        );
        tokio::pin!(io);
        tokio::pin!(control);
        tokio::select! {
            result = &mut control => result,
            result = &mut io => { result.map_err(failure)?; Err(failure("EtherCAT network task stopped")) },
        }
    }
}
#[cfg(feature = "execute")]
pub use runtime::*;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_updates_are_bounded_and_preserve_other_channels() {
        let mut bytes = [1, 2, 3, 4];
        apply_output(&mut bytes, 1, &[7, 8]).unwrap();
        assert_eq!(bytes, [1, 7, 8, 4]);
        assert!(apply_output(&mut bytes, usize::MAX, &[9]).is_err());
        assert!(apply_output(&mut bytes, 4, &[9]).is_err());
        assert_eq!(bytes, [1, 7, 8, 4]);
    }
    fn mapped_device(input_bytes: usize, output_bytes: usize) -> EthercatDevice {
        EthercatDevice {
            position: 0,
            station_address: 0x1000,
            name: String::new(),
            vendor_id: 0,
            product_id: 0,
            revision: 0,
            serial: 0,
            input_bytes,
            output_bytes,
        }
    }
    #[test]
    fn working_counters_include_each_datagram_overlapping_a_device_direction() {
        assert_eq!(expected_working_counter(&[mapped_device(1, 1)]), 3);
        assert_eq!(expected_working_counter(&[mapped_device(0, 0)]), 0);
        // A direction ending exactly at the frame boundary does not count in the next frame.
        assert_eq!(
            expected_working_counter(&[mapped_device(LRW_DATA_BYTES, LRW_DATA_BYTES)]),
            3
        );
        // Both directions of one device cross a datagram boundary: two reads plus two writes.
        assert_eq!(
            expected_working_counter(&[mapped_device(LRW_DATA_BYTES + 1, LRW_DATA_BYTES + 1)]),
            6
        );
        // The second device begins one byte before a boundary and spans both LRWs.
        assert_eq!(
            expected_working_counter(&[mapped_device(LRW_DATA_BYTES - 1, 0), mapped_device(2, 0),]),
            3
        );
        // All input ranges precede all output ranges, even across different devices.
        assert_eq!(
            expected_working_counter(&[mapped_device(LRW_DATA_BYTES - 1, 2), mapped_device(0, 1),]),
            7
        );
    }
    #[test]
    fn topology_check_rejects_swapped_or_missing_devices() {
        let actual = EthercatDevice {
            position: 0,
            station_address: 0x1000,
            name: "test".into(),
            vendor_id: 2,
            product_id: 10,
            revision: 1,
            serial: 99,
            input_bytes: 1,
            output_bytes: 1,
        };
        let expected = ExpectedDevice {
            vendor_id: 2,
            product_id: 10,
            revision: None,
            serial: Some(99),
        };
        assert!(validate_inventory(&[expected.clone()], &[actual.clone()]).is_ok());
        assert!(validate_inventory(&[expected.clone()], &[]).is_err());
        assert!(
            validate_inventory(
                &[ExpectedDevice {
                    product_id: 11,
                    ..expected
                }],
                &[actual]
            )
            .is_err()
        );
    }
}
