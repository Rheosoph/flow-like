use crate::dp::Peripheral;

/// Operating state of the DP master
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum OperatingState {
    /// The DP master is part of the token ring but not performing any cyclic data exchange.
    Stop,
    /// All peripherals/slaves are initialized and blocked.  Cyclic data exchange is performed, but
    /// not outputs are written.
    Clear,
    /// Regular operation.  All peripherals/slaves are initialized and blocked.  Cyclic data
    /// exchange is performed with full I/O.
    Operate,
}

impl OperatingState {
    #[inline(always)]
    pub fn is_stop(self) -> bool {
        self == OperatingState::Stop
    }

    #[inline(always)]
    pub fn is_clear(self) -> bool {
        self == OperatingState::Clear
    }

    #[inline(always)]
    pub fn is_operate(self) -> bool {
        self == OperatingState::Operate
    }
}

/// Events from the last poll cycle
///
/// These events have occurred during the last poll cycle and likely need to be handled by the
/// application.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DpEvents {
    /// A full message cycle with all peripherals was completed.
    pub cycle_completed: bool,
    /// An event related to a specific peripheral occurred.
    ///
    /// The handle of the perpheral is included to identify it.
    pub peripheral: Option<(crate::dp::PeripheralHandle, crate::dp::PeripheralEvent)>,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
enum CycleState {
    /// Currently exchanging data with peripheral at the given index.
    ///
    /// **Important**: This is **not** the address, but the internal peripheral index.
    DataExchange(u8),
    /// State to indicate the a full data exchange cycle has been completed.
    CycleCompleted,
}

/// The DP master
///
/// Currently only implements a subset of DP-V0.
///
/// The DP master holds all peripherals that we interact with.  To get access, use the
/// [`PeripheralHandle`][`crate::dp::PeripheralHandle`] that you get when calling
/// [`dp_master.add()`][`crate::dp::DpMaster::add`].
///
/// When constructing the DP master, you need to pass a storage for peripherals.  This can either
/// be a fixed-size storage (slice or array) or, if `alloc`/`std` is available, a `Vec<>` that will
/// be dynamically grown to house the peripherals.
///
/// The DP master starts in the [`Stop`][`OperatingState::Stop`] state.  To communicate with
/// peripherals, you first need to move it into the [`Operate`][`OperatingState::Operate`] state
/// using the [`dp_master.enter_operate()`][`DpMaster::enter_operate`] method.
///
/// # Example
/// ```
/// use profirust::dp;
/// let buffer: [dp::PeripheralStorage; 4] = Default::default();
/// let mut dp_master = dp::DpMaster::new(buffer);
/// // or with `std`:
/// // let mut dp_master = dp::DpMaster::new(Vec::new());
///
/// // Let's add a peripheral.
/// let remoteio_address = 7;
/// let remoteio_options = dp::PeripheralOptions {
///     // ...
///     // best generated using `gsdtool`
///     // ...
///     ..Default::default()
/// };
/// let mut buffer_inputs = [0u8; 8];
/// let mut buffer_outputs = [0u8; 4];
///
/// let remoteio = dp_master.add(dp::Peripheral::new(
///     remoteio_address,
///     remoteio_options,
///     &mut buffer_inputs[..],
///     &mut buffer_outputs[..],
/// ));
///
/// dp_master.enter_operate();
/// ```
pub struct DpMaster<'a> {
    peripherals: crate::dp::PeripheralSet<'a>,
    state: DpMasterState,
}

pub struct DpMasterState {
    /// Operating State of the master.
    pub(crate) operating_state: OperatingState,

    /// Last time we sent a "Global Control" telegram to advertise our operating state.
    last_global_control: Option<crate::time::Instant>,

    /// Cycle State, tracking progress of the data exchange cycle
    cycle_state: CycleState,

    /// Last set of events that occurred
    last_events: DpEvents,

    #[cfg(feature = "debug-measure-dp-cycle")]
    last_cycle: Option<crate::time::Instant>,
}

impl<'a> DpMaster<'a> {
    pub fn new<S>(storage: S) -> Self
    where
        S: Into<managed::ManagedSlice<'a, crate::dp::PeripheralStorage<'a>>>,
    {
        let storage = storage.into();
        if storage.len() > 124 {
            log::warn!("DP master was provided with storage for more than 124 peripherals, this is wasted memory!");
        }
        Self {
            peripherals: crate::dp::PeripheralSet::new(storage),
            state: DpMasterState {
                operating_state: OperatingState::Stop,
                last_global_control: None,
                cycle_state: CycleState::DataExchange(0),
                last_events: Default::default(),
                #[cfg(feature = "debug-measure-dp-cycle")]
                last_cycle: None,
            },
        }
    }

