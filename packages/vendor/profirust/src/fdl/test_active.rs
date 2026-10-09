use crate::fdl;
use crate::phy;
use crate::phy::ProfibusPhy;

struct FdlActiveUnderTest {
    control_addr: u8,
    timestep: crate::time::Duration,
    pub phy_control: phy::SimulatorPhy,
    phy_active: phy::SimulatorPhy,
    pub active_station: fdl::FdlActiveStation,
}

impl Default for FdlActiveUnderTest {
    fn default() -> Self {
        Self::new(7)
    }
}

impl FdlActiveUnderTest {
    pub fn new(addr: crate::Address) -> Self {
        let baud = crate::Baudrate::B19200;
        let control_addr = 15;
        let timestep = crate::time::Duration::from_micros(100);

        let phy_control = phy::SimulatorPhy::new(baud, "phy#control");
        let phy_active = phy_control.duplicate("phy#ut");

        let mut active_station = fdl::FdlActiveStation::new(
            crate::fdl::ParametersBuilder::new(addr, baud)
                .highest_station_address(16)
                .slot_bits(300)
                .build(),
        );

        crate::test_utils::set_active_addr(active_station.parameters().address);
        active_station.set_online();

        Self {
            control_addr,
            timestep,
            phy_control,
            phy_active,
            active_station,
        }
    }

    pub fn now(&self) -> crate::time::Instant {
        self.phy_control.bus_time()
    }

    pub fn fdl_param(&self) -> &fdl::Parameters {
        self.active_station.parameters()
    }

    pub fn do_fdl_active_station_cycle(&mut self) {
        crate::test_utils::set_active_addr(self.active_station.parameters().address);
        self.active_station
            .poll(self.phy_control.bus_time(), &mut self.phy_active, &mut ());
        crate::test_utils::set_active_addr(self.control_addr);
    }

    pub fn do_timestep(&mut self) {
        self.phy_control.advance_bus_time(self.timestep);
        crate::test_utils::set_log_timestamp(self.phy_control.bus_time());
        self.do_fdl_active_station_cycle();
    }

    pub fn wait_for_matching<F: FnMut(fdl::Telegram) -> bool>(
        &mut self,
        f: F,
    ) -> crate::time::Duration {
        let start = self.phy_control.bus_time();
        crate::test_utils::set_active_addr(self.control_addr);
        for now in self.phy_control.iter_until_matching(self.timestep, f) {
            crate::test_utils::set_log_timestamp(now);
            crate::test_utils::set_active_addr(self.active_station.parameters().address);
            self.active_station.poll(now, &mut self.phy_active, &mut ());
            crate::test_utils::set_active_addr(self.control_addr);
        }
        self.phy_control.bus_time() - start
    }

    pub fn wait_next_telegram<R: Default, F: FnOnce(fdl::Telegram) -> R>(
        &mut self,
        f: F,
    ) -> (crate::time::Duration, R) {
        let start = self.phy_control.bus_time();
        crate::test_utils::set_active_addr(self.control_addr);
        let mut res = Default::default();
        let mut f = Some(f);
        for now in self.phy_control.iter_until_matching(self.timestep, |t| {
            res = (f.take().unwrap())(t);
            true
        }) {
            crate::test_utils::set_log_timestamp(now);
            crate::test_utils::set_active_addr(self.active_station.parameters().address);
            self.active_station.poll(now, &mut self.phy_active, &mut ());
            crate::test_utils::set_active_addr(self.control_addr);
        }
        (self.phy_control.bus_time() - start, res)
    }

    #[track_caller]
    pub fn assert_next_telegram(&mut self, expected: fdl::Telegram) -> crate::time::Duration {
        let mut pdu = [0u8; 256];
        let (time, t) = self.wait_next_telegram(|t| Some(t.clone_with_pdu_buffer(&mut pdu)));
        assert_eq!(t, Some(expected));
        time
    }

    pub fn advance_bus_time_min_tsdr(&mut self) {
        self.phy_control.advance_bus_time_min_tsdr();
        self.do_fdl_active_station_cycle();
    }

    pub fn advance_bus_time_sync_pause(&mut self) {
        self.advance_bus_time_bits(33);
        self.do_fdl_active_station_cycle();
    }

