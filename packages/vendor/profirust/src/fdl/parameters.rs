/// FDL active station parameters
///
/// These parameters configure the behavior of an FDL active station.
///
/// You should use the [`ParametersBuilder`] to build the parameters struct.  Check its
/// documentation for detailed explanations of the individual parameters.
///
/// # Example
/// ```
/// use profirust::fdl;
/// # let buffer: [profirust::dp::PeripheralStorage; 4] = Default::default();
/// # let dp_master = profirust::dp::DpMaster::new(buffer);
///
/// let master_address = 2;
/// let param = fdl::ParametersBuilder::new(master_address, profirust::Baudrate::B19200)
///     .slot_bits(300)
///     .build_verified(&dp_master);
/// ```
#[derive(Debug, PartialEq, Eq, Clone)]
#[non_exhaustive]
pub struct Parameters {
    /// Station address for this master/active station
    pub address: u8,
    /// Baudrate
    pub baudrate: crate::Baudrate,
    /// T<sub>SL</sub>: Slot time in bits
    pub slot_bits: u16,
    /// T<sub>TR</sub>: Target rotation time for the token
    pub token_rotation_bits: u32,
    /// GAP: update factor (how many token rotations to wait before polling the gap again)
    pub gap_wait_rotations: u8,
    /// HSA: Highest station address
    ///
    /// The HSA is only relevant for finding other active stations who want to become part of the
    /// token ring.  Passive stations are allows to have addresses higher than HSA.  Active
    /// stations must have an address less than HSA.
    pub highest_station_address: u8,
    /// Maximum number of retries when no answer was received
    pub max_retry_limit: u8,
    /// min T<sub>SDR</sub>: Minimum delay before anyone is allowed to respond to a telegram
    pub min_tsdr_bits: u8,
    /// Watchdog timeout for peripherals monitoring the DP master
    pub watchdog_factors: Option<(u8, u8)>,
}

impl Default for Parameters {
    fn default() -> Self {
        Parameters {
            address: 1,
            baudrate: crate::Baudrate::B19200,
            // Tied to the baudrate - will usually be adjusted by the ParametersBuilder.
            slot_bits: 100,
            // TTR default value as found elsewhere.
            // TODO: Need to decide how to deal with this value.
            token_rotation_bits: 32436,
            // GAP update factor, default 10 as found elsewhere.
            gap_wait_rotations: 10,
            // 125 is the highest possible address - by default all addresses are included.
            // (and HSA is highest address + 1)
            highest_station_address: 126,
            // Defaults to 1 byte time (= 11 bits)
            min_tsdr_bits: 11,
            // Retry limit defaults to 1, meaning that a telegram will be retried once.  This is a
            // sane default as retries should not be necessary at all on a bus that is set up
            // correctly.
            max_retry_limit: 1,
            // No watchdog by default.
            //
            // TODO: Is this what we want?  Found 6250 x HSA recommended elsewhere.
            watchdog_factors: None,
        }
    }
}

impl Parameters {
    #[inline(always)]
    pub fn debug_assert_consistency(&self) {
        crate::debug_assert_address(self.address);
        debug_assert!(self.highest_station_address <= 126);
    }
}

#[inline]
fn min_slot_bits(baudrate: crate::Baudrate) -> u16 {
    match baudrate {
        crate::Baudrate::B9600
        | crate::Baudrate::B19200
        | crate::Baudrate::B31250
        | crate::Baudrate::B45450
        | crate::Baudrate::B93750
        | crate::Baudrate::B187500 => 100,
        crate::Baudrate::B500000 => 200,
        crate::Baudrate::B1500000 => 300,
        crate::Baudrate::B3000000 => 400,
        crate::Baudrate::B6000000 => 600,
        crate::Baudrate::B12000000 => 1000,
    }
}

#[inline]
fn watchdog_factors(dur: crate::time::Duration) -> Option<Result<(u8, u8), ()>> {
    // TODO: Support the different watchdog time bases in some way?
    Some(dur)
        .filter(|dur| *dur != crate::time::Duration::ZERO)
        .map(|dur| {
            let timeout_10ms: u32 = (dur.total_millis() / 10).try_into().or(Err(()))?;

            for f1 in 1..256 {
                let f2 = (timeout_10ms + f1 - 1) / f1;

                if f2 < 256 {
                    return Ok((u8::try_from(f1).unwrap(), u8::try_from(f2).unwrap()));
                }
            }

            // Timeout is still too big
            Err(())
        })
}

/// Builder for the parameters of an FDL active station
pub struct ParametersBuilder(Parameters);

impl ParametersBuilder {
    /// Start building parameters for an FDL active station with the given `address`.
    ///
    /// - `address` must be a valid PROFIBUS address (<= 125).
    /// - `baudrate` is the baudrate the is used for this PROFIBUS network.
    #[inline]
    pub fn new(address: u8, baudrate: crate::Baudrate) -> Self {
        assert!(address <= 125);
        Self(Parameters {
            address,
            baudrate,
            slot_bits: min_slot_bits(baudrate),
            ..Default::default()
        })
    }

    /// Configure non-standard T<sub>SL</sub> (slot time in bits)
    ///
    /// The slot time must be larger than the maximum T<sub>SDR</sub> of all peripherals.
    /// `build_verified()` will check that this is the case.
    ///
    /// The slot time must be creater than the default slot time:
    ///
    /// | Baudrate | Minimum Slot Time (in Bits) |
    /// | ---: | ---: |
    /// | <=187500 | 100 |
    /// | 500000 | 200 |
    /// | 1500000 | 300 |
    /// | 3000000 | 400 |
    /// | 6000000 | 600 |
    /// | 12000000 | 1000 |
    #[inline]
    pub fn slot_bits(&mut self, slot_bits: u16) -> &mut Self {
        self.0.slot_bits = slot_bits;
        assert!(slot_bits >= min_slot_bits(self.0.baudrate));
        self
    }

