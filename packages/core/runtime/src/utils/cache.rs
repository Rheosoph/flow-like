use flow_like_types::Result;
use std::{
    fs::File,
    io::{Read, Write},
    path::PathBuf,
};

const CACHE_KEY: &str = "flow-like";

pub fn get_cache_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("CACHE_DIR") {
        return PathBuf::from(dir).join(CACHE_KEY);
    }

    if std::env::var("AWS_LAMBDA_FUNCTION_NAME").is_ok() {
        return PathBuf::from("/tmp").join(CACHE_KEY);
    }

    if let Some(cache_dir) = dirs_next::cache_dir() {
        cache_dir.join(CACHE_KEY)
    } else if let Some(data_dir) = dirs_next::data_dir() {
        data_dir.join(CACHE_KEY)
    } else if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join(".cache").join(CACHE_KEY)
    } else {
        PathBuf::from(CACHE_KEY)
    }
}

pub fn get_cache_file_path(name: &str) -> PathBuf {
    let parts = name.split("/").collect::<Vec<&str>>();
    let mut dir = get_cache_dir();
    for part in parts.iter() {
        dir = dir.join(part);
    }
    dir
}

pub fn delete_cache_file(name: &str) -> Result<()> {
    let path = get_cache_file_path(name);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

pub fn write_cache_file(name: &str, data: &[u8]) -> Result<()> {
    let path = get_cache_file_path(name);
    if !path.exists() {
        std::fs::create_dir_all(path.parent().unwrap())?;
    }
    let mut file = File::create(path)?;
    file.write_all(data)?;
    Ok(())
}

pub fn cache_file_exists(name: &str) -> bool {
    let path = get_cache_file_path(name);
    path.exists()
}

pub fn read_cache_file(name: &str) -> Result<Vec<u8>> {
    let path = get_cache_file_path(name);
    let mut file = File::open(path)?;
    let mut data = Vec::new();
    file.read_to_end(&mut data)?;
    Ok(data)
}

// TODO: should this be moved? ===============================================
pub fn read_file(file: &PathBuf) -> Result<Vec<u8>> {
    let mut file = File::open(file)?;
    let mut data = Vec::new();
    file.read_to_end(&mut data)?;
    Ok(data)
}

pub fn write_file(file: &PathBuf, data: &[u8]) -> Result<()> {
    if !file
        .parent()
        .ok_or(flow_like_types::anyhow!("No parent"))?
        .exists()
    {
        std::fs::create_dir_all(file.parent().unwrap())?;
    }
    let mut file = File::create(file)?;
    file.write_all(data)?;
    Ok(())
}
