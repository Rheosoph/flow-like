//! Plan container header: fixed-size, plain little-endian bytes.
//!
//! Never rkyv-encoded. A binary must be able to read the format version and stamps of a
//! plan written by *any* other binary — including a future one whose archived layout it
//! cannot interpret — so it can decide to fall back rather than misread.

use super::PlanError;

pub const PLAN_MAGIC: [u8; 4] = *b"FLPL";

/// Bump on any change to an archived struct in this module tree.
///
/// Plans are addressed by format version in the object name (`…f{N}.plan`), so versions
/// coexist and a mixed fleet never fights over the same object.
pub const PLAN_FORMAT_VERSION: u16 = 2;

/// Location, size and schema version of one section within the container.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SectionRef {
    pub offset: u64,
    pub len: u64,
    pub version: u16,
}

impl SectionRef {
    pub fn new(offset: u64, len: u64, version: u16) -> Self {
        Self {
            offset,
            len,
            version,
        }
    }
}

/// Stamps identifying exactly what a plan was compiled from.
///
/// Any mismatch at load time means the plan is stale and the caller recompiles from the
/// `.board`. `catalog_signature` matters because compilation freezes the result of
/// `Board::node_updates`, which today self-heals boards against catalog changes on every
/// load; a plan compiled against one catalog must never be run against another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanStamps {
    pub board_content_hash: u64,
    pub catalog_signature: u64,
    pub wasm_signature: u64,
    pub board_version: (u32, u32, u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanHeader {
    pub format_version: u16,
    pub stamps: PlanStamps,
    pub hot: SectionRef,
    pub cold: SectionRef,
    pub debug: SectionRef,
}

impl PlanHeader {
    /// magic(4) + format(2) + header_len(2) + stamps(8*3 + 4*3) + 3 sections(8+8+2+pad 6)
    pub const ENCODED_LEN: usize = 4 + 2 + 2 + 24 + 12 + (3 * 24);

    pub fn new(format_version: u16, stamps: PlanStamps) -> Self {
        Self {
            format_version,
            stamps,
            hot: SectionRef::default(),
            cold: SectionRef::default(),
            debug: SectionRef::default(),
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::ENCODED_LEN);
        out.extend_from_slice(&PLAN_MAGIC);
        out.extend_from_slice(&self.format_version.to_le_bytes());
        out.extend_from_slice(&(Self::ENCODED_LEN as u16).to_le_bytes());

        out.extend_from_slice(&self.stamps.board_content_hash.to_le_bytes());
        out.extend_from_slice(&self.stamps.catalog_signature.to_le_bytes());
        out.extend_from_slice(&self.stamps.wasm_signature.to_le_bytes());
        out.extend_from_slice(&self.stamps.board_version.0.to_le_bytes());
        out.extend_from_slice(&self.stamps.board_version.1.to_le_bytes());
        out.extend_from_slice(&self.stamps.board_version.2.to_le_bytes());

        for section in [&self.hot, &self.cold, &self.debug] {
            out.extend_from_slice(&section.offset.to_le_bytes());
            out.extend_from_slice(&section.len.to_le_bytes());
            out.extend_from_slice(&section.version.to_le_bytes());
            out.extend_from_slice(&[0u8; 6]);
        }

        debug_assert_eq!(out.len(), Self::ENCODED_LEN);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, PlanError> {
        if bytes.len() < Self::ENCODED_LEN {
            return Err(PlanError::Truncated {
                section: "header",
                needed: Self::ENCODED_LEN,
                available: bytes.len(),
            });
        }
        if bytes[0..4] != PLAN_MAGIC {
            return Err(PlanError::BadMagic);
        }

        let mut cursor = Cursor::new(bytes, 4);
        let format_version = cursor.u16();
        let _header_len = cursor.u16();

        let stamps = PlanStamps {
            board_content_hash: cursor.u64(),
            catalog_signature: cursor.u64(),
            wasm_signature: cursor.u64(),
            board_version: (cursor.u32(), cursor.u32(), cursor.u32()),
        };

        let mut sections = [SectionRef::default(); 3];
        for section in sections.iter_mut() {
            section.offset = cursor.u64();
            section.len = cursor.u64();
            section.version = cursor.u16();
            cursor.skip(6);
        }

        Ok(Self {
            format_version,
            stamps,
            hot: sections[0],
            cold: sections[1],
            debug: sections[2],
        })
    }

    /// Whether this plan was compiled from exactly the given inputs.
    pub fn matches(
        &self,
        board_content_hash: u64,
        catalog_signature: u64,
        wasm_signature: u64,
    ) -> bool {
        self.format_version == PLAN_FORMAT_VERSION
            && self.stamps.board_content_hash == board_content_hash
            && self.stamps.catalog_signature == catalog_signature
            && self.stamps.wasm_signature == wasm_signature
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8], pos: usize) -> Self {
        Self { bytes, pos }
    }

    fn take<const N: usize>(&mut self) -> [u8; N] {
        let mut buf = [0u8; N];
        buf.copy_from_slice(&self.bytes[self.pos..self.pos + N]);
        self.pos += N;
        buf
    }

    fn u16(&mut self) -> u16 {
        u16::from_le_bytes(self.take::<2>())
    }

    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.take::<4>())
    }

    fn u64(&mut self) -> u64 {
        u64::from_le_bytes(self.take::<8>())
    }

    fn skip(&mut self, n: usize) {
        self.pos += n;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_roundtrips() {
        let mut header = PlanHeader::new(
            PLAN_FORMAT_VERSION,
            PlanStamps {
                board_content_hash: 0xdead_beef_cafe_1234,
                catalog_signature: 42,
                wasm_signature: 7,
                board_version: (1, 2, 3),
            },
        );
        header.hot = SectionRef::new(128, 4096, 1);
        header.cold = SectionRef::new(4224, 512, 1);
        header.debug = SectionRef::new(4736, 0, 1);

        let encoded = header.encode();
        assert_eq!(encoded.len(), PlanHeader::ENCODED_LEN);
        assert_eq!(PlanHeader::decode(&encoded).unwrap(), header);
    }

    #[test]
    fn rejects_foreign_bytes() {
        let bytes = vec![0u8; PlanHeader::ENCODED_LEN];
        assert!(matches!(
            PlanHeader::decode(&bytes),
            Err(PlanError::BadMagic)
        ));
    }

    #[test]
    fn rejects_truncated_header() {
        assert!(matches!(
            PlanHeader::decode(&PLAN_MAGIC),
            Err(PlanError::Truncated { .. })
        ));
    }

    /// A future binary's plan must still be *readable enough* to reject cleanly.
    #[test]
    fn future_format_version_is_visible() {
        let header = PlanHeader::new(
            PLAN_FORMAT_VERSION + 9,
            PlanStamps {
                board_content_hash: 1,
                catalog_signature: 2,
                wasm_signature: 3,
                board_version: (0, 0, 1),
            },
        );
        let decoded = PlanHeader::decode(&header.encode()).unwrap();
        assert_eq!(decoded.format_version, PLAN_FORMAT_VERSION + 9);
        assert!(!decoded.matches(1, 2, 3));
    }
}