    pub fn advance_bus_time_bits(&mut self, bits: u32) {
        self.phy_control.advance_bus_time(self.bits_to_time(bits));
    }

    pub fn bits_to_time(&self, bits: u32) -> crate::time::Duration {
        self.active_station.parameters().bits_to_time(bits)
    }

    pub fn time_to_bits(&self, time: crate::time::Duration) -> u64 {
        self.active_station.parameters().baudrate.time_to_bits(time)
    }

    pub fn transmit_telegram<F>(&mut self, f: F) -> Option<fdl::TelegramTxResponse>
    where
        F: FnOnce(crate::fdl::TelegramTx) -> Option<fdl::TelegramTxResponse>,
    {
        let now = self.phy_control.bus_time();
        self.phy_control.transmit_telegram(now, f)
    }

    pub fn wait_transmission(&mut self) {
        while self
            .phy_control
            .poll_transmission(self.phy_control.bus_time())
        {
            self.do_timestep();
        }
    }

    pub fn assert_idle_time(&mut self, time: crate::time::Duration) {
        let timeout = self.phy_control.bus_time() + time;
        while self.phy_control.bus_time() < timeout {
            self.do_timestep();
            if self
                .phy_control
                .poll_pending_received_bytes(self.phy_control.bus_time())
                != 0
            {
                panic!("Idle time assertion failed!");
            }
        }
    }

    pub fn assert_idle_bits(&mut self, bits: u32) {
        self.assert_idle_time(self.bits_to_time(bits));
    }

    pub fn prepare_two_station_ring(&mut self) {
        self.advance_bus_time_sync_pause();
        self.transmit_telegram(|tx| Some(tx.send_token_telegram(15, 15)));
        self.wait_transmission();

        self.advance_bus_time_sync_pause();
        self.transmit_telegram(|tx| Some(tx.send_token_telegram(15, 15)));
        self.wait_transmission();

        self.advance_bus_time_sync_pause();
        self.transmit_telegram(|tx| Some(tx.send_token_telegram(15, 15)));
        self.wait_transmission();

        self.advance_bus_time_sync_pause();
        self.transmit_telegram(|tx| Some(tx.send_fdl_status_request(7, 15)));
        self.wait_transmission();

        self.assert_next_telegram(fdl::Telegram::Data(fdl::DataTelegram {
            h: fdl::DataTelegramHeader {
                da: 15,
                sa: 7,
                dsap: None,
                ssap: None,
                fc: fdl::FunctionCode::Response {
                    state: fdl::ResponseState::MasterWithoutToken,
                    status: fdl::ResponseStatus::Ok,
                },
            },
            pdu: &[],
        }));

        self.advance_bus_time_sync_pause();
        self.transmit_telegram(|tx| Some(tx.send_token_telegram(7, 15)));
        self.wait_transmission();
    }
}

/// Test that an active station sends a claimed token twice before doing anything else.
#[test]
fn new_token_is_sent_twice() {
    crate::test_utils::prepare_test_logger();
    let mut fdl_ut = FdlActiveUnderTest::default();

    let addr = fdl_ut.fdl_param().address;
    fdl_ut.wait_for_matching(|t| {
        t == fdl::Telegram::Token(fdl::TokenTelegram { da: addr, sa: addr })
    });

    let mut got_second_token = false;
    fdl_ut.wait_for_matching(|t| {
        if !got_second_token {
            if t == fdl::Telegram::Token(fdl::TokenTelegram { da: addr, sa: addr }) {
                got_second_token = true;
            } else {
                panic!("Got an unexpected telegram instead of second token: {t:?}");
            }
            false
        } else {
            true
        }
    });
}

