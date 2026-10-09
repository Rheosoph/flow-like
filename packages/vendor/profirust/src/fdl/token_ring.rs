/// Status of the `LAS` (List of Active Stations)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LasState {
    /// We are waiting for the first token rotation to then enter discovery
    Uninitialized,
    /// We are listening to the first token rotation to discover all active stations.
    Discovery,
    /// We are listening to the second token rotation to verify correctness of the LAS.
    Verification,
    /// The LAS has been verified and is valid.  It will be updated live from now on.
    Valid,
}

impl LasState {
    /// Whether the LAS is valid.
    ///
    /// Returns `true` when `self == LasState::Valid`
    #[inline(always)]
    pub fn is_valid(self) -> bool {
        matches!(self, Self::Valid)
    }
}

/// Management of the token ring from the station's point of view
#[derive(Clone, PartialEq, Eq)]
pub struct TokenRing {
    /// `LAS` (List of Active Stations)
    active_stations: bitvec::BitArr!(for 128),

    /// Status of the `LAS`
    las_state: LasState,

    /// `TS` (This Station)
    this_station: crate::Address,

    /// `NS` (Next Station)
    ///
    /// The `next_station` is who we will forward the token to once we release it.
    ///
    /// There is always a `next_station`.  When no other active stations are known, we are our own
    /// `next_station`, so NS==TS.
    next_station: crate::Address,

    /// `PS` (Previous Station)
    ///
    /// The `previous_station` is who we will receive the token from.  At first, only a token from
    /// `previous_station` is accepted unless a station makes it clear that it is our new
    /// `previous_station` by passing the token to us a second time.
    ///
    /// There is always a `previous_station`.  When no other active stations are known, we are our
    /// own `previous_station`, so PS==TS.
    previous_station: crate::Address,
}

impl TokenRing {
    pub fn new(param: &crate::fdl::Parameters) -> Self {
        let mut active_stations = bitvec::array::BitArray::ZERO;
        // Mark ourselves in the list of active stations.
        active_stations.set(usize::from(param.address), true);

        Self {
            active_stations,
            las_state: LasState::Uninitialized,
            this_station: param.address,
            next_station: param.address,
            previous_station: param.address,
        }
    }

