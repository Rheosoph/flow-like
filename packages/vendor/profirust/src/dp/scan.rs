#[derive(Debug, PartialEq, Eq, Clone)]
pub struct DpPeripheralDescription {
    pub address: crate::Address,
    pub ident: u16,
    pub master_address: Option<crate::Address>,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum DpScanEvent {
    PeripheralFound(DpPeripheralDescription),
    PeripheralRequery(DpPeripheralDescription),
    PeripheralLost(crate::Address),
}

pub struct DpScanner {
    stations: bitvec::BitArr!(for 128),
    cursor: crate::Address,
    pending_event: Option<DpScanEvent>,
    current_address_done: bool,
}

impl DpScanner {
    pub fn new() -> Self {
        Self {
            stations: bitvec::array::BitArray::ZERO,
            cursor: 0,
            pending_event: None,
            current_address_done: false,
        }
    }

    pub fn take_last_event(&mut self) -> Option<DpScanEvent> {
        self.pending_event.take()
    }

    fn parse_diag_response(
        &self,
        telegram: crate::fdl::Telegram,
        address: crate::Address,
    ) -> Option<crate::dp::DiagnosticsInfo> {
        if let crate::fdl::Telegram::Data(t) = telegram {
            if t.h.dsap != crate::consts::SAP_MASTER_MS0 {
                log::warn!("Diagnostics response by #{} to wrong SAP: {t:?}", address);
                return None;
            }
            if t.h.ssap != crate::consts::SAP_SLAVE_DIAGNOSIS {
                log::warn!("Diagnostics response by #{} from wrong SAP: {t:?}", address);
                return None;
            }
            if t.pdu.len() < 6 {
                log::warn!("Diagnostics response by #{} is too short: {t:?}", address);
                return None;
            }

            let master_address = if t.pdu[3] == 255 {
                None
            } else {
                Some(t.pdu[3])
            };

            let mut diag = crate::dp::DiagnosticsInfo {
                flags: crate::dp::DiagnosticFlags::from_bits_retain(u16::from_le_bytes(
                    t.pdu[0..2].try_into().unwrap(),
                )),
                master_address,
                ident_number: u16::from_be_bytes(t.pdu[4..6].try_into().unwrap()),
            };

            if !diag
                .flags
                .contains(crate::dp::DiagnosticFlags::PERMANENT_BIT)
            {
                log::warn!("Inconsistent diagnostics for peripheral #{}!", address);
            }
            // we don't need the permanent bit anymore now
            diag.flags.remove(crate::dp::DiagnosticFlags::PERMANENT_BIT);

            log::debug!("Peripheral Diagnostics (#{}): {:?}", address, diag);

            if diag.flags.contains(crate::dp::DiagnosticFlags::EXT_DIAG) {
                log::debug!("Extended Diagnostics (#{}): {:?}", address, &t.pdu[6..]);
            }

            Some(diag)
        } else {
            log::warn!(
                "Unexpected diagnostics response for #{}: {telegram:?}",
                address
            );
            None
        }
    }
}

impl crate::fdl::FdlApplication for DpScanner {
    fn transmit_telegram(
        &mut self,
        now: crate::time::Instant,
        fdl: &crate::fdl::FdlActiveStation,
        tx: crate::fdl::TelegramTx,
        high_prio_only: bool,
    ) -> Option<crate::fdl::TelegramTxResponse> {
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
            Some(tx.send_data_telegram(
                crate::fdl::DataTelegramHeader {
                    da: address,
                    sa: this_station,
                    dsap: crate::consts::SAP_SLAVE_DIAGNOSIS,
                    ssap: crate::consts::SAP_MASTER_MS0,
                    fc: crate::fdl::FunctionCode::new_srd_low(crate::fdl::FrameCountBit::First),
                },
                0,
                |_buf| (),
            ))
        }
    }

    fn receive_reply(
        &mut self,
        now: crate::time::Instant,
        fdl: &crate::fdl::FdlActiveStation,
        address: u8,
        telegram: crate::fdl::Telegram,
    ) {
        let station_unknown = !self.stations.get(usize::from(address)).unwrap();
        self.current_address_done = true;

        let event = if let Some(diag) = self.parse_diag_response(telegram, address) {
            let desc = DpPeripheralDescription {
                address,
                ident: diag.ident_number,
                master_address: diag.master_address,
            };

            if station_unknown {
                Some(DpScanEvent::PeripheralFound(desc))
            } else {
                Some(DpScanEvent::PeripheralRequery(desc))
            }
        } else {
            None
        };

        log::trace!("Received reply from #{address}: {:?}", event);

        if station_unknown && event.is_some() {
            self.stations.set(usize::from(address), true);
        }

        self.pending_event = event;
    }

    fn handle_timeout(
        &mut self,
        now: crate::time::Instant,
        fdl: &crate::fdl::FdlActiveStation,
        address: u8,
    ) {
        self.current_address_done = true;
        if *self.stations.get(usize::from(address)).unwrap() {
            log::debug!("Lost peripheral #{}.", address,);
            self.pending_event = Some(DpScanEvent::PeripheralLost(address));
            self.stations.set(usize::from(address), false);
        } else {
            log::trace!("Timeout for address #{address}.");
        }
    }
}