/// Test that the active station waits for the appropriate amount of time before claiming the token
/// for itself on an idle bus.
#[rstest::rstest]
fn token_timeout(#[values(0, 1, 2, 7, 14)] addr: crate::Address) {
    crate::test_utils::prepare_test_logger();
    let mut fdl_ut = FdlActiveUnderTest::new(addr);

    fdl_ut.transmit_telegram(|tx| Some(tx.send_token_telegram(15, 15)));

    let start = fdl_ut.now();

    fdl_ut.wait_for_matching(|t| {
        t == fdl::Telegram::Token(fdl::TokenTelegram { da: addr, sa: addr })
    });

    let token_telegram_time = fdl_ut.bits_to_time(3 * 11);
    let timeout_measured = fdl_ut.now() - start - token_telegram_time;

    let expected_timeout =
        fdl_ut.bits_to_time(u32::from(fdl_ut.fdl_param().slot_bits) * (6 + 2 * u32::from(addr)));

    // Ensure the measured timeout also lies well before the timeout of the next address would
    // be reached.
    let expected_timeout_max = fdl_ut
        .bits_to_time(u32::from(fdl_ut.fdl_param().slot_bits) * (6 + 2 * u32::from(addr + 1)));

    log::info!(
        "Measured token timeout: {}us",
        timeout_measured.total_micros()
    );
    log::info!(
        "Expected token timeout: {}us < t < {}us",
        expected_timeout.total_micros(),
        expected_timeout_max.total_micros()
    );

    assert!(timeout_measured >= expected_timeout);
    assert!(timeout_measured <= expected_timeout_max);
}

/// Test active station FDL status response before initialization
#[test]
fn early_fdl_status() {
    crate::test_utils::prepare_test_logger();
    let mut fdl_ut = FdlActiveUnderTest::default();
    let addr = fdl_ut.fdl_param().address;

    fdl_ut.transmit_telegram(|tx| Some(tx.send_fdl_status_request(addr, 15)));

    fdl_ut.assert_next_telegram(fdl::Telegram::Data(fdl::DataTelegram {
        h: fdl::DataTelegramHeader {
            da: 15,
            sa: addr,
            dsap: None,
            ssap: None,
            fc: fdl::FunctionCode::Response {
                state: fdl::ResponseState::MasterNotReady,
                status: fdl::ResponseStatus::Ok,
            },
        },
        pdu: &[],
    }));
}

/// Test that an active station does not respond to a token telegram in ListenToken state
#[test]
fn ignore_telegrams_listen_token() {
    crate::test_utils::prepare_test_logger();
    let mut fdl_ut = FdlActiveUnderTest::default();
    let addr = fdl_ut.fdl_param().address;

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_token_telegram(addr, 15)));

    let time = fdl_ut.assert_next_telegram(fdl::Telegram::Token(fdl::TokenTelegram {
        da: addr,
        sa: addr,
    }));

    assert!(time > fdl_ut.fdl_param().token_lost_timeout());
}

/// Test that an active station correctly discovers an address collision in an active ring
#[test]
fn address_collision_ring() {
    crate::test_utils::prepare_test_logger_with_warnings(vec![
        "Witnessed collision of another active station with own address (#7)!",
        "Witnessed second collision of another active station with own address (#7), going offline.",
    ]);
    let mut fdl_ut = FdlActiveUnderTest::default();
    let addr = fdl_ut.fdl_param().address;

    let ring_members = [2, 4, 7, 12, 15];
    assert!(ring_members.contains(&addr));

    for _ in 0..2 {
        for addresses in ring_members.windows(2) {
            let prev = addresses[0];
            let next = addresses[1];
            fdl_ut.advance_bus_time_sync_pause();
            fdl_ut.transmit_telegram(|tx| Some(tx.send_token_telegram(next, prev)));
            fdl_ut.wait_transmission();
        }
        // Wrap around
        fdl_ut.advance_bus_time_sync_pause();
        fdl_ut.transmit_telegram(|tx| {
            Some(tx.send_token_telegram(
                *ring_members.first().unwrap(),
                *ring_members.last().unwrap(),
            ))
        });
        fdl_ut.wait_transmission();
    }

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_fdl_status_request(addr, 4)));
    fdl_ut.wait_transmission();

    fdl_ut.advance_bus_time_sync_pause();

    // After witnessing its own address on the bus twice, the active station must go offline.
    assert_eq!(
        fdl_ut.active_station.connectivity_state(),
        fdl::active::ConnectivityState::Offline
    );
}

