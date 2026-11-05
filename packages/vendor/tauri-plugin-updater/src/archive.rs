// Copyright 2019-2026 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use std::{
    io::{Read, Seek},
    path::Path,
};

pub(crate) fn extract_zip<R: Read + Seek>(reader: R, target: &Path) -> zip::result::ZipResult<()> {
    let mut archive = zip::ZipArchive::new(reader)?;
    // Check every entry before extraction can write any files.
    archive.validate_header_consistency()?;
    archive.extract(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    fn archive() -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for name in ["first.txt", "setup.exe"] {
            writer.start_file(name, options).unwrap();
            writer.write_all(b"installer").unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn extracts_consistent_installer_archive() {
        let temp = tempfile::tempdir().unwrap();
        extract_zip(Cursor::new(archive()), temp.path()).unwrap();
        assert_eq!(
            std::fs::read(temp.path().join("setup.exe")).unwrap(),
            b"installer"
        );
    }

    #[test]
    fn rejects_conflicting_names_before_extracting_any_entry() {
        let mut bytes = archive();
        let offset = zip::ZipArchive::new(Cursor::new(&bytes))
            .unwrap()
            .by_name("setup.exe")
            .unwrap()
            .header_start() as usize;
        bytes[offset + 30] = b'x';
        let temp = tempfile::tempdir().unwrap();
        let error = extract_zip(Cursor::new(bytes), temp.path()).unwrap_err();
        assert!(error.to_string().contains("filename mismatch"), "{error}");
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
    }
}
