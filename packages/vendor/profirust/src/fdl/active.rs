//! Implementation of an FDL active station.

#![deny(unused_must_use)]
use crate::fdl::FdlApplication;
use crate::phy::ProfibusPhy;

/// Operating state of the FDL active station
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum ConnectivityState {
    /// The station is not participating in bus communication in any way.
    Offline,
    /// The station is only passive.
    ///
    /// It will respond to requests, but it will not attempt to become part of the token ring.
    Passive,
    /// The station tries to become part of the token ring and then performs communication.
    Online,
}

impl ConnectivityState {
    #[inline(always)]
    pub fn is_offline(self) -> bool {
        self == ConnectivityState::Offline
    }

    #[inline(always)]
    pub fn is_passive(self) -> bool {
        self == ConnectivityState::Passive
    }

    #[inline(always)]
    pub fn is_online(self) -> bool {
        self == ConnectivityState::Online
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum GapState {
    /// Waiting for some time until the next gap polling cycle is performed.
    ///
    /// The `rotation_count` value is the number of token rotations since the last polling cycle.
    Waiting { rotation_count: u8 },

    /// A poll of the given address is scheduled next.
    DoPoll { current_address: crate::Address },
}

impl GapState {
    pub fn increment_wait(&mut self) {
        match self {
            GapState::Waiting {
                ref mut rotation_count,
            } => *rotation_count += 1,
            _ => (),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum PassTokenAttempt {
    First,
    Second,
    Third,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum ScheduleNext {
    Scheduled,
    CycleCompleted,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
struct UseTokenData {
    pub token_time: crate::time::Instant,
    pub first_app: Option<usize>,
}

impl UseTokenData {
    pub fn with_token_time(token_time: crate::time::Instant) -> Self {
        Self {
            token_time,
            first_app: None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum State {
    Offline,
    PassiveIdle,
    ListenToken {
        status_request: Option<crate::Address>,
        collision_count: u8,
    },
    ActiveIdle {
        status_request: Option<crate::Address>,
        new_previous_station: Option<crate::Address>,
        collision_count: u8,
    },
    UseToken {
        data: UseTokenData,
        first_cycle_done: bool,
    },
    ClaimToken {
        first: bool,
    },
    AwaitDataResponse {
        address: crate::Address,
        data: UseTokenData,
    },
    PassToken {
        do_gap: bool,
        attempt: PassTokenAttempt,
    },
    CheckTokenPass {
        attempt: PassTokenAttempt,
    },
    AwaitStatusResponse {
        address: crate::Address,
    },
}

macro_rules! debug_assert_state {
    ($state:expr, $expected:pat) => {
        debug_assert!(
            matches!($state, $expected),
            "unexpected state: {:?}",
            $state
        )
    };
}

impl State {
    pub fn have_token(&self) -> bool {
        match self {
            State::Offline { .. }
            | State::PassiveIdle { .. }
            | State::ListenToken { .. }
            | State::ActiveIdle { .. }
            | State::PassToken { .. }
            | State::CheckTokenPass { .. } => false,
            State::ClaimToken { .. }
            | State::UseToken { .. }
            | State::AwaitDataResponse { .. }
            | State::AwaitStatusResponse { .. } => true,
        }
    }

    fn transition_offline(&mut self) {
        debug_assert_state!(
            self,
            State::Offline { .. }
                | State::PassiveIdle { .. }
                | State::ListenToken { .. }
                | State::PassToken { .. }
        );
        *self = State::Offline
    }

    fn transition_passive_idle(&mut self) {
        debug_assert_state!(self, State::Offline { .. } | State::PassiveIdle { .. });
        *self = State::PassiveIdle;
    }

    fn transition_listen_token(&mut self) {
        debug_assert_state!(
            self,
            State::ListenToken { .. } | State::Offline { .. } | State::ActiveIdle { .. }
        );
        *self = State::ListenToken {
            status_request: None,
            collision_count: 0,
        };
    }

    fn transition_active_idle(&mut self) {
        debug_assert_state!(
            self,
            State::ActiveIdle { .. }
                | State::ListenToken { .. }
                | State::UseToken { .. }
                | State::AwaitDataResponse { .. }
                | State::CheckTokenPass { .. }
                | State::AwaitStatusResponse { .. }
        );
        *self = State::ActiveIdle {
            status_request: None,
            new_previous_station: None,
            collision_count: 0,
        };
    }

    fn transition_use_token(&mut self, data: UseTokenData) {
        debug_assert_state!(
            self,
            State::UseToken { .. }
                | State::ClaimToken { .. }
                | State::PassToken { .. }
                | State::AwaitDataResponse { .. }
                | State::ActiveIdle { .. }
        );
        *self = State::UseToken {
            data,
            first_cycle_done: false,
        };
    }

    fn transition_claim_token(&mut self) {
        debug_assert_state!(
            self,
            State::ClaimToken { .. } | State::ListenToken { .. } | State::ActiveIdle { .. }
        );
        *self = State::ClaimToken { first: true };
    }

    fn transition_await_data_response(&mut self, address: crate::Address, data: UseTokenData) {
        debug_assert_state!(
            self,
            State::AwaitDataResponse { .. } | State::UseToken { .. }
        );
        *self = State::AwaitDataResponse { address, data };
    }

    fn transition_pass_token(&mut self, do_gap: bool, attempt: PassTokenAttempt) {
        debug_assert_state!(
            self,
            State::PassToken { .. }
                | State::UseToken { .. }
                | State::ClaimToken { .. }
                | State::CheckTokenPass { .. }
                | State::AwaitStatusResponse { .. }
        );
        *self = State::PassToken { do_gap, attempt };
    }

    fn transition_check_token_pass(&mut self, attempt: PassTokenAttempt) {
        debug_assert_state!(self, State::CheckTokenPass { .. } | State::PassToken { .. });
        *self = State::CheckTokenPass { attempt };
    }

    fn transition_await_status_response(&mut self, address: crate::Address) {
        debug_assert_state!(
            self,
            State::AwaitStatusResponse { .. } | State::PassToken { .. }
        );
        *self = State::AwaitStatusResponse { address };
    }
}

/// Accessors for state-specific fields.  These accessors panic when trying to access a field
/// from a different state.
impl State {
    fn get_listen_token_status_request(&mut self) -> &mut Option<crate::Address> {
        match self {
            Self::ListenToken { status_request, .. } => status_request,
            _ => unreachable!(),
        }
    }

    fn get_listen_token_collision_count(&mut self) -> &mut u8 {
        match self {
            Self::ListenToken {
                collision_count, ..
            } => collision_count,
            _ => unreachable!(),
        }
    }

    fn get_active_idle_status_request(&mut self) -> &mut Option<crate::Address> {
        match self {
            Self::ActiveIdle { status_request, .. } => status_request,
            _ => unreachable!(),
        }
    }

    fn get_active_idle_new_previous_station(&mut self) -> &mut Option<crate::Address> {
        match self {
            Self::ActiveIdle {
                new_previous_station,
                ..
            } => new_previous_station,
            _ => unreachable!(),
        }
    }

    fn get_active_idle_collision_count(&mut self) -> &mut u8 {
        match self {
            Self::ActiveIdle {
                collision_count, ..
            } => collision_count,
            _ => unreachable!(),
        }
    }

    fn get_use_token_data(&mut self) -> &mut UseTokenData {
        match self {
            Self::UseToken { data, .. } => data,
            _ => unreachable!(),
        }
    }

    fn get_use_token_first_cycle_done(&mut self) -> &mut bool {
        match self {
            Self::UseToken {
                first_cycle_done, ..
            } => first_cycle_done,
            _ => unreachable!(),
        }
    }

    fn get_claim_token_first(&mut self) -> &mut bool {
        match self {
            Self::ClaimToken { first, .. } => first,
            _ => unreachable!(),
        }
    }

    fn get_await_data_response_address(&mut self) -> &mut crate::Address {
        match self {
            Self::AwaitDataResponse { address, .. } => address,
            _ => unreachable!(),
        }
    }

    fn get_await_data_response_data(&mut self) -> &mut UseTokenData {
        match self {
            Self::AwaitDataResponse { data, .. } => data,
            _ => unreachable!(),
        }
    }

    fn get_pass_token_do_gap(&mut self) -> &mut bool {
        match self {
            Self::PassToken { do_gap, .. } => do_gap,
            _ => unreachable!(),
        }
    }

    fn get_pass_token_attempt(&mut self) -> &mut PassTokenAttempt {
        match self {
            Self::PassToken { attempt, .. } => attempt,
            _ => unreachable!(),
        }
    }

    fn get_await_status_response_address(&mut self) -> &mut crate::Address {
        match self {
            Self::AwaitStatusResponse { address, .. } => address,
            _ => unreachable!(),
        }
    }

    fn get_check_token_pass_attempt(&mut self) -> &mut PassTokenAttempt {
        match self {
            Self::CheckTokenPass { attempt, .. } => attempt,
            _ => unreachable!(),
        }
    }
}

#[derive(Debug)]
pub struct FdlActiveStation {
    /// Parameters for the connected bus and this station
    p: crate::fdl::Parameters,

    /// Management of the token ring
    token_ring: crate::fdl::TokenRing,

    /// Connectivity status of this station
    connectivity_state: ConnectivityState,

    // State of GAP polling
    gap_state: GapState,

    /// State of the active station
    state: State,

    /// Timestamp of the last time we found the bus to be active (= someone transmitting)
    last_bus_activity: Option<crate::time::Instant>,

    /// Amount of bytes pending in the receive buffer.
    ///
    /// This known value is compared to the latest one reported by the PHY to find out whether new
    /// data was received since the last poll.
    pending_bytes: usize,

    /// Timestamp of the acquisition of the last token.
    last_token_time: crate::time::Instant,

    /// Timestamp of the end of our token hold time.
    end_token_hold_time: crate::time::Instant,

    /// Index of the application that gets to transmit a telegram next.
    next_application: usize,
}

impl FdlActiveStation {
    pub fn new(param: crate::fdl::Parameters) -> Self {
        param.debug_assert_consistency();

        Self {
            token_ring: crate::fdl::TokenRing::new(&param),
            // A station must always start offline
            connectivity_state: ConnectivityState::Offline,
            gap_state: GapState::DoPoll {
                current_address: param.address,
            },
            state: State::Offline,
            last_bus_activity: None,
            pending_bytes: 0,
            last_token_time: crate::time::Instant::ZERO,
            end_token_hold_time: crate::time::Instant::ZERO,
            next_application: 0,
            p: param,
        }
    }

    /// Return a reference to the parameters configured for this FDL active station.
    #[inline(always)]
    pub fn parameters(&self) -> &crate::fdl::Parameters {
        &self.p
    }

    #[inline(always)]
    pub fn connectivity_state(&self) -> ConnectivityState {
        self.connectivity_state
    }

    #[inline]
    pub fn set_state(&mut self, state: ConnectivityState) {
        log::info!("FDL active station entering state \"{:?}\"", state);
        self.connectivity_state = state;

        if state == ConnectivityState::Offline {
            // If we are going offline, reset all internal state by recreating the FDL station.
            let parameters = core::mem::take(&mut self.p);
            *self = Self::new(parameters);
        } else if state != ConnectivityState::Online {
            todo!(
                "ConnectivityState {:?} is not yet supported properly!",
                state
            );
        }
    }

    /// Enter the [`Offline`][`ConnectivityState::Offline`] connectivity state.
    ///
    /// This is equivalent to calling `.set_state(ConnectivityState::Offline)`.
    #[inline]
    pub fn set_offline(&mut self) {
        self.set_state(ConnectivityState::Offline)
    }

    /// Enter the [`Passive`][`ConnectivityState::Passive`] connectivity state.
    ///
    /// This is equivalent to calling `.set_state(ConnectivityState::Passive)`.
    #[inline]
    pub fn set_passive(&mut self) {
        self.set_state(ConnectivityState::Passive)
    }

    /// Enter the [`Online`][`ConnectivityState::Online`] connectivity state.
    ///
    /// This is equivalent to calling `.set_state(ConnectivityState::Online)`.
    #[inline]
    pub fn set_online(&mut self) {
        self.set_state(ConnectivityState::Online)
    }

    /// Returns `true` when this active stations believes to be in the token ring.
    pub fn is_in_ring(&self) -> bool {
        matches!(
            self.state,
            State::UseToken { .. }
                | State::PassToken { .. }
                | State::ActiveIdle { .. }
                | State::ClaimToken { .. }
                | State::CheckTokenPass { .. }
                | State::AwaitDataResponse { .. }
                | State::AwaitStatusResponse { .. }
        )
    }

    #[doc(hidden)]
    pub fn inspect_token_ring(&self) -> &crate::fdl::TokenRing {
        &self.token_ring
    }
}

#[must_use = "\"poll done\" marker must lead to exit of poll function!"]
struct PollDone();

#[must_use = "\"poll result\" must lead to exit of poll function!"]
struct PollResult<E> {
    events: E,
}

impl PollDone {
    pub fn waiting_for_transmission() -> Self {
        PollDone()
    }

    pub fn waiting_for_bus() -> Self {
        PollDone()
    }

    pub fn waiting_for_delay() -> Self {
        PollDone()
    }

    pub fn offline() -> Self {
        PollDone()
    }

    pub fn with_events<E>(self, events: E) -> PollResult<E> {
        PollResult { events }
    }
}

impl<E: Default> From<PollDone> for PollResult<E> {
    fn from(value: PollDone) -> Self {
        PollResult {
            events: Default::default(),
        }
    }
}

macro_rules! return_if_done {
    ($expr:expr) => {
        match $expr {
            Some(e) => return e.into(),
            None => (),
        }
    };
}

impl FdlActiveStation {
    /// Mark the bus as active at the current point in time.
    ///
    /// Sets the current time as last_bus_activity unless we have already deduced that bus activity
    /// will continue until some point in the future.
    fn mark_bus_activity(&mut self, now: crate::time::Instant) {
        let last = self.last_bus_activity.get_or_insert(now);
        *last = (*last).max(now);
    }

    /// Check whether a transmission is currently ongoing.
    ///
    /// There are two scenarios where an ongoing transmission is detected:
    ///
    /// 1. If the PHY reports that it is still transmitting.
    /// 2. If we believe that we must still be sending data from timing calculations.
    fn check_for_ongoing_transmision(
        &mut self,
        now: crate::time::Instant,
        phy: &mut impl ProfibusPhy,
    ) -> Option<PollDone> {
        let phy_transmitting = phy.poll_transmission(now);
        if phy_transmitting || self.last_bus_activity.map(|l| now <= l).unwrap_or(false) {
            self.mark_bus_activity(now);
            Some(PollDone::waiting_for_transmission())
        } else {
            None
        }
    }

    /// Wait for 33 bit times since last bus activity.
    ///
    /// This synchronization pause is required before every transmission.
    fn wait_synchronization_pause(&mut self, now: crate::time::Instant) -> Option<PollDone> {
        if now <= (*self.last_bus_activity.get_or_insert(now) + self.p.bits_to_time(33)) {
            Some(PollDone::waiting_for_delay())
        } else {
            None
        }
    }

    /// Marks transmission starting `now` and continuing for `bytes` length.
    fn mark_tx(&mut self, now: crate::time::Instant, bytes: usize) -> PollDone {
        self.last_bus_activity = Some(
            now + self
                .p
                .baudrate
                .bits_to_time(11 * u32::try_from(bytes).unwrap()),
        );
        PollDone::waiting_for_transmission()
    }

    fn check_for_bus_activity(&mut self, now: crate::time::Instant, phy: &mut impl ProfibusPhy) {
        let pending_bytes = phy.poll_pending_received_bytes(now);
        if pending_bytes > self.pending_bytes {
            self.mark_bus_activity(now);
            self.pending_bytes = pending_bytes;
        }
    }

    /// Mark receival of a telegram.
    fn mark_rx(&mut self, now: crate::time::Instant) {
        self.pending_bytes = 0;
        self.mark_bus_activity(now);
    }

    /// Check whether the time to respond has passed without initiation of a response.
    fn check_slot_expired(&mut self, now: crate::time::Instant) -> bool {
        // We have two situations:
        // 1. Either the slot expires without any repsonse activity at all
        // 2. Or we received some bytes, but not a full telegram
        let last_bus_activity = *self.last_bus_activity.get_or_insert(now);
        if self.pending_bytes == 0 {
            now > (last_bus_activity + self.p.slot_time())
        } else {
            // TODO: Technically, no inter-character delay is allowed at all but we are in a rough
            // spot here.  The peripheral will most likely continue transmitting data in a short
            // while so let's be conservative and wait an entire slot time again after partial
            // receival.
            //
            // The tricky part here is that this timeout also becomes very relevant on non-realtime
            // systems like a vanilla Linux where PROFIBUS communication happens over USB.  We can
            // have longer delays between consecutive characters there.  For example, sometimes
            // data is received in chunks of 32 bytes.  This obviously looks like a large
            // inter-character delay that we need to be robust against.
            now > (last_bus_activity + self.p.slot_time())
        }
    }
}

impl FdlActiveStation {
    #[must_use = "poll done marker"]
    fn handle_lost_token(
        &mut self,
        now: crate::time::Instant,
        phy: &mut impl ProfibusPhy,
    ) -> Option<PollDone> {
        // If we do not know of any previous bus activity, conservatively assume that the last
        // activity was just now and start counting from here...
        let last_bus_activity = *self.last_bus_activity.get_or_insert(now);
        if (now - last_bus_activity) >= self.p.token_lost_timeout() {
            if self.token_ring.ready_for_ring() {
                log::warn!("Token lost! Generating a new one.");
            } else {
                log::info!("Generating new token due to silent bus.");
            }

            self.state.transition_claim_token();
            Some(self.do_claim_token(now, phy))
        } else {
            None
        }
    }

    fn next_gap_poll(&self, current_address: crate::Address) -> GapState {
        let next_station = self.token_ring.next_station();
        let next_address = if current_address == (self.p.highest_station_address - 1) {
            0
        } else {
            current_address + 1
        };

        if next_address >= next_station && next_station > self.p.address {
            // We have reached the end of the GAP, enter waiting state.
            GapState::Waiting { rotation_count: 0 }
        } else if next_address >= next_station
            && next_station < self.p.address
            && next_address < self.p.address
        {
            // We have reached the end of the GAP, enter waiting state (wrap-around GAP case).
            GapState::Waiting { rotation_count: 0 }
        } else {
            GapState::DoPoll {
                current_address: next_address,
            }
        }
    }
}

/// State Machine of the FDL active station
impl FdlActiveStation {
    #[must_use = "poll done marker"]
    fn do_listen_token<'a, PHY: ProfibusPhy>(
        &mut self,
        now: crate::time::Instant,
        phy: &mut PHY,
    ) -> PollDone {
        debug_assert_state!(self.state, State::ListenToken { .. });

        return_if_done!(self.handle_lost_token(now, phy));

        // Handle pending response to a telegram request we received
        if let Some(status_request_source) = *self.state.get_listen_token_status_request() {
            return_if_done!(self.wait_synchronization_pause(now));

            // We must only respond to be ready (=without token) when the request is sent by our
            // known previous neighbor station.
            let state = if self.token_ring.ready_for_ring()
                && status_request_source == self.token_ring.previous_station()
            {
                crate::fdl::ResponseState::MasterWithoutToken
            } else {
                crate::fdl::ResponseState::MasterNotReady
            };

            let tx_res = phy
                .transmit_telegram(now, |tx| {
                    Some(tx.send_fdl_status_response(
                        status_request_source,
                        self.p.address,
                        state,
                        crate::fdl::ResponseStatus::Ok,
                    ))
                })
                .unwrap();

            if self.token_ring.ready_for_ring() {
                self.state.transition_active_idle();
            } else {
                *self.state.get_listen_token_status_request() = None;
            }
            return self.mark_tx(now, tx_res.bytes_sent());
        }

        // Handle received telegrams
        phy.receive_all_telegrams(now, |telegram, is_last_telegram| {
            self.mark_rx(now);

            // This unusual construct is needed to catch situations where multiple telegrams are
            // received at once and the first one leads us to go offline due to collision.
            if self.connectivity_state() == ConnectivityState::Offline {
                return PollDone::waiting_for_bus();
            }

            // Handle address collision detection
            if telegram.source_address() == Some(self.p.address) {
                let collision_count = self.state.get_listen_token_collision_count();

                *collision_count += 1;

                match *collision_count {
                    1 => {
                        log::warn!("Witnessed collision of another active station with own address (#{})!", self.p.address);
                    }
                    2 | _ => {
                        log::warn!(
                            "Witnessed second collision of another active station with own address (#{}), going offline.",
                            self.p.address,
                        );
                        self.set_offline();
                    }
                }
                return PollDone::waiting_for_bus();
            }

            match telegram {
                // Handle witnessing a token telegram
                crate::fdl::Telegram::Token(token_telegram) => {
                    self.token_ring.witness_token_pass(token_telegram.sa, token_telegram.da);
                    PollDone::waiting_for_bus()
                }

                // Handle FDL requests sent to us
                crate::fdl::Telegram::Data(data_telegram)
                    if data_telegram.is_fdl_status_request().is_some()
                        && data_telegram.h.da == self.p.address =>
                {
                    if is_last_telegram {
                        *self.state.get_listen_token_status_request() = Some(data_telegram.h.sa);
                        PollDone::waiting_for_delay()
                    } else {
                        PollDone::waiting_for_bus()
                    }
                }
                _ => PollDone::waiting_for_bus(),
            }
        }).unwrap_or(PollDone::waiting_for_bus())
    }

    fn handle_telegram(
        &mut self,
        now: crate::time::Instant,
        telegram: crate::fdl::Telegram,
        is_last_telegram: bool,
    ) -> PollDone {
        // This unusual construct is needed to catch situations where multiple telegrams are
        // received at once and the first one leads us to go back to ListenToken due to collision.
        if matches!(self.state, State::ListenToken { .. }) {
            return PollDone::waiting_for_bus();
        }

        debug_assert_state!(self.state, State::ActiveIdle { .. });

        match telegram {
            // Handle any token telegrams
            crate::fdl::Telegram::Token(token_telegram) => {
                let collision_count = self.state.get_active_idle_collision_count();
                if token_telegram.sa == self.p.address {
                    *collision_count += 1;

                    match *collision_count {
                        1 => {
                            log::warn!("Witnessed collision of another active station with own address (#{})!", self.p.address);
                        }
                        2 | _ => {
                            log::warn!(
                                "Witnessed second collision of another active station with own address (#{}), leaving ring.",
                                self.p.address,
                            );
                            self.state.transition_listen_token();
                        }
                    }
                    return PollDone::waiting_for_bus();
                } else {
                    // Collisions are only counted in ActiveIdle if we see them happening back to
                    // back.
                    *collision_count = 0;
                }

                // Witness token passes between other stations and token passes to us that are
                // followed by more received telegrams.
                if token_telegram.da != self.p.address || !is_last_telegram {
                    self.token_ring
                        .witness_token_pass(token_telegram.sa, token_telegram.da);

                    PollDone::waiting_for_bus()
                } else {
                    // We may only accept the token from the known neighbor (on their first try)
                    if token_telegram.sa == self.token_ring.previous_station() {
                        self.state
                            .transition_use_token(UseTokenData::with_token_time(now));
                        PollDone::waiting_for_delay()
                    } else {
                        match *self.state.get_active_idle_new_previous_station() {
                            Some(address) if address == token_telegram.sa => {
                                // We have seen this previous_station before, so accept the
                                // token.
                                self.token_ring
                                    .witness_token_pass(token_telegram.sa, token_telegram.da);
                                self.state
                                    .transition_use_token(UseTokenData::with_token_time(now));
                                PollDone::waiting_for_delay()
                            }
                            _ => {
                                // Unknown, pend the address for receiving the retry.
                                *self.state.get_active_idle_new_previous_station() =
                                    Some(token_telegram.sa);
                                PollDone::waiting_for_bus()
                            }
                        }
                    }
                }
            }

            // Handle FDL requests sent to us
            crate::fdl::Telegram::Data(data_telegram)
                if data_telegram.is_fdl_status_request().is_some()
                    && data_telegram.h.da == self.p.address
                    && is_last_telegram =>
            {
                *self.state.get_active_idle_status_request() = Some(data_telegram.h.sa);
                PollDone::waiting_for_delay()
            }
            _ => PollDone::waiting_for_bus(),
        }
    }

    #[must_use = "poll done marker"]
    fn do_active_idle<'a, PHY: ProfibusPhy>(
        &mut self,
        now: crate::time::Instant,
        phy: &mut PHY,
    ) -> PollDone {
        debug_assert_state!(self.state, State::ActiveIdle { .. });

        return_if_done!(self.handle_lost_token(now, phy));

        // Handle pending response to a telegram request we received
        if let Some(status_request_source) = *self.state.get_active_idle_status_request() {
            return_if_done!(self.wait_synchronization_pause(now));

            let tx_res = phy
                .transmit_telegram(now, |tx| {
                    Some(tx.send_fdl_status_response(
                        status_request_source,
                        self.p.address,
                        crate::fdl::ResponseState::MasterInRing,
                        crate::fdl::ResponseStatus::Ok,
                    ))
                })
                .unwrap();

            *self.state.get_active_idle_status_request() = None;
            return self.mark_tx(now, tx_res.bytes_sent());
        }

        phy.receive_all_telegrams(now, |telegram, is_last_telegram| {
            self.mark_rx(now);

            self.handle_telegram(now, telegram, is_last_telegram)
        })
        .unwrap_or(PollDone::waiting_for_bus())
    }

    #[must_use = "poll done marker"]
    fn do_claim_token<'a, PHY: ProfibusPhy>(
        &mut self,
        now: crate::time::Instant,
        phy: &mut PHY,
    ) -> PollDone {
        debug_assert_state!(self.state, State::ClaimToken { .. });

        // The token is claimed by sending a telegram to ourselves twice.
        return_if_done!(self.wait_synchronization_pause(now));
        let tx_res = phy
            .transmit_telegram(now, |tx| {
                Some(tx.send_token_telegram(self.p.address, self.p.address))
            })
            .unwrap();

        self.token_ring.claim_token();

        if *self.state.get_claim_token_first() {
            // This will lead to sending the claim token telegram again
            *self.state.get_claim_token_first() = false;
        } else {
            // Now we have claimed the token and can proceed to use it.
            self.state
                .transition_use_token(UseTokenData::with_token_time(now));
        }

        self.mark_tx(now, tx_res.bytes_sent())
    }

    #[must_use = "poll done marker"]
    fn app_transmit_telegram<PHY: ProfibusPhy>(
        &mut self,
        now: crate::time::Instant,
        phy: &mut PHY,
        app: &mut dyn FdlApplication,
        high_prio_only: bool,
    ) -> Option<PollDone> {
        if let Some(tx_res) = phy.transmit_telegram(now, |tx| {
            let res = app.transmit_telegram(now, self, tx, high_prio_only);
            res
        }) {
            if let Some(addr) = tx_res.expects_reply() {
                let data = *self.state.get_use_token_data();
                self.state.transition_await_data_response(addr, data);
            }
            Some(self.mark_tx(now, tx_res.bytes_sent()))
        } else {
            None
        }
    }

    fn schedule_next_application(&mut self, num_apps: usize) -> ScheduleNext {
        let data = self.state.get_use_token_data();
        let first_app = *data.first_app.get_or_insert(self.next_application);
        self.next_application = (self.next_application + 1) % num_apps;
        if self.next_application == first_app {
            ScheduleNext::CycleCompleted
        } else {
            ScheduleNext::Scheduled
        }
    }

    #[must_use = "poll done marker"]
    fn apps_transmit_telegram<PHY: ProfibusPhy>(
        &mut self,
        now: crate::time::Instant,
        phy: &mut PHY,
        apps: &mut [&mut dyn FdlApplication],
        high_prio_only: bool,
    ) -> Option<PollDone> {
        for _ in 0..apps.len() {
            // TODO: Need to deal with the application list changing size.  Currently, this will lead
            // to a panic or unexpected telegrams being forwarded to an application.
            let current_app = &mut apps[self.next_application];
            let res = self.app_transmit_telegram(now, phy, *current_app, high_prio_only);
            return_if_done!(res);

            // The previous application claims its cycle is done, so the next application can take
            // over.
            if self.schedule_next_application(apps.len()) == ScheduleNext::CycleCompleted {
                // All applications completed their cycle once since we got the token, now it's
                // time to pass the token.
                break;
            }
        }
        None
    }

    #[must_use = "poll done marker"]
    fn do_use_token<PHY: ProfibusPhy>(
        &mut self,
        now: crate::time::Instant,
        phy: &mut PHY,
        apps: &mut [&mut dyn FdlApplication],
    ) -> PollDone {
        debug_assert_state!(self.state, State::UseToken { .. });

        let data = *self.state.get_use_token_data();
        if self.last_token_time != data.token_time {
            self.end_token_hold_time = self.last_token_time + self.p.token_rotation_time();
            self.last_token_time = data.token_time;

            if let GapState::DoPoll { .. } = self.gap_state {
                // Subtract the gap poll time from the end_token_hold_time so we leave time for
                // polling the gap.
                self.end_token_hold_time -= self.p.bits_to_time(u32::from(self.p.slot_bits) + 100);
            }
        }

        return_if_done!(self.wait_synchronization_pause(now));

        if now < self.end_token_hold_time {
            *self.state.get_use_token_first_cycle_done() = true;
            return_if_done!(self.apps_transmit_telegram(now, phy, apps, false));
        } else if !*self.state.get_use_token_first_cycle_done() {
            // Do one high priority message cycle
            *self.state.get_use_token_first_cycle_done() = true;
            return_if_done!(self.apps_transmit_telegram(now, phy, apps, true));
        }

        self.state
            .transition_pass_token(true, PassTokenAttempt::First);

        PollDone::waiting_for_delay()
    }

    fn do_await_data_response<PHY: ProfibusPhy>(
        &mut self,
        now: crate::time::Instant,
        phy: &mut PHY,
        apps: &mut [&mut dyn FdlApplication],
    ) -> PollDone {
        debug_assert_state!(self.state, State::AwaitDataResponse { .. });

        let address = *self.state.get_await_data_response_address();
        let data = *self.state.get_await_data_response_data();

        // TODO: Need to deal with the application list changing size.  Currently, this will lead
        // to a panic or unexpected telegrams being forwarded to an application.
        let app = &mut apps[self.next_application];

        // Here we conservatively only receive the first pending telegram because it is very
        // unlikely that some other station randomly stole our token.  If it did, we will notice in
        // the next poll cycle.
        let reply_events: Result<Option<()>, PollDone> = phy
            .receive_telegram(now, |telegram| {
                self.mark_rx(now);

                let is_valid_response = match &telegram {
                    crate::fdl::Telegram::Token(_) => false,
                    crate::fdl::Telegram::ShortConfirmation(_) => true,
                    crate::fdl::Telegram::Data(t) => {
                        t.h.sa == address && t.h.da == self.p.address && matches!(t.h.fc, crate::fdl::FunctionCode::Response { .. })
                    },
                };

                if is_valid_response {
                    Ok(Some(app.receive_reply(now, self, address, telegram)))
                } else {
                    // When receiving a valid telegram that isn't a valid response, something went
                    // wrong and we must go back to active idle state.
                    log::warn!("Received unexpected telegram while waiting for reply from #{address}: {:?}", telegram);
                    self.state.transition_active_idle();
                    Err(PollDone::waiting_for_bus())
                }
            })
            .unwrap_or(Ok(None));

        match reply_events {
            Err(d) => {
                return d.into();
            }
            Ok(Some(())) => {
                self.state.transition_use_token(data);
                *self.state.get_use_token_first_cycle_done() = true;
                return PollDone::waiting_for_delay();
            }
            Ok(None) => (),
        }

        if self.check_slot_expired(now) {
            app.handle_timeout(now, self, address);
            self.state.transition_use_token(data);
            *self.state.get_use_token_first_cycle_done() = true;

            // Immediately evaluate UseToken state because the bus is free for immediate
            // transmission
            self.do_use_token(now, phy, apps)
        } else {
            PollDone::waiting_for_bus()
        }
    }

    #[must_use = "poll done marker"]
    fn do_pass_token<'a, PHY: ProfibusPhy>(
        &mut self,
        now: crate::time::Instant,
        phy: &mut PHY,
    ) -> PollDone {
        debug_assert_state!(self.state, State::PassToken { .. });

        return_if_done!(self.wait_synchronization_pause(now));

        if *self.state.get_pass_token_do_gap() {
            match &mut self.gap_state {
                GapState::Waiting {
                    ref mut rotation_count,
                } => {
                    if *rotation_count > self.p.gap_wait_rotations {
                        // We're done waiting, do a poll now!
                        log::debug!("Starting next gap polling cycle!");
                        self.gap_state = self.next_gap_poll(self.p.address);
                    } else {
                        *rotation_count += 1;
                    }
                }
                GapState::DoPoll { current_address } => {
                    let current_address = *current_address;
                    self.gap_state = self.next_gap_poll(current_address);
                }
            }

            if let GapState::DoPoll { current_address } = self.gap_state {
                let tx_res = phy
                    .transmit_telegram(now, |tx| {
                        Some(tx.send_fdl_status_request(current_address, self.p.address))
                    })
                    .unwrap();

                self.state.transition_await_status_response(current_address);

                return self.mark_tx(now, tx_res.bytes_sent());
            }
        }

        let tx_res = phy
            .transmit_telegram(now, |tx| {
                Some(tx.send_token_telegram(self.token_ring.next_station(), self.p.address))
            })
            .unwrap();

        self.token_ring
            .witness_token_pass(self.p.address, self.token_ring.next_station());

        if self.token_ring.next_station() == self.p.address {
            self.state
                .transition_use_token(UseTokenData::with_token_time(now));
        } else {
            let attempt = *self.state.get_pass_token_attempt();
            self.state.transition_check_token_pass(attempt);
        }

        self.mark_tx(now, tx_res.bytes_sent())
    }

    #[must_use = "poll done marker"]
    fn do_await_status_response<'a, PHY: ProfibusPhy>(
        &mut self,
        now: crate::time::Instant,
        phy: &mut PHY,
    ) -> PollDone {
        debug_assert_state!(self.state, State::AwaitStatusResponse { .. });

        let address = *self.state.get_await_status_response_address();

        // Here we conservatively only receive the first pending telegram because it is very
        // unlikely that some other station randomly stole our token.  If it did, we will notice in
        // the next poll cycle.
        let received = phy.receive_telegram(now, |telegram| {
            self.mark_rx(now);

            if let crate::fdl::Telegram::Data(telegram) = &telegram {
                if telegram.h.sa == address && telegram.h.da == self.p.address {
                    if let crate::fdl::FunctionCode::Response { state, status } = telegram.h.fc {
                        log::trace!("Address #{address} responded");
                        if status == crate::fdl::ResponseStatus::Ok
                            && matches!(state, crate::fdl::ResponseState::MasterWithoutToken | crate::fdl::ResponseState::MasterInRing) {
                            self.token_ring.set_next_station(address);
                        }
                        self.state.transition_pass_token(false, PassTokenAttempt::First);
                        return PollDone::waiting_for_delay();
                    }
                }

            }

            log::warn!("Received unexpected telegram while waiting for status reply from #{address}: {telegram:?}");
            self.state.transition_active_idle();
            PollDone::waiting_for_bus()
        });

        if let Some(res) = received {
            return res;
        }

        if self.check_slot_expired(now) {
            log::trace!("No reply from #{address}");
            self.state
                .transition_pass_token(false, PassTokenAttempt::First);
            // Immediately evaluate PassToken state because the bus is free for immediate
            // transmission
            self.do_pass_token(now, phy)
        } else {
            PollDone::waiting_for_bus()
        }
    }

    #[must_use = "poll done marker"]
    fn do_check_token_pass<'a, PHY: ProfibusPhy>(
        &mut self,
        now: crate::time::Instant,
        phy: &mut PHY,
    ) -> PollDone {
        debug_assert_state!(self.state, State::CheckTokenPass { .. });

        if self.check_slot_expired(now) {
            match *self.state.get_check_token_pass_attempt() {
                PassTokenAttempt::First => {
                    log::warn!(
                        "Token was apparently not received by #{}, resending...",
                        self.token_ring.next_station()
                    );
                    self.state
                        .transition_pass_token(false, PassTokenAttempt::Second);
                }
                PassTokenAttempt::Second => {
                    log::warn!(
                        "Token was again not received by #{}, resending...",
                        self.token_ring.next_station()
                    );
                    self.state
                        .transition_pass_token(false, PassTokenAttempt::Third);
                }
                PassTokenAttempt::Third => {
                    log::warn!(
                        "Token was also not received on third attempt, clearing #{} from LAS.",
                        self.token_ring.next_station()
                    );
                    self.token_ring
                        .remove_station(self.token_ring.next_station());
                    // For the new NS, we are now on the first attempt again.
                    self.state
                        .transition_pass_token(false, PassTokenAttempt::First);
                }
            }
            // Immediately evaluate PassToken state because the bus is free for immediate
            // transmission
            return self.do_pass_token(now, phy);
        }

        let mut first_in = true;
        phy.receive_all_telegrams(now, |telegram, is_last_telegram| {
            self.mark_rx(now);

            // Only check and transition to ActiveIdle on the first telegram.
            if first_in {
                if telegram.source_address() != Some(self.token_ring.next_station()) {
                    log::warn!(
                        "Unexpected station #{} transmitting after token pass to #{}",
                        telegram.source_address().unwrap(),
                        self.token_ring.next_station()
                    );
                }

                // In case this was a telegram to us, we must already handle it in ActiveIdle state
                self.state.transition_active_idle();

                first_in = false;
            }
            self.handle_telegram(now, telegram, is_last_telegram)
        })
        .unwrap_or(PollDone::waiting_for_bus())
    }

    /// Poll the bus with a single active application.
    ///
    /// Poll must always be called with the same application.  The application may only be switched
    /// when the FdlActiveStation is currently offline.
    pub fn poll<PHY: ProfibusPhy>(
        &mut self,
        now: crate::time::Instant,
        phy: &mut PHY,
        app: &mut dyn FdlApplication,
    ) {
        let _result = self.poll_inner(now, phy, &mut [app]);
    }

    /// Poll the bus with multiple active applications.
    ///
    /// With multiple applications a simple round-robin scheduling is performed.  Each application
    /// gets to transmit all telegrams of its cycle before the next application is activated.
    /// This requires cooperations of the applications.  They must report completion of their cycle
    /// (by refusing to transmit another telegram once) after all work for one iteration has
    /// completed.
    ///
    /// When all applications complete their cycle once before the token hold time expires, the token is
    /// forwarded early.  Otherwise, the token is forwarded in between application telegrams and
    /// the applications get to continue their cycles once the token is received again.
    ///
    /// **Warning**: The list of applications must not change unless the FdlActiveStation is
    /// currently offline.  Changing the list may lead to unexpected behavior of applications or
    /// panics.
    pub fn poll_multi<PHY: ProfibusPhy>(
        &mut self,
        now: crate::time::Instant,
        phy: &mut PHY,
        apps: &mut [&mut dyn FdlApplication],
    ) {
        let _result = self.poll_inner(now, phy, apps);
    }

    fn poll_inner<PHY: ProfibusPhy>(
        &mut self,
        now: crate::time::Instant,
        phy: &mut PHY,
        apps: &mut [&mut dyn FdlApplication],
    ) -> PollDone {
        // Handle connectivity_state changes
        match self.connectivity_state {
            ConnectivityState::Offline => {
                debug_assert!(matches!(self.state, State::Offline));
                // When we are offline, don't do anything at all.
                return PollDone::offline().into();
            }
            ConnectivityState::Passive => {
                // TODO: Check if these are all the states from which we can transition to passive
                // idle
                match &self.state {
                    State::ActiveIdle { .. } | State::ListenToken { .. } | State::Offline => {
                        self.state.transition_passive_idle();
                    }
                    State::PassiveIdle => (),
                    s => {
                        log::debug!("Can't transition from \"{s:?}\" to PassiveIdle");
                    }
                }
            }
            ConnectivityState::Online => {
                if matches!(self.state, State::Offline | State::PassiveIdle) {
                    self.state.transition_listen_token();
                }
            }
        }

        // When a transmission is ongoing, we cannot do anything else in the meantime.  Thus,
        // return immediately in this case.
        return_if_done!(self.check_for_ongoing_transmision(now, phy));

        // Important: We may receive more data later during processing so we need to keep in mind
        // that the activity marker might change again later during the poll cycle.
        self.check_for_bus_activity(now, phy);

        match &self.state {
            State::Offline { .. } => unreachable!(),
            State::ListenToken { .. } => self.do_listen_token(now, phy).into(),
            State::ClaimToken { .. } => self.do_claim_token(now, phy).into(),
            State::UseToken { .. } => self.do_use_token(now, phy, apps).into(),
            State::AwaitDataResponse { .. } => self.do_await_data_response(now, phy, apps).into(),
            State::PassToken { .. } => self.do_pass_token(now, phy).into(),
            State::CheckTokenPass { .. } => self.do_check_token_pass(now, phy).into(),
            State::ActiveIdle { .. } => self.do_active_idle(now, phy).into(),
            State::AwaitStatusResponse { .. } => self.do_await_status_response(now, phy).into(),
            s => todo!("Active station state {s:?} not implemented yet!"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ensure the `FdlActiveStation` struct size doesn't completely get out of control.
    #[test]
    fn fdl_active_station_struct_size() {
        let size = std::mem::size_of::<FdlActiveStation>();
        println!("FDL active station struct is {size} bytes large.");
        assert!(size <= 256);
    }

    #[test]
    fn fdl_active_station_smoke() {
        crate::test_utils::prepare_test_logger();

        let mut phy = crate::phy::SimulatorPhy::new(crate::Baudrate::B19200, "phy");
        let mut fdl = FdlActiveStation::new(Default::default());

        crate::test_utils::set_active_addr(fdl.parameters().address);

        fdl.set_online();
        fdl.set_offline();
        fdl.set_online();

        let mut now = crate::time::Instant::ZERO;
        while now.total_millis() < 200 {
            fdl.poll(now, &mut phy, &mut ());

            now += crate::time::Duration::from_micros(100);
            phy.set_bus_time(now);
            crate::test_utils::set_log_timestamp(now);
        }
    }
}