    /// Add a peripheral to the set, and return its handle.
    ///
    /// # Panics
    /// This function panics if the storage is fixed-size (not a `Vec`) and is full.
    pub fn add(&mut self, peripheral: Peripheral<'a>) -> crate::dp::PeripheralHandle {
        self.peripherals.add(peripheral)
    }

    /// Get a peripheral from the set by its handle, as mutable.
    ///
    /// # Panics
    /// This function may panic if the handle does not belong to this peripheral set.
    pub fn get_mut(&mut self, handle: crate::dp::PeripheralHandle) -> &mut Peripheral<'a> {
        self.peripherals.get_mut(handle)
    }

    pub fn iter_mut(
        &mut self,
    ) -> impl Iterator<Item = (crate::dp::PeripheralHandle, &mut Peripheral<'a>)> {
        self.peripherals.iter_mut()
    }

    pub fn iter(&self) -> impl Iterator<Item = (crate::dp::PeripheralHandle, &Peripheral<'a>)> {
        self.peripherals.iter()
    }

    /// Return the last events set once.
    ///
    /// On consecutive calls, an empty events set it returned.  If events are not retrieved using
    /// this function, they may be overridden by newer events on the next poll cycle.
    pub fn take_last_events(&mut self) -> DpEvents {
        core::mem::take(&mut self.state.last_events)
    }

    #[inline(always)]
    pub fn operating_state(&self) -> OperatingState {
        self.state.operating_state
    }

    #[inline]
    pub fn enter_state(&mut self, state: OperatingState) {
        log::info!("DP master entering state \"{:?}\"", state);
        self.state.operating_state = state;
        // Ensure we will send a new global control telegram ASAP:
        self.state.last_global_control = None;

        // Keep the current cycle index: a reply may still be in flight when the application
        // changes state. The next transmission advertises Clear/Operate; Stop suppresses it.
    }

    /// Enter the [`Stop`][`OperatingState::Stop`] operating state.
    ///
    /// This is equivalent to calling `.enter_state(OperatingState::Stop)`.
    #[inline]
    pub fn enter_stop(&mut self) {
        self.enter_state(OperatingState::Stop)
    }

    /// Enter the [`Clear`][`OperatingState::Clear`] operating state.
    ///
    /// This is equivalent to calling `.enter_state(OperatingState::Clear)`.
    #[inline]
    pub fn enter_clear(&mut self) {
        self.enter_state(OperatingState::Clear)
    }

    /// Enter the [`Operate`][`OperatingState::Operate`] operating state.
    ///
    /// This is equivalent to calling `.enter_state(OperatingState::Operate)`.
    #[inline]
    pub fn enter_operate(&mut self) {
        self.enter_state(OperatingState::Operate)
    }

    fn increment_cycle_state(&mut self, index: u8, now: crate::time::Instant) -> bool {
        if let Some(next) = self.peripherals.get_next_index(index) {
            self.state.cycle_state = CycleState::DataExchange(next);
            false
        } else {
            #[cfg(feature = "debug-measure-dp-cycle")]
            {
                if let Some(last_cycle) = self.state.last_cycle {
                    log::debug!("DP Cycle Time: {} us", (now - last_cycle).total_micros());
                }
                self.state.last_cycle = Some(now);
            }

            self.state.cycle_state = CycleState::CycleCompleted;
            true
        }
    }
}

