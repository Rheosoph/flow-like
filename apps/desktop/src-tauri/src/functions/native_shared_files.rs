use std::{io::Read, path::Path};

pub(crate) fn read(root: &Path, path: &Path) -> Result<(Vec<u8>, String), String> {
    const MAX_BYTES: u64 = 20 * 1024 * 1024;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    if !path.starts_with(&root) || path == root {
        return Err("The file is outside the shared-file directory".into());
    }
    if !path.is_file() {
        return Err("Shared files must be regular files".into());
    }
    let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return Err("Shared files must be regular files of at most 20 MiB".into());
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Shared file exceeds 20 MiB".into());
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Invalid shared filename")?;
    Ok((bytes, name.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "flow-native-shared-test-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            fs::create_dir(path.join("SharedFiles")).unwrap();
            Self(path)
        }
        fn root(&self) -> PathBuf {
            self.0.join("SharedFiles")
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn imports_only_regular_files_within_the_share_directory() {
        let fixture = Fixture::new();
        let file = fixture.root().join("translation.txt");
        fs::write(&file, b"bonjour").unwrap();
        assert_eq!(
            read(&fixture.root(), &file).unwrap(),
            (b"bonjour".to_vec(), "translation.txt".to_owned())
        );
        assert!(read(&fixture.root(), &fixture.root()).is_err());
        fs::create_dir(fixture.root().join("folder")).unwrap();
        assert!(read(&fixture.root(), &fixture.root().join("folder")).is_err());
    }

    #[test]
    fn rejects_traversal_and_sibling_directory_prefixes() {
        let fixture = Fixture::new();
        fs::write(fixture.0.join("outside.txt"), b"private").unwrap();
        assert!(read(&fixture.root(), &fixture.root().join("../outside.txt")).is_err());
        fs::create_dir(fixture.0.join("SharedFiles-other")).unwrap();
        let sibling = fixture.0.join("SharedFiles-other/private.txt");
        fs::write(&sibling, b"private").unwrap();
        assert!(read(&fixture.root(), &sibling).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn rejects_symlinks_to_files_outside_the_share_directory() {
        let fixture = Fixture::new();
        let outside = fixture.0.join("outside.txt");
        fs::write(&outside, b"private").unwrap();
        let link = fixture.root().join("link.txt");
        std::os::unix::fs::symlink(outside, &link).unwrap();
        assert!(read(&fixture.root(), &link).is_err());
    }

    #[test]
    fn refuses_oversized_files_before_reading() {
        let fixture = Fixture::new();
        let path = fixture.root().join("oversized.bin");
        fs::File::create(&path)
            .unwrap()
            .set_len(20 * 1024 * 1024 + 1)
            .unwrap();
        assert!(read(&fixture.root(), &path).unwrap_err().contains("20 MiB"));
    }
}
