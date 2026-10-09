//! Minimal PKZIP reader for zipped GenICam XMLs: locate the
//! end-of-central-directory record from the tail,
//! walk the central directory, extract the first file (raw DEFLATE or
//! stored).

use miniz_oxide::inflate::decompress_to_vec_with_limit;

const EOCD_MAGIC: u32 = 0x0605_4b50; // PK\x05\x06
const CENTRAL_MAGIC: u32 = 0x0201_4b50; // PK\x01\x02
const LOCAL_MAGIC: u32 = 0x0403_4b50; // PK\x03\x04

const METHOD_STORED: u16 = 0;
const METHOD_DEFLATE: u16 = 8;

/// Sanity bound: GenICam XMLs are at most a few MiB.
const MAX_UNCOMPRESSED: usize = 64 * 1024 * 1024;

pub fn extract_first_file(zip: &[u8]) -> Result<Vec<u8>, String> {
    let eocd = find_eocd(zip).ok_or("no end-of-central-directory record")?;
    let n_entries = u16_at(zip, eocd + 10)? as usize;
    let central_offset = u32_at(zip, eocd + 16)? as usize;
    if n_entries == 0 {
        return Err("empty archive".into());
    }

    let entry = central_offset;
    if u32_at(zip, entry)? != CENTRAL_MAGIC {
        return Err("bad central directory".into());
    }
    let method = u16_at(zip, entry + 10)?;
    let compressed_size = u32_at(zip, entry + 20)? as usize;
    let uncompressed_size = u32_at(zip, entry + 24)? as usize;
    let local_offset = u32_at(zip, entry + 42)? as usize;
    if uncompressed_size > MAX_UNCOMPRESSED {
        return Err("archive entry too large".into());
    }

    if u32_at(zip, local_offset)? != LOCAL_MAGIC {
        return Err("bad local file header".into());
    }
    // Name/extra lengths from the *local* header decide the data offset.
    let name_len = u16_at(zip, local_offset + 26)? as usize;
    let extra_len = u16_at(zip, local_offset + 28)? as usize;
    let data_start = local_offset + 30 + name_len + extra_len;
    let data = zip
        .get(data_start..data_start + compressed_size)
        .ok_or("truncated archive data")?;

    match method {
        METHOD_STORED => Ok(data.to_vec()),
        METHOD_DEFLATE => decompress_to_vec_with_limit(data, MAX_UNCOMPRESSED)
            .map_err(|e| format!("deflate: {e:?}")),
        other => Err(format!("unsupported compression method {other}")),
    }
}

fn find_eocd(zip: &[u8]) -> Option<usize> {
    // The EOCD is 22 bytes plus an optional comment of up to 64 KiB.
    let start = zip.len().checked_sub(22)?;
    (0..=start.min(0xffff))
        .map(|back| start - back)
        .find(|&pos| u32_at(zip, pos) == Ok(EOCD_MAGIC))
}

fn u16_at(buf: &[u8], pos: usize) -> Result<u16, String> {
    buf.get(pos..pos + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| "truncated archive".into())
}

fn u32_at(buf: &[u8], pos: usize) -> Result<u32, String> {
    buf.get(pos..pos + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| "truncated archive".into())
}

#[cfg(test)]
pub(crate) fn build_zip(filename: &str, content: &[u8], deflate: bool) -> Vec<u8> {
    let (method, data): (u16, Vec<u8>) = if deflate {
        (
            METHOD_DEFLATE,
            miniz_oxide::deflate::compress_to_vec(content, 6),
        )
    } else {
        (METHOD_STORED, content.to_vec())
    };
    let crc = crc32(content);

    let mut zip = Vec::new();
    zip.extend_from_slice(&LOCAL_MAGIC.to_le_bytes());
    zip.extend_from_slice(&20u16.to_le_bytes()); // version needed
    zip.extend_from_slice(&0u16.to_le_bytes()); // flags
    zip.extend_from_slice(&method.to_le_bytes());
    zip.extend_from_slice(&[0u8; 4]); // time/date
    zip.extend_from_slice(&crc.to_le_bytes());
    zip.extend_from_slice(&(data.len() as u32).to_le_bytes());
    zip.extend_from_slice(&(content.len() as u32).to_le_bytes());
    zip.extend_from_slice(&(filename.len() as u16).to_le_bytes());
    zip.extend_from_slice(&0u16.to_le_bytes()); // extra len
    zip.extend_from_slice(filename.as_bytes());
    zip.extend_from_slice(&data);

    let central_offset = zip.len();
    zip.extend_from_slice(&CENTRAL_MAGIC.to_le_bytes());
    zip.extend_from_slice(&20u16.to_le_bytes()); // version made by
    zip.extend_from_slice(&20u16.to_le_bytes()); // version needed
    zip.extend_from_slice(&0u16.to_le_bytes()); // flags
    zip.extend_from_slice(&method.to_le_bytes());
    zip.extend_from_slice(&[0u8; 4]); // time/date
    zip.extend_from_slice(&crc.to_le_bytes());
    zip.extend_from_slice(&(data.len() as u32).to_le_bytes());
    zip.extend_from_slice(&(content.len() as u32).to_le_bytes());
    zip.extend_from_slice(&(filename.len() as u16).to_le_bytes());
    zip.extend_from_slice(&[0u8; 12]); // extra/comment lens, disk, attrs(int)
    zip.extend_from_slice(&[0u8; 4]); // external attrs
    zip.extend_from_slice(&0u32.to_le_bytes()); // local header offset
    zip.extend_from_slice(filename.as_bytes());
    let central_size = zip.len() - central_offset;

    zip.extend_from_slice(&EOCD_MAGIC.to_le_bytes());
    zip.extend_from_slice(&[0u8; 4]); // disk numbers
    zip.extend_from_slice(&1u16.to_le_bytes()); // entries on disk
    zip.extend_from_slice(&1u16.to_le_bytes()); // entries total
    zip.extend_from_slice(&(central_size as u32).to_le_bytes());
    zip.extend_from_slice(&(central_offset as u32).to_le_bytes());
    zip.extend_from_slice(&0u16.to_le_bytes()); // comment len
    zip
}

#[cfg(test)]
fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_deflate() {
        let content = b"<RegisterDescription>hello</RegisterDescription>".repeat(50);
        let zip = build_zip("cam.xml", &content, true);
        assert!(zip.len() < content.len(), "deflate should compress");
        assert_eq!(extract_first_file(&zip).unwrap(), content);
    }

    #[test]
    fn roundtrip_stored() {
        let content = b"stored content".to_vec();
        let zip = build_zip("cam.xml", &content, false);
        assert_eq!(extract_first_file(&zip).unwrap(), content);
    }

    #[test]
    fn rejects_garbage() {
        assert!(extract_first_file(b"not a zip at all").is_err());
        assert!(extract_first_file(&[]).is_err());
    }
}