/// Test that an active station correctly notices an address collision in the ListenToken state.
#[test]
fn address_collision_in_listen_token() {
    crate::test_utils::prepare_test_logger_with_warnings(vec![
        "Witnessed collision of another active station with own address (#7)!",
        "Witnessed second collision of another active station with own address (#7), going offline."
    ]);
    let mut fdl_ut = FdlActiveUnderTest::default();
    let addr = fdl_ut.fdl_param().address;

    fdl_ut.transmit_telegram(|tx| Some(tx.send_token_telegram(15, addr)));
    fdl_ut.wait_transmission();
    fdl_ut.advance_bus_time_sync_pause();

    // Afer the first collision, the station should still be going.
    assert_eq!(
        fdl_ut.active_station.connectivity_state(),
        crate::fdl::active::ConnectivityState::Online
    );

    fdl_ut.transmit_telegram(|tx| Some(tx.send_token_telegram(15, addr)));
    fdl_ut.wait_transmission();
    fdl_ut.advance_bus_time_sync_pause();

    // Afer the second collision, the station should now be offline.
    assert_eq!(
        fdl_ut.active_station.connectivity_state(),
        crate::fdl::active::ConnectivityState::Offline
    );
}

/// Test that an active station correctly notices an address collision in the ActiveIdle state.
#[test]
fn address_collision_in_active_idle() {
    crate::test_utils::prepare_test_logger_with_warnings(vec![
        "Unexpected station #7 transmitting after token pass to #15",
        "Witnessed collision of another active station with own address (#7)!",
        "Witnessed second collision of another active station with own address (#7), leaving ring.",
        "Witnessed second collision of another active station with own address (#7), going offline.",
    ]);
    let mut fdl_ut = FdlActiveUnderTest::default();
    let addr = fdl_ut.fdl_param().address;

    fdl_ut.prepare_two_station_ring();

    fdl_ut.wait_for_matching(|t| t == fdl::Telegram::Token(fdl::TokenTelegram { da: 15, sa: 7 }));

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_token_telegram(9, 7)));
    fdl_ut.wait_transmission();

    fdl_ut.advance_bus_time_sync_pause();

    // Afer the first collision, the station should still be going.
    assert!(fdl_ut.active_station.is_in_ring());

    fdl_ut.transmit_telegram(|tx| Some(tx.send_token_telegram(9, 7)));
    fdl_ut.wait_transmission();
    fdl_ut.advance_bus_time_sync_pause();

    // Afer the second collision, the station should now no longer be part of the token ring.
    assert!(!fdl_ut.active_station.is_in_ring());

    fdl_ut.transmit_telegram(|tx| Some(tx.send_token_telegram(9, 7)));
    fdl_ut.wait_transmission();
    fdl_ut.advance_bus_time_sync_pause();

    fdl_ut.transmit_telegram(|tx| Some(tx.send_token_telegram(9, 7)));
    fdl_ut.wait_transmission();
    fdl_ut.advance_bus_time_sync_pause();

    // Afer two more collisions in ListenToken state, the station should now be offline.
    assert_eq!(
        fdl_ut.active_station.connectivity_state(),
        crate::fdl::active::ConnectivityState::Offline
    );
}

/// Test that an active station waits at least two full token rotations before reporting to be
/// ready for entering the ring.
#[test]
fn two_rotations_before_ready() {
    crate::test_utils::prepare_test_logger();
    let mut fdl_ut = FdlActiveUnderTest::new(7);

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_token_telegram(2, 15)));
    fdl_ut.wait_transmission();

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_token_telegram(4, 2)));
    fdl_ut.wait_transmission();

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_fdl_status_request(7, 4)));
    fdl_ut.wait_transmission();

    // Master must answer at this point that it is not yet ready
    fdl_ut.assert_next_telegram(fdl::Telegram::Data(fdl::DataTelegram {
        h: fdl::DataTelegramHeader {
            da: 4,
            sa: 7,
            dsap: None,
            ssap: None,
            fc: fdl::FunctionCode::Response {
                state: fdl::ResponseState::MasterNotReady,
                status: fdl::ResponseStatus::Ok,
            },
        },
        pdu: &[],
    }));

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_token_telegram(15, 4)));
    fdl_ut.wait_transmission();

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_token_telegram(2, 15)));
    fdl_ut.wait_transmission();

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_token_telegram(4, 2)));
    fdl_ut.wait_transmission();

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_fdl_status_request(7, 4)));
    fdl_ut.wait_transmission();

    // Master must still answer at this point that it is not yet ready
    fdl_ut.assert_next_telegram(fdl::Telegram::Data(fdl::DataTelegram {
        h: fdl::DataTelegramHeader {
            da: 4,
            sa: 7,
            dsap: None,
            ssap: None,
            fc: fdl::FunctionCode::Response {
                state: fdl::ResponseState::MasterNotReady,
                status: fdl::ResponseStatus::Ok,
            },
        },
        pdu: &[],
    }));

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_token_telegram(15, 4)));
    fdl_ut.wait_transmission();

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_token_telegram(2, 15)));
    fdl_ut.wait_transmission();

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_token_telegram(4, 2)));
    fdl_ut.wait_transmission();

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_fdl_status_request(7, 4)));
    fdl_ut.wait_transmission();

    // Now the master shall respond that it is ready to receive a token
    fdl_ut.assert_next_telegram(fdl::Telegram::Data(fdl::DataTelegram {
        h: fdl::DataTelegramHeader {
            da: 4,
            sa: 7,
            dsap: None,
            ssap: None,
            fc: fdl::FunctionCode::Response {
                state: fdl::ResponseState::MasterWithoutToken,
                status: fdl::ResponseStatus::Ok,
            },
        },
        pdu: &[],
    }));

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_token_telegram(7, 4)));

    fdl_ut.wait_for_matching(|t| t == fdl::Telegram::Token(fdl::TokenTelegram { da: 15, sa: 7 }));
}