    pub fn iter_active_stations(
        &self,
    ) -> impl Iterator<Item = crate::Address> + DoubleEndedIterator + '_ {
        self.active_stations
            .iter_ones()
            .map(|a| u8::try_from(a).unwrap())
    }

    pub fn ready_for_ring(&self) -> bool {
        self.las_state.is_valid()
    }

    pub fn this_station(&self) -> crate::Address {
        self.this_station
    }

    pub fn next_station(&self) -> crate::Address {
        self.next_station
    }

    pub fn previous_station(&self) -> crate::Address {
        self.previous_station
    }

    fn verify_las_from_token_pass(&mut self, sa: crate::Address, da: crate::Address) -> bool {
        // SA station must be active
        if !self.active_stations[usize::from(sa)] {
            return false;
        }

        // DA station must be active
        if !self.active_stations[usize::from(da)] {
            return false;
        }

        // No stations between SA and DA must be active
        if da > sa {
            if self.active_stations[usize::from(sa + 1)..usize::from(da)].any() {
                return false;
            }
        } else {
            // Handle wrap-around
            if self.active_stations[usize::from(sa + 1)..].any() {
                return false;
            }
            if self.active_stations[..usize::from(da)].any() {
                return false;
            }
        }

        true
    }

    fn update_las_from_token_pass(&mut self, sa: crate::Address, da: crate::Address) {
        // Clear the GAP from this token pass as it does not contain any known active stations.
        if da > sa {
            self.active_stations[usize::from(sa)..usize::from(da)].fill(false);
        } else {
            self.active_stations[usize::from(sa)..].fill(false);
            self.active_stations[..usize::from(da)].fill(false);
        }

        // At this point, we only know that the source address is alive so we only enter it into
        // the LAS.  The destination will be added later, when it actively forwards the token
        // itself.
        self.active_stations.set(usize::from(sa), true);

        self.update_next_previous();
    }

    fn update_next_previous(&mut self) {
        let next_station =
            if let Some(next) = self.iter_active_stations().find(|a| *a > self.this_station) {
                next
            } else if let Some(first) = self.iter_active_stations().next() {
                first
            } else {
                self.this_station
            };

        let previous_station = if let Some(previous) = self
            .iter_active_stations()
            .rev()
            .find(|a| *a < self.this_station)
        {
            previous
        } else if let Some(last) = self.iter_active_stations().rev().next() {
            last
        } else {
            self.this_station
        };

        if self.next_station != next_station {
            log::trace!("New NS is #{next_station}");
        }
        if self.previous_station != previous_station {
            log::trace!("New PS is #{previous_station}");
        }

        self.next_station = next_station;
        self.previous_station = previous_station;
    }

    pub fn witness_token_pass(&mut self, sa: crate::Address, da: crate::Address) {
        if sa > 125 {
            log::warn!("Witnessed token pass from invalid address #{sa}->#{da}, ignoring.");
            return;
        }
        if da > 125 {
            log::warn!("Witnessed token pass to invalid address #{da}<-#{sa}, ignoring.");
            return;
        }

        match self.las_state {
            // If we see the wrap-around, start discovery
            LasState::Uninitialized => {
                if da <= sa {
                    self.las_state = LasState::Discovery;
                    log::trace!("Starting discovery of active stations...");
                }
            }
            LasState::Discovery => {
                self.update_las_from_token_pass(sa, da);
                if da <= sa {
                    self.las_state = LasState::Verification;
                    log::trace!("Starting verification of active stations list...");
                }
            }
            LasState::Verification => {
                // If verification fails, restart discovery
                if !self.verify_las_from_token_pass(sa, da) {
                    self.update_las_from_token_pass(sa, da);
                    self.las_state = LasState::Discovery;
                    log::trace!("Rediscovering active stations due to a change...");
                } else if da <= sa {
                    self.las_state = LasState::Valid;
                    log::trace!("List of active stations is complete!");
                }
            }
            LasState::Valid => {
                self.update_las_from_token_pass(sa, da);
            }
        }
    }

    pub fn claim_token(&mut self) {
        if self.las_state != LasState::Valid {
            log::trace!("Declaring list of active stations valid due to claiming the token.");
        }
        self.las_state = LasState::Valid;
    }

    pub fn set_next_station(&mut self, address: crate::Address) {
        self.active_stations.set(usize::from(address), true);
        self.update_las_from_token_pass(self.this_station, address);
    }

    pub fn remove_station(&mut self, address: crate::Address) {
        self.active_stations.set(usize::from(address), false);
        self.update_next_previous();
    }
}

impl core::fmt::Debug for TokenRing {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut active_stations = [0u8; 127];
        let num_stations = self.iter_active_stations().count();
        for (i, addr) in self.iter_active_stations().enumerate() {
            active_stations[i] = addr;
        }
        f.debug_struct("TokenRing")
            .field("previous_station", &self.previous_station)
            .field("this_station", &self.this_station)
            .field("next_station", &self.next_station)
            .field("las_state", &self.las_state)
            .field("active_stations", &&active_stations[..num_stations])
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn test_token_ring_add_some_stations() {
        let mut token_ring = TokenRing::new(&Default::default());

        token_ring.witness_token_pass(29, 3);
        token_ring.witness_token_pass(3, 15);
        token_ring.witness_token_pass(15, 29);

        dbg!(&token_ring);
    }

    #[test]
    fn test_las_initialization_bad_verification() {
        let mut token_ring = TokenRing::new(&crate::fdl::Parameters {
            address: 7,
            ..Default::default()
        });

        assert_eq!(token_ring.las_state, LasState::Uninitialized);

        token_ring.witness_token_pass(15, 29);
        token_ring.witness_token_pass(29, 3);

        assert_eq!(token_ring.las_state, LasState::Discovery);

        token_ring.witness_token_pass(3, 15);
        token_ring.witness_token_pass(15, 29);
        token_ring.witness_token_pass(29, 3);

        assert_eq!(token_ring.las_state, LasState::Verification);

        token_ring.witness_token_pass(3, 15);
        token_ring.witness_token_pass(15, 18);
        token_ring.witness_token_pass(18, 29);

        assert_eq!(token_ring.las_state, LasState::Discovery);

        token_ring.witness_token_pass(29, 3);

        assert_eq!(token_ring.las_state, LasState::Verification);

        token_ring.witness_token_pass(3, 15);
        token_ring.witness_token_pass(15, 29);

        assert_eq!(token_ring.las_state, LasState::Discovery);

        token_ring.witness_token_pass(29, 3);

        assert_eq!(token_ring.las_state, LasState::Verification);

        token_ring.witness_token_pass(3, 15);
        token_ring.witness_token_pass(15, 29);
        token_ring.witness_token_pass(29, 3);

        assert_eq!(token_ring.las_state, LasState::Valid);
        assert!(token_ring.ready_for_ring());
        assert_eq!(token_ring.next_station(), 15);
        assert_eq!(token_ring.previous_station(), 3);
    }