    /// Set the highest projected (active) station address.
    ///
    /// The HSA is used when scanning for other FDL active station who want to participate on the bus.
    ///
    /// By default, all addresses are scanned (HSA = 126).  This means all active stations will be found
    /// but it also means that the time until an active station can join the token ring is rather long.  It
    /// is advisable to choose low addresses for all active stations and then set the HSA accordingly to
    /// optimize recovery time after a master drops from the bus.
    #[inline]
    pub fn highest_station_address(&mut self, hsa: u8) -> &mut Self {
        assert!(hsa > self.0.address && hsa <= 126);
        self.0.highest_station_address = hsa;
        // TODO: We probably shouldn't override an explicitly set value here...
        self.token_rotation_bits(u32::from(hsa) * 5000);
        self
    }

    /// Set the projected token rotation time (in bits).
    ///
    /// The TTR is used to ensure each FDL active station gets a chance to communicate with its peripherals
    /// in deterministic time.
    ///
    /// It is important that the TTR is not too small as this unnecessarily slows down
    /// communication.  Defaults to 32436.
    pub fn token_rotation_bits(&mut self, ttr: u32) -> &mut Self {
        assert!(ttr >= 256 && ttr <= 16_777_960);
        self.0.token_rotation_bits = ttr;
        self
    }

    /// Set how many token rotations to wait before restarting the GAP scan.
    ///
    /// The GAP scan is used to detect other FDL active stations who want to communicate on the bus.
    ///
    /// This factor is the wait time between scan cycles.  A low value means stations are found
    /// very quickly but the tradeoff is a higher average cycle time.
    pub fn gap_wait_rotations(&mut self, gap_wait: u8) -> &mut Self {
        assert!(gap_wait >= 1 && gap_wait <= 100);
        self.0.gap_wait_rotations = gap_wait;
        self
    }

    /// Set the maximum number of retries when communication with a peripheral fails.
    ///
    /// After this amount of retries, the peripheral is considered offline and will need to be
    /// reconfigured once it appears again.
    ///
    /// On a bus that is electrically sound, no retries should ever be necessary.  When you have to
    /// increase the retry count to keep your bus working, it is recommended to check for
    /// electrical and/or noise problems.
    ///
    /// Default value is 1, meaning a telegram is retried once when no response was received.
    #[inline]
    pub fn max_retry_limit(&mut self, max_retry_limit: u8) -> &mut Self {
        assert!(max_retry_limit >= 1 && max_retry_limit <= 15);
        self.0.max_retry_limit = max_retry_limit;
        self
    }

    /// Set the minimum response time that peripherals should adhere to.
    ///
    /// This value can be increased when peripherals responding after 11 bits is too fast for the
    /// bus to settle.
    #[inline]
    pub fn min_tsdr(&mut self, min_tsdr_bits: u8) -> &mut Self {
        assert!(min_tsdr_bits >= 11);
        self.0.min_tsdr_bits = min_tsdr_bits;
        self
    }

    /// Set the watchdog timeout that peripherals should use to fail-safe after loosing
    /// communication.
    #[inline]
    pub fn watchdog_timeout(&mut self, wdg: crate::time::Duration) -> &mut Self {
        assert!(wdg >= crate::time::Duration::from_millis(10));
        assert!(wdg <= crate::time::Duration::from_secs(650));
        self.0.watchdog_factors = watchdog_factors(wdg).transpose().unwrap();
        self
    }

    /// Build the parameters struct.
    #[inline]
    pub fn build(&self) -> Parameters {
        self.0.clone()
    }

    /// Build the parameters struct and verify it against the given DP master.
    ///
    /// This ensures that, for example, the selected T<sub>SL</sub> is greater than the max Tsdr of
    /// all peripherals currently tracked by the DP master.
    #[inline]
    pub fn build_verified(&self, dp_master: &crate::dp::DpMaster) -> Parameters {
        for (_, peripheral) in dp_master.iter() {
            assert!(
                peripheral.options().max_tsdr + 15 <= self.0.slot_bits,
                "max Tsdr of peripheral #{} too large for slot time",
                peripheral.address(),
            );
        }
        self.0.clone()
    }
}

impl Parameters {
    pub fn bits_to_time(&self, bits: u32) -> crate::time::Duration {
        self.baudrate.bits_to_time(bits)
    }

    /// T<sub>SL</sub> (slot time) converted to duration
    pub fn slot_time(&self) -> crate::time::Duration {
        self.bits_to_time(u32::from(self.slot_bits))
    }

    /// min T<sub>SDR</sub> (minimum time before responding) converted to duration
    pub fn min_tsdr_time(&self) -> crate::time::Duration {
        self.bits_to_time(u32::from(self.min_tsdr_bits))
    }

    /// Timeout after which the token is considered lost.
    ///
    /// Calculated as 6 * T<sub>SL</sub> + 2 * Addr * T<sub>SL</sub>.
    pub fn token_lost_timeout(&self) -> crate::time::Duration {
        let timeout_bits = u32::from(self.slot_bits) * (6 + 2 * u32::from(self.address));
        self.bits_to_time(timeout_bits)
    }

    /// T<sub>TR</sub> (projected token rotation time)
    pub fn token_rotation_time(&self) -> crate::time::Duration {
        self.bits_to_time(self.token_rotation_bits)
    }

    /// Watchdog timeout
    pub fn watchdog_timeout(&self) -> Option<crate::time::Duration> {
        self.watchdog_factors
            .map(|(f1, f2)| crate::time::Duration::from_millis(u64::from(f1) * u64::from(f2) * 10))
    }
}