/// Test that an active station discovers another active neighbor station.
#[test]
fn active_station_discovers_neighbor() {
    crate::test_utils::prepare_test_logger();
    let mut fdl_ut = FdlActiveUnderTest::new(7);

    fdl_ut.wait_for_matching(|t| {
        if let fdl::Telegram::Data(data_telegram) = t {
            data_telegram.is_fdl_status_request().is_some()
                && data_telegram.h.da == 4
                && data_telegram.h.sa == 7
        } else {
            false
        }
    });

    fdl_ut.advance_bus_time_min_tsdr();
    fdl_ut.transmit_telegram(|tx| {
        Some(tx.send_fdl_status_response(
            7,
            4,
            fdl::ResponseState::MasterWithoutToken,
            fdl::ResponseStatus::Ok,
        ))
    });
    fdl_ut.wait_transmission();

    fdl_ut.wait_for_matching(|t| t == fdl::Telegram::Token(fdl::TokenTelegram { da: 4, sa: 7 }));
}

/// Test that an active station resends the token when not received by next station
#[test]
fn active_station_resends_token() {
    crate::test_utils::prepare_test_logger_with_warnings(vec![
        "Token was apparently not received by #15, resending...",
        "Token was again not received by #15, resending...",
    ]);
    let mut fdl_ut = FdlActiveUnderTest::new(7);

    fdl_ut.prepare_two_station_ring();

    fdl_ut.wait_for_matching(|t| t == fdl::Telegram::Token(fdl::TokenTelegram { da: 15, sa: 7 }));

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_fdl_status_request(8, 15)));
    fdl_ut.wait_transmission();

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_token_telegram(7, 15)));
    fdl_ut.wait_transmission();

    fdl_ut.wait_for_matching(|t| t == fdl::Telegram::Token(fdl::TokenTelegram { da: 15, sa: 7 }));

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_fdl_status_request(9, 15)));
    fdl_ut.wait_transmission();

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_token_telegram(7, 15)));
    fdl_ut.wait_transmission();

    fdl_ut.wait_for_matching(|t| t == fdl::Telegram::Token(fdl::TokenTelegram { da: 15, sa: 7 }));

    // This time the next station doesn't do anything, so the previous one must resend the token.

    let wait_time = fdl_ut
        .wait_for_matching(|t| t == fdl::Telegram::Token(fdl::TokenTelegram { da: 15, sa: 7 }));
    assert!(wait_time <= fdl_ut.fdl_param().slot_time() * 2);

    // And the station must also do a second retry.

    let wait_time = fdl_ut
        .wait_for_matching(|t| t == fdl::Telegram::Token(fdl::TokenTelegram { da: 15, sa: 7 }));
    assert!(wait_time <= fdl_ut.fdl_param().slot_time() * 2);

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_fdl_status_request(9, 15)));
    fdl_ut.wait_transmission();

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_token_telegram(7, 15)));
    fdl_ut.wait_transmission();

    fdl_ut.wait_for_matching(|t| t == fdl::Telegram::Token(fdl::TokenTelegram { da: 15, sa: 7 }));
}

