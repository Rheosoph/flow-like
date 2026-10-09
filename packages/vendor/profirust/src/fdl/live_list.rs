#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StationDescription {
    pub address: crate::Address,
    pub state: super::ResponseState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StationEvent {
    Discovered(StationDescription),
    Lost(crate::Address),
}

#[derive(Debug, Clone)]
pub struct LiveList {
    stations: bitvec::BitArr!(for 128),
    cursor: crate::Address,
    pending_event: Option<StationEvent>,
    current_address_done: bool,
}

impl LiveList {
    pub fn new() -> Self {
        Self {
            stations: bitvec::array::BitArray::ZERO,
            cursor: 0,
            pending_event: None,
            current_address_done: false,
        }
    }

    pub fn iter_stations(&self) -> impl Iterator<Item = crate::Address> + DoubleEndedIterator + '_ {
        self.stations.iter_ones().map(|a| u8::try_from(a).unwrap())
    }

    pub fn take_last_event(&mut self) -> Option<StationEvent> {
        self.pending_event.take()
    }
}

impl crate::fdl::FdlApplication for LiveList {
    fn transmit_telegram(
        &mut self,
        now: crate::time::Instant,
        fdl: &super::FdlActiveStation,
        tx: super::TelegramTx,
        high_prio_only: bool,
    ) -> Option<super::TelegramTxResponse> {
        let this_station = fdl.parameters().address;
        let address = self.cursor;

        if self.current_address_done {
            self.current_address_done = false;
            if self.cursor < 125 {
                self.cursor += 1;
            } else {
                self.cursor = 0;
            }
            None
        } else {
            Some(tx.send_fdl_status_request(address, this_station))
        }
    }

    fn receive_reply(
        &mut self,
        now: crate::time::Instant,
        fdl: &super::FdlActiveStation,
        addr: u8,
        telegram: super::Telegram,
    ) {
        self.current_address_done = true;
        let event = if !self.stations.get(usize::from(addr)).unwrap() {
            self.stations.set(usize::from(addr), true);

            if let super::Telegram::Data(super::DataTelegram {
                h:
                    super::DataTelegramHeader {
                        fc: super::FunctionCode::Response { state, status },
                        ..
                    },
                ..
            }) = telegram
            {
                Some(StationEvent::Discovered(StationDescription {
                    address: addr,
                    state,
                }))
            } else {
                None
            }
        } else {
            // We know this station already, so no event.
            None
        };

        self.pending_event = event;
    }

    fn handle_timeout(
        &mut self,
        now: crate::time::Instant,
        fdl: &super::FdlActiveStation,
        addr: u8,
    ) {
        self.current_address_done = true;
        if *self.stations.get(usize::from(addr)).unwrap() {
            self.pending_event = Some(StationEvent::Lost(addr));
            self.stations.set(usize::from(addr), false);
        }
    }
}
