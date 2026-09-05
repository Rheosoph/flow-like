#[cfg(target_family = "windows")]
use flow_like_types::tokio::fs::{File, OpenOptions};
#[cfg(target_family = "windows")]
use flow_like_types::tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use std::io;
#[cfg(target_family = "windows")]
use std::path::Path;
use std::path::PathBuf;

#[cfg(target_family = "windows")]
const BUFFER_SIZE: usize = 8192; // 8 KB buffer size

#[cfg(target_family = "windows")]
async fn copy_large_file<P: AsRef<Path>>(source: P, destination: P) -> io::Result<u64> {
    let source_file = File::open(source).await?;
    let destination_file = OpenOptions::new()
        .write(true)
        .create(true)
        .open(destination)
        .await?;

    let mut reader = BufReader::new(source_file);
    let mut writer = BufWriter::new(destination_file);

    let mut total_bytes_written = 0;
    let mut buffer = vec![0; BUFFER_SIZE];

    loop {
        // Read a chunk of data
        let bytes_read = match reader.read(&mut buffer).await {
            Ok(0) => break, // EOF
            Ok(n) => n,
            Err(e) => return Err(e),
        };

        // Write the chunk to the destination
        writer.write_all(&buffer[..bytes_read]).await?;
        total_bytes_written += bytes_read as u64;
    }

    // Ensure all data is flushed to the file
    writer.flush().await?;
    Ok(total_bytes_written)
}

#[cfg(target_family = "unix")]
pub async fn create_symlink_or_junction(src: PathBuf, target_dir: PathBuf) -> io::Result<()> {
    std::os::unix::fs::symlink(src, target_dir)
}

#[cfg(target_family = "windows")]
pub async fn create_symlink_or_junction(src: PathBuf, target_dir: PathBuf) -> io::Result<()> {
    match std::fs::hard_link(&src, &target_dir) {
        Ok(_) => Ok(()),
        Err(_) => {
            // probably a different drive
            println!("Falling back to copying the file");
            match copy_large_file(src, target_dir).await {
                Ok(_) => Ok(()),
                Err(e) => Err(e),
            }
        }
    }
}