/// Test that an active station forwards the token to the next station when when the previous next
/// station vanishes.
#[test]
fn active_station_next_vanishes() {
    crate::test_utils::prepare_test_logger_with_warnings(vec![
        "Token was apparently not received by #15, resending...",
        "Token was again not received by #15, resending...",
        "Token was also not received on third attempt, clearing #15 from LAS.",
    ]);
    let mut fdl_ut = FdlActiveUnderTest::new(7);

    fdl_ut.prepare_two_station_ring();

    fdl_ut.wait_for_matching(|t| t == fdl::Telegram::Token(fdl::TokenTelegram { da: 15, sa: 7 }));

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_fdl_status_request(8, 15)));
    fdl_ut.wait_transmission();

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_token_telegram(7, 15)));
    fdl_ut.wait_transmission();

    fdl_ut.wait_for_matching(|t| t == fdl::Telegram::Token(fdl::TokenTelegram { da: 15, sa: 7 }));

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_fdl_status_request(9, 15)));
    fdl_ut.wait_transmission();

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_token_telegram(7, 15)));
    fdl_ut.wait_transmission();

    fdl_ut.wait_for_matching(|t| t == fdl::Telegram::Token(fdl::TokenTelegram { da: 15, sa: 7 }));

    // This time the next station doesn't do anything, so the previous one must resend the token.

    let wait_time = fdl_ut
        .wait_for_matching(|t| t == fdl::Telegram::Token(fdl::TokenTelegram { da: 15, sa: 7 }));
    assert!(wait_time <= fdl_ut.fdl_param().slot_time() * 2);

    // Second retry attempt.

    let wait_time = fdl_ut
        .wait_for_matching(|t| t == fdl::Telegram::Token(fdl::TokenTelegram { da: 15, sa: 7 }));
    assert!(wait_time <= fdl_ut.fdl_param().slot_time() * 2);

    // And now station #15 is kicked off the ring, leaving #7 alone.

    let wait_time = fdl_ut
        .wait_for_matching(|t| t == fdl::Telegram::Token(fdl::TokenTelegram { da: 7, sa: 7 }));
    assert!(wait_time <= fdl_ut.fdl_param().slot_time() * 2);
}

/// Test that an active station is able to respond to telegrams immediately after it passed the
/// token
#[test]
fn active_station_replies_after_token_pass() {
    crate::test_utils::prepare_test_logger();
    let mut fdl_ut = FdlActiveUnderTest::new(7);

    fdl_ut.prepare_two_station_ring();

    fdl_ut.wait_for_matching(|t| t == fdl::Telegram::Token(fdl::TokenTelegram { da: 15, sa: 7 }));

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_token_telegram(7, 15)));
    fdl_ut.wait_transmission();

    fdl_ut.assert_next_telegram(fdl::Telegram::Data(fdl::DataTelegram {
        h: fdl::DataTelegramHeader {
            da: 9,
            sa: 7,
            dsap: None,
            ssap: None,
            fc: fdl::FunctionCode::Request {
                fcb: fdl::FrameCountBit::Inactive,
                req: fdl::RequestType::FdlStatus,
            },
        },
        pdu: &[],
    }));
}

/// Test that an active station responds to unknown requests.
#[ignore = "not yet implemented"]
#[test]
fn active_station_responds_unknown() {
    crate::test_utils::prepare_test_logger();
    let mut fdl_ut = FdlActiveUnderTest::default();

    fdl_ut.prepare_two_station_ring();

    fdl_ut.wait_for_matching(|t| t == fdl::Telegram::Token(fdl::TokenTelegram { da: 15, sa: 7 }));

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| {
        Some(tx.send_data_telegram(
            fdl::DataTelegramHeader {
                da: 7,
                sa: 15,
                dsap: crate::consts::SAP_SLAVE_DIAGNOSIS,
                ssap: crate::consts::SAP_MASTER_MS0,
                fc: crate::fdl::FunctionCode::new_srd_low(Default::default()),
            },
            0,
            |_buf| (),
        ))
    });
    fdl_ut.wait_transmission();

    fdl_ut.assert_next_telegram(fdl::Telegram::Data(fdl::DataTelegram {
        h: fdl::DataTelegramHeader {
            da: 15,
            sa: 7,
            dsap: crate::consts::SAP_MASTER_MS0,
            ssap: crate::consts::SAP_SLAVE_DIAGNOSIS,
            fc: fdl::FunctionCode::Response {
                state: fdl::ResponseState::MasterInRing,
                status: fdl::ResponseStatus::SapNotEnabled,
            },
        },
        pdu: &[],
    }));
}

