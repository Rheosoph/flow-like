use std::{
    fs::File,
    io::Write,
    path::{Path, PathBuf},
};

pub struct LockFile {
    pub path: PathBuf,
}

impl LockFile {
    pub fn new(path: &Path) -> LockFile {
        LockFile {
            path: path.to_path_buf(),
        }
    }

    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    pub fn create(&self) -> flow_like_types::Result<()> {
        let mut file = File::create(&self.path)?;
        file.write_all(b"")?;
        file.sync_all()?;
        Ok(())
    }

    pub fn delete(&self) -> flow_like_types::Result<()> {
        std::fs::remove_file(&self.path)?;
        Ok(())
    }
}
