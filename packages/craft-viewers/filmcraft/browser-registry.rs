//! Browser media sources are registered by the embedding application.
use filmcraft_media::{Opener, ReaderOpener};
use std::sync::{OnceLock, RwLock};

pub fn openers() -> Vec<Opener> {
    Vec::new()
}
// Browser sources own their decoder lifetimes; this registry has no idle pool.
pub fn release_idle() {}
pub fn reader_openers() -> Vec<ReaderOpener> {
    readers().read().unwrap_or_else(|e| e.into_inner()).clone()
}
fn readers() -> &'static RwLock<Vec<ReaderOpener>> {
    static READERS: OnceLock<RwLock<Vec<ReaderOpener>>> = OnceLock::new();
    READERS.get_or_init(|| RwLock::new(Vec::new()))
}
pub fn register_reader_opener(opener: ReaderOpener) {
    readers()
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .push(opener);
}
pub mod mp4 {
    pub fn sniff(bytes: &[u8]) -> bool {
        bytes
            .get(4..8)
            .is_some_and(|kind| matches!(kind, b"ftyp" | b"moov" | b"mdat" | b"wide" | b"free"))
    }
}
pub mod hw {
    pub fn set_hardware_decoding(_enabled: bool) {}
}