/// Test that a token lost timeout is triggered in the active idle state as well
#[test]
fn active_idle_token_lost() {
    crate::test_utils::prepare_test_logger_with_warnings(vec!["Token lost! Generating a new one."]);
    let mut fdl_ut = FdlActiveUnderTest::default();

    fdl_ut.prepare_two_station_ring();

    fdl_ut.wait_for_matching(|t| t == fdl::Telegram::Token(fdl::TokenTelegram { da: 15, sa: 7 }));

    // We must send one more telegram so the station under test can happily exit the CheckTokenPass
    // state
    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_fdl_status_request(3, 15)));
    fdl_ut.wait_transmission();

    let time =
        fdl_ut.assert_next_telegram(fdl::Telegram::Token(fdl::TokenTelegram { da: 7, sa: 7 }));

    assert!(time > fdl_ut.fdl_param().token_lost_timeout());
}

/// Test that the active station correctly rejects a single token from a new previous station
#[test]
fn active_station_rejects_new_previous_neighbor() {
    crate::test_utils::prepare_test_logger_with_warnings(vec!["Token lost! Generating a new one."]);
    let mut fdl_ut = FdlActiveUnderTest::default();

    fdl_ut.prepare_two_station_ring();

    fdl_ut.wait_for_matching(|t| t == fdl::Telegram::Token(fdl::TokenTelegram { da: 15, sa: 7 }));

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_token_telegram(42, 15)));
    fdl_ut.wait_transmission();

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_token_telegram(7, 42)));
    fdl_ut.wait_transmission();

    // Active station must not accept this token, so the next thing we see is the token lost
    // timeout.

    let time =
        fdl_ut.assert_next_telegram(fdl::Telegram::Token(fdl::TokenTelegram { da: 7, sa: 7 }));

    assert!(time > fdl_ut.fdl_param().token_lost_timeout());
}

/// Test that the active station correctly accepts a token from a new previous station
#[test]
fn active_station_accepts_new_previous_neighbor() {
    crate::test_utils::prepare_test_logger();
    let mut fdl_ut = FdlActiveUnderTest::default();

    fdl_ut.prepare_two_station_ring();

    fdl_ut.wait_for_matching(|t| t == fdl::Telegram::Token(fdl::TokenTelegram { da: 15, sa: 7 }));

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_token_telegram(42, 15)));
    fdl_ut.wait_transmission();

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_token_telegram(7, 42)));
    fdl_ut.wait_transmission();

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_token_telegram(7, 42)));
    fdl_ut.wait_transmission();

    // Active station must not accept this token, so the next thing we see is the token lost
    // timeout.

    let wait_time = fdl_ut
        .wait_for_matching(|t| t == fdl::Telegram::Token(fdl::TokenTelegram { da: 15, sa: 7 }));
    assert!(wait_time <= fdl_ut.fdl_param().slot_time() * 2);
}

/// Test that the active station correctly goes offline when requested to
#[test]
fn going_offline() {
    crate::test_utils::prepare_test_logger();
    let mut fdl_ut = FdlActiveUnderTest::default();

    fdl_ut.prepare_two_station_ring();

    fdl_ut.wait_for_matching(|t| t == fdl::Telegram::Token(fdl::TokenTelegram { da: 15, sa: 7 }));

    fdl_ut.active_station.set_offline();
    assert_eq!(
        fdl_ut.active_station.connectivity_state(),
        fdl::ConnectivityState::Offline
    );

    for _ in 0..100 {
        fdl_ut.do_timestep();
    }

    assert_eq!(
        fdl_ut.active_station.connectivity_state(),
        fdl::ConnectivityState::Offline
    );
}