impl<'a> crate::fdl::FdlApplication for DpMaster<'a> {
    fn transmit_telegram(
        &mut self,
        now: crate::time::Instant,
        fdl: &crate::fdl::FdlActiveStation,
        mut tx: crate::fdl::TelegramTx,
        high_prio_only: bool,
    ) -> Option<crate::fdl::TelegramTxResponse> {
        // In STOP state, never send anything
        if self.state.operating_state.is_stop() {
            // TODO: Is overwriting the last events here the best course of action?
            self.state.last_events = DpEvents::default();
            return None;
        }

        // First check whether it is time for another global control telegram
        //
        // TODO: 50 Tsl is an arbitrary interval.  Documentation talks about 3 times the watchdog
        // period, but that seems rather arbitrary as well.
        if !high_prio_only
            && self
                .state
                .last_global_control
                .map(|t| now - t >= fdl.parameters().slot_time() * 50)
                .unwrap_or(true)
        {
            self.state.last_global_control = Some(now);
            log::trace!(
                "DP master sending global control for state {:?}",
                self.state.operating_state
            );
            // TODO: Is overwriting the last events here the best course of action?
            self.state.last_events = DpEvents::default();
            return Some(tx.send_data_telegram(
                crate::fdl::DataTelegramHeader {
                    da: 0x7f,
                    sa: fdl.parameters().address,
                    dsap: crate::consts::SAP_SLAVE_GLOBAL_CONTROL,
                    ssap: crate::consts::SAP_MASTER_MS0,
                    fc: crate::fdl::FunctionCode::Request {
                        // TODO: Do we need an FCB for GC telegrams?
                        fcb: crate::fdl::FrameCountBit::Inactive,
                        req: crate::fdl::RequestType::SdnLow,
                    },
                },
                2,
                |buf| {
                    buf[0] = match self.state.operating_state {
                        OperatingState::Clear => 0x02,
                        OperatingState::Operate => 0x00,
                        OperatingState::Stop => unreachable!(),
                    };
                    buf[1] = 0x00;
                },
            ));
        }

        let mut peripheral_event = None;
        loop {
            let index = match self.state.cycle_state {
                CycleState::DataExchange(i) => i,
                CycleState::CycleCompleted => {
                    // On CycleCompleted, return None to let the FDL know where done.  Reset the
                    // cycle state to the beginning for the next time.
                    self.state.cycle_state = CycleState::DataExchange(0);
                    self.state.last_events = DpEvents {
                        peripheral: peripheral_event,
                        ..Default::default()
                    };
                    return None;
                }
            };

            if let Some((handle, peripheral)) = self.peripherals.get_at_index_mut(index) {
                let res = peripheral.transmit_telegram(now, &self.state, fdl, tx, high_prio_only);

                match res {
                    Ok(tx_res) => {
                        // When this peripheral initiated a transmission, break out of the loop
                        self.state.last_events = DpEvents {
                            peripheral: peripheral_event,
                            ..Default::default()
                        };
                        return Some(tx_res);
                    }
                    Err((tx_returned, event)) => {
                        tx = tx_returned;

                        if let Some(event) = event {
                            // If we get here and peripheral_event were already filled, we would
                            // end up with the problem that only one event can be reported.
                            //
                            // However, lucky for us, this should never occur.  The only peripheral
                            // event we can receive in transmit_telegram() is the Offline event and
                            // there can never be a situation where multiple peripherals go offline
                            // in the same poll cycle.
                            assert!(peripheral_event.is_none());
                            peripheral_event = Some((handle, event));
                        }

                        // When this peripheral was not interested in sending data, move on to the
                        // next one.
                        if self.increment_cycle_state(index, now) {
                            // And immediately reset to the beginning for the next cycle.  This is
                            // only okay here because we are in transmit_telegram() and will return
                            // without transmission on the next line.
                            self.state.cycle_state = CycleState::DataExchange(0);
                            self.state.last_events = DpEvents {
                                cycle_completed: true,
                                peripheral: peripheral_event,
                            };
                            return None;
                        }
                    }
                }
            }
        }
    }

    fn receive_reply(
        &mut self,
        now: crate::time::Instant,
        fdl: &crate::fdl::FdlActiveStation,
        addr: u8,
        telegram: crate::fdl::Telegram,
    ) {
        let index = match self.state.cycle_state {
            CycleState::DataExchange(i) => i,
            CycleState::CycleCompleted => {
                unreachable!("impossible to get a reply when the cycle was completed!");
            }
        };
        match self.peripherals.get_at_index_mut(index) {
            Some((handle, peripheral)) if addr == peripheral.address() => {
                let event = peripheral.receive_reply(now, &self.state, fdl, telegram);
                let cycle_completed = self.increment_cycle_state(index, now);
                self.state.last_events = DpEvents {
                    cycle_completed,
                    peripheral: event.map(|ev| (handle, ev)),
                };
            }
            _ => {
                unreachable!(
                    "Received reply for unknown/unexpected peripheral #{addr}: {telegram:?}"
                );
            }
        }
    }

    fn handle_timeout(
        &mut self,
        now: crate::time::Instant,
        fdl: &crate::fdl::FdlActiveStation,
        addr: u8,
    ) {
        // At this time, there is no meaningful action to take in response to this.  Timeout
        // handling is actually done as part of the transmit_telegram() code.
        //
        // log::warn!("Timeout while waiting for response from #{}!", addr);
    }
}
