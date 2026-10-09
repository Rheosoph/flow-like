use crate::dp::Peripheral;
use core::fmt;

/// Storage type that can hold one peripheral.
#[derive(Default)]
pub struct PeripheralStorage<'a> {
    inner: Option<Peripheral<'a>>,
}

/// Handle that can be used to obtain a peripheral from the DP master.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PeripheralHandle {
    index: u8,
    address: u8,
}

impl PeripheralHandle {
    #[inline(always)]
    pub fn address(self) -> u8 {
        self.address
    }
}

impl fmt::Display for PeripheralHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Station {}", self.address)
    }
}

pub struct PeripheralSet<'a> {
    peripherals: managed::ManagedSlice<'a, PeripheralStorage<'a>>,
}

impl<'a> PeripheralSet<'a> {
    pub fn new<S>(storage: S) -> Self
    where
        S: Into<managed::ManagedSlice<'a, PeripheralStorage<'a>>>,
    {
        Self {
            peripherals: storage.into(),
        }
    }

    /// Add a peripheral to the set, and return its handle.
    ///
    /// # Panics
    /// This function panics if the storage is fixed-size (not a `Vec`) and is full.
    pub fn add(&mut self, peripheral: Peripheral<'a>) -> PeripheralHandle {
        for (index, slot) in self.peripherals.iter_mut().enumerate() {
            if slot.inner.is_none() {
                let address = peripheral.address();
                slot.inner = Some(peripheral);
                return PeripheralHandle {
                    index: u8::try_from(index).unwrap(),
                    address,
                };
            }
        }

        match &mut self.peripherals {
            managed::ManagedSlice::Borrowed(_) => panic!("Adding peripheral to full PeripheralSet"),
            #[cfg(any(feature = "std", feature = "alloc"))]
            managed::ManagedSlice::Owned(peripherals) => {
                let address = peripheral.address();
                peripherals.push(PeripheralStorage {
                    inner: Some(peripheral),
                });
                PeripheralHandle {
                    index: (peripherals.len() - 1).try_into().unwrap(),
                    address,
                }
            }
        }
    }

    /// Get a peripheral from the set by its handle, as mutable.
    ///
    /// # Panics
    /// This function may panic if the handle does not belong to this peripheral set.
    pub fn get_mut(&mut self, handle: PeripheralHandle) -> &mut Peripheral<'a> {
        self.peripherals[usize::from(handle.index)]
            .inner
            .as_mut()
            .expect("Handle does not refer to a valid peripheral")
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (PeripheralHandle, &mut Peripheral<'a>)> {
        self.peripherals
            .iter_mut()
            .enumerate()
            .filter_map(|(i, p)| {
                p.inner.as_mut().map(|p| {
                    (
                        PeripheralHandle {
                            index: u8::try_from(i).unwrap(),
                            address: p.address(),
                        },
                        p,
                    )
                })
            })
    }

    pub fn iter(&self) -> impl Iterator<Item = (PeripheralHandle, &Peripheral<'a>)> {
        self.peripherals.iter().enumerate().filter_map(|(i, p)| {
            p.inner.as_ref().map(|p| {
                (
                    PeripheralHandle {
                        index: u8::try_from(i).unwrap(),
                        address: p.address(),
                    },
                    p,
                )
            })
        })
    }

    pub(crate) fn get_at_index_mut(
        &mut self,
        index: u8,
    ) -> Option<(PeripheralHandle, &mut Peripheral<'a>)> {
        self.peripherals
            .iter_mut()
            .enumerate()
            .skip(usize::from(index))
            .find_map(|(i, p)| {
                p.inner.as_mut().map(|p| {
                    (
                        PeripheralHandle {
                            index: u8::try_from(i).unwrap(),
                            address: p.address(),
                        },
                        p,
                    )
                })
            })
    }

    pub(crate) fn get_next_index(&mut self, index: u8) -> Option<u8> {
        self.peripherals
            .iter_mut()
            .enumerate()
            .skip(usize::from(index))
            .filter(|(_, p)| p.inner.is_some())
            .nth(1)
            .map(|(i, _)| u8::try_from(i).unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_index_api() {
        let buffer = [
            PeripheralStorage::default(),
            PeripheralStorage::default(),
            PeripheralStorage::default(),
            PeripheralStorage {
                inner: Some(Peripheral::default()),
            },
            PeripheralStorage::default(),
            PeripheralStorage::default(),
            PeripheralStorage {
                inner: Some(Peripheral::default()),
            },
            PeripheralStorage {
                inner: Some(Peripheral::default()),
            },
            PeripheralStorage::default(),
            PeripheralStorage {
                inner: Some(Peripheral::default()),
            },
            PeripheralStorage::default(),
            PeripheralStorage::default(),
        ];
        let mut set = PeripheralSet::new(buffer);

        let mut i = 0;
        let mut indices = vec![];
        loop {
            let (h, _) = match set.get_at_index_mut(i) {
                Some(p) => p,
                None => break,
            };
            indices.push(h.index);

            if let Some(next) = set.get_next_index(i) {
                i = next;
            } else {
                break;
            }

            // Do some mean shenanigans
            if i == 6 {
                set.peripherals[0].inner = Some(Peripheral::default());
            } else if i == 9 {
                set.peripherals[11].inner = Some(Peripheral::default());
            }
        }

        assert_eq!(indices, &[3, 6, 7, 9, 11]);

        let mut i = 0;
        let mut indices = vec![];
        loop {
            let (h, _) = match set.get_at_index_mut(i) {
                Some(p) => p,
                None => break,
            };
            indices.push(h.index);

            if let Some(next) = set.get_next_index(i) {
                i = next;
            } else {
                break;
            }

            // Do some mean shenanigans
            if i == 6 {
                set.peripherals[6].inner = None;
            } else if i == 9 {
                set.peripherals[11].inner = None;
            }
        }

        assert_eq!(indices, &[0, 3, 7, 9]);

        let mut i = 0;
        let mut indices = vec![];
        loop {
            let (h, _) = match set.get_at_index_mut(i) {
                Some(p) => p,
                None => break,
            };
            indices.push(h.index);

            if let Some(next) = set.get_next_index(i) {
                i = next;
            } else {
                break;
            }

            // Do some mean shenanigans
            if i == 9 {
                set.peripherals[9].inner = None;
            }
        }

        assert_eq!(indices, &[0, 3, 7]);
    }
}