#[test]
fn multimaster_smoke() {
    crate::test_utils::prepare_test_logger_with_warnings(vec![
        "Token was apparently not received by #2, resending...",
    ]);
    let baud = crate::Baudrate::B19200;

    let actives_addr = vec![2, 7, 13, 24];

    let phy = crate::phy::SimulatorPhy::new(baud, "phy#main");

    let mut actives: Vec<_> = actives_addr
        .iter()
        .copied()
        .map(|addr| {
            let phy = phy.duplicate(format!("phy#{addr}").leak());
            let mut fdl = crate::fdl::FdlActiveStation::new(
                crate::fdl::ParametersBuilder::new(addr, baud)
                    .highest_station_address(30)
                    .slot_bits(300)
                    .gap_wait_rotations(30)
                    .build(),
            );
            crate::test_utils::set_active_addr(addr);
            fdl.set_online();
            (addr, phy, fdl)
        })
        .collect();

    let start = crate::time::Instant::ZERO;
    let mut now = start;
    while (now - start) < crate::time::Duration::from_millis(3200) {
        crate::test_utils::set_log_timestamp(now);
        phy.set_bus_time(now);

        for (addr, phy, fdl) in actives.iter_mut() {
            crate::test_utils::set_active_addr(*addr);
            fdl.poll(now, phy, &mut ());
        }

        now += crate::time::Duration::from_micros(100);
    }

    for (addr, _, fdl) in actives.iter() {
        assert!(
            fdl.is_in_ring(),
            "station #{addr} is not in the token ring!"
        );

        let known_actives: Vec<_> = fdl.inspect_token_ring().iter_active_stations().collect();
        assert_eq!(
            known_actives, actives_addr,
            "wrong LAS formed in station #{addr}"
        );
    }
}

#[test]
fn active_station_receives_faulty_token_telegram() {
    crate::test_utils::prepare_test_logger_with_warnings(vec![
        "Unexpected station #174 transmitting after token pass to #15",
        "Witnessed token pass from invalid address #174->#223, ignoring.",
    ]);
    let mut fdl_ut = FdlActiveUnderTest::default();

    fdl_ut.prepare_two_station_ring();

    fdl_ut.wait_for_matching(|t| t == fdl::Telegram::Token(fdl::TokenTelegram { da: 15, sa: 7 }));

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_token_telegram(223, 174)));
    fdl_ut.wait_transmission();

    fdl_ut.advance_bus_time_sync_pause();
    fdl_ut.transmit_telegram(|tx| Some(tx.send_token_telegram(7, 15)));
    fdl_ut.wait_transmission();

    fdl_ut.wait_for_matching(|t| t == fdl::Telegram::Token(fdl::TokenTelegram { da: 15, sa: 7 }));
}

#[test]
fn slot_time_timing() {
    crate::test_utils::prepare_test_logger();
    let mut fdl_ut = FdlActiveUnderTest::default();
    let slot_bits = fdl_ut.fdl_param().slot_bits;
    let slot_time = fdl_ut.fdl_param().slot_time();

    log::debug!("slot_bits = {slot_bits}");
    log::debug!("slot_time = {slot_time}");

    fdl_ut.wait_for_matching(|t| {
        t == fdl::Telegram::Data(fdl::DataTelegram {
            h: fdl::DataTelegramHeader {
                da: 9,
                sa: 7,
                dsap: None,
                ssap: None,
                fc: fdl::FunctionCode::Request {
                    fcb: fdl::FrameCountBit::Inactive,
                    req: fdl::RequestType::FdlStatus,
                },
            },
            pdu: &[],
        })
    });

    log::debug!("After receiving request...");

    let time =
        fdl_ut.assert_next_telegram(fdl::Telegram::Token(fdl::TokenTelegram { da: 7, sa: 7 }));

    // We have to subtract the telegram runtime of the just received token telegram
    let time = time - fdl_ut.bits_to_time(33);

    let bits_over = fdl_ut.time_to_bits(time - slot_time);
    log::debug!("Slot time was {bits_over} bits over projected time.");

    assert!(
        time > slot_time,
        "Slot time was {time} instead of {slot_time}!"
    );
    assert!(
        time < (slot_time * 2),
        "Slot time was {time} instead of {slot_time} (that's {bits_over} too many T_bit)!"
    );
}