    #[test]
    fn next_station_correct_after_removal() {
        let mut token_ring = TokenRing::new(&Default::default());

        token_ring.witness_token_pass(29, 3);
        token_ring.witness_token_pass(3, 15);
        token_ring.witness_token_pass(15, 29);

        assert_eq!(token_ring.next_station(), 3);

        token_ring.remove_station(3);

        assert_eq!(token_ring.next_station(), 15);
    }

    #[test]
    fn token_rink_eats_bad_addresses() {
        crate::test_utils::prepare_test_logger_with_warnings(vec![
            "Witnessed token pass from invalid address #223->#7, ignoring.",
            "Witnessed token pass to invalid address #223<-#7, ignoring.",
        ]);
        let mut token_ring = TokenRing::new(&Default::default());

        token_ring.witness_token_pass(223, 7);
        token_ring.witness_token_pass(7, 223);
    }

    proptest! {
        #[test]
        fn test_las_update_correctness(
            previous in prop::collection::vec(any::<bool>(), 126),
            da in 0..126u8,
            sa in 0..126u8,
        ) {
            let mut token_ring = TokenRing::new(&crate::fdl::Parameters {
                address: 7,
                ..Default::default()
            });
            for (mut station, state) in token_ring.active_stations.iter_mut().zip(previous.iter()) {
                *station = *state;
            }

            let current = token_ring.active_stations.clone();
            let verify = token_ring.verify_las_from_token_pass(sa, da);

            token_ring.update_las_from_token_pass(sa, da);

            if !verify {
                // Also put the destination address into the LAS to make verification happy
                token_ring.active_stations.set(usize::from(da), true);
            }

            if verify {
                assert_eq!(token_ring.active_stations, current);
            }

            assert!(token_ring.verify_las_from_token_pass(sa, da));

            let current = token_ring.active_stations.clone();
            token_ring.update_las_from_token_pass(sa, da);
            assert_eq!(token_ring.active_stations, current);
        }

        #[test]
        fn test_las_initialization_happy_path(
            mut active_stations in prop::collection::vec(0..126u8, 1..16),
        ) {
            let mut token_ring = TokenRing::new(&crate::fdl::Parameters {
                address: 7,
                ..Default::default()
            });

            active_stations.sort();
            active_stations.dedup();

            assert_eq!(token_ring.las_state, LasState::Uninitialized);

            for addresses in active_stations.windows(2) {
                let prev = addresses[0];
                let next = addresses[1];
                token_ring.witness_token_pass(prev, next);
            }
            // Wrap-around
            token_ring.witness_token_pass(active_stations[active_stations.len() - 1], active_stations[0]);

            assert_eq!(token_ring.las_state, LasState::Discovery);

            for addresses in active_stations.windows(2) {
                let prev = addresses[0];
                let next = addresses[1];
                token_ring.witness_token_pass(prev, next);
            }
            // Wrap-around
            token_ring.witness_token_pass(active_stations[active_stations.len() - 1], active_stations[0]);

            assert_eq!(token_ring.las_state, LasState::Verification);

            for addresses in active_stations.windows(2) {
                let prev = addresses[0];
                let next = addresses[1];
                token_ring.witness_token_pass(prev, next);
            }
            // Wrap-around
            token_ring.witness_token_pass(active_stations[active_stations.len() - 1], active_stations[0]);

            assert_eq!(token_ring.las_state, LasState::Valid);
            assert!(token_ring.ready_for_ring());

            let known_stations = token_ring.iter_active_stations().collect::<Vec<_>>();
            assert_eq!(active_stations, known_stations);

            let next = active_stations.iter().copied().find(|a| *a > 7).or_else(|| active_stations.iter().copied().next()).unwrap();
            assert_eq!(token_ring.next_station(), next);

            let previous = active_stations.iter().rev().copied().find(|a| *a < 7).or_else(|| active_stations.iter().rev().copied().next()).unwrap();
            assert_eq!(token_ring.previous_station(), previous);
        }
    }
}
