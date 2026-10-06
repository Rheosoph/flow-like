use anyhow::{Result, bail};
use futures::TryStreamExt;
use object_store::GetResult;

/// A seekable staged object. Disk failures retain the previous in-memory behavior.
#[derive(Debug)]
pub enum StagedObject {
    File(std::fs::File),
    Memory(std::io::Cursor<bytes::Bytes>),
}

impl StagedObject {
    pub fn new() -> Self {
        Self::from_file_result(tempfile::tempfile())
    }

    fn from_file_result(file: std::io::Result<std::fs::File>) -> Self {
        match file {
            Ok(file) => Self::File(file),
            Err(_) => Self::Memory(std::io::Cursor::new(bytes::Bytes::new())),
        }
    }

    pub fn try_clone(&self) -> std::io::Result<Self> {
        match self {
            Self::File(file) => file.try_clone().map(Self::File),
            Self::Memory(bytes) => Ok(Self::Memory(bytes.clone())),
        }
    }
}

impl Default for StagedObject {
    fn default() -> Self {
        Self::new()
    }
}

impl From<std::fs::File> for StagedObject {
    fn from(file: std::fs::File) -> Self {
        Self::File(file)
    }
}

impl From<Vec<u8>> for StagedObject {
    fn from(bytes: Vec<u8>) -> Self {
        Self::Memory(std::io::Cursor::new(bytes::Bytes::from(bytes)))
    }
}

impl std::io::Read for StagedObject {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::File(file) => std::io::Read::read(file, buffer),
            Self::Memory(bytes) => std::io::Read::read(bytes, buffer),
        }
    }
}

impl std::io::Seek for StagedObject {
    fn seek(&mut self, position: std::io::SeekFrom) -> std::io::Result<u64> {
        match self {
            Self::File(file) => std::io::Seek::seek(file, position),
            Self::Memory(bytes) => std::io::Seek::seek(bytes, position),
        }
    }
}

impl std::io::Write for StagedObject {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        use std::io::{Read, Seek};
        match self {
            Self::Memory(cursor) => {
                let position = usize::try_from(cursor.position())
                    .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
                let end = position
                    .checked_add(buffer.len())
                    .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
                let mut bytes = std::mem::take(cursor.get_mut())
                    .try_into_mut()
                    .unwrap_or_else(|shared| bytes::BytesMut::from(shared.as_ref()));
                if end > bytes.len() {
                    bytes.resize(end, 0);
                }
                bytes[position..end].copy_from_slice(buffer);
                *cursor.get_mut() = bytes.freeze();
                cursor.set_position(end as u64);
                Ok(buffer.len())
            }
            Self::File(file) => match file.write(buffer) {
                Ok(count) => Ok(count),
                Err(_) => {
                    let position = file.stream_position()?;
                    file.rewind()?;
                    let mut bytes = Vec::new();
                    file.take(position).read_to_end(&mut bytes)?;
                    bytes.extend_from_slice(buffer);
                    let mut cursor = std::io::Cursor::new(bytes::Bytes::from(bytes));
                    cursor.set_position(position + buffer.len() as u64);
                    *self = Self::Memory(cursor);
                    Ok(buffer.len())
                }
            },
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::File(file) => std::io::Write::flush(file),
            Self::Memory(_) => Ok(()),
        }
    }
}

/// Stream to disk when available, falling back to memory if disk cannot accept it.
pub async fn spool_object(result: GetResult) -> Result<StagedObject> {
    use std::io::{Seek, Write};

    let mut file = StagedObject::new();
    let mut stream = result.into_stream();
    while let Some(chunk) = stream.try_next().await? {
        file = tokio::task::spawn_blocking(move || -> std::io::Result<_> {
            file.write_all(&chunk)?;
            Ok(file)
        })
        .await??;
    }
    tokio::task::spawn_blocking(move || -> std::io::Result<_> {
        file.rewind()?;
        Ok(file)
    })
    .await?
    .map_err(Into::into)
}

/// Reject oversized metadata before reading and enforce the same limit on bytes received.
pub async fn read_object(result: GetResult, max_bytes: u64) -> Result<Vec<u8>> {
    if result.meta.size > max_bytes {
        bail!("Object exceeds the {max_bytes} byte limit");
    }
    let mut stream = result.into_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.try_next().await? {
        if (bytes.len() as u64).saturating_add(chunk.len() as u64) > max_bytes {
            bail!("Object exceeds the {max_bytes} byte limit");
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use object_store::{ObjectStoreExt, memory::InMemory, path::Path};

    #[test]
    fn unavailable_or_exhausted_disk_preserves_the_complete_object() {
        use std::io::{Read, Seek, Write};
        let mut unavailable =
            StagedObject::from_file_result(Err(std::io::ErrorKind::PermissionDenied.into()));
        unavailable.write_all(b"whole input").unwrap();
        unavailable.rewind().unwrap();
        let mut bytes = Vec::new();
        unavailable.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"whole input");

        let mut source = tempfile::NamedTempFile::new().unwrap();
        source.write_all(b"already staged").unwrap();
        let mut read_only = std::fs::File::open(source.path()).unwrap();
        read_only.seek(std::io::SeekFrom::End(0)).unwrap();
        let mut exhausted = StagedObject::File(read_only);
        exhausted.write_all(b" and remaining stream").unwrap();
        assert!(matches!(exhausted, StagedObject::Memory(_)));
        exhausted.rewind().unwrap();
        bytes.clear();
        exhausted.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"already staged and remaining stream");
    }

    #[test]
    fn memory_readers_share_storage_and_writes_preserve_other_readers() {
        use std::io::{Read, Seek, Write};
        let bytes = vec![7; 11 * 1024 * 1024];
        let original = bytes.as_ptr();
        let mut staged = StagedObject::from(bytes);
        let mut reader = staged.try_clone().unwrap();
        for value in [&staged, &reader] {
            let StagedObject::Memory(cursor) = value else {
                panic!("expected memory")
            };
            assert_eq!(cursor.get_ref().as_ptr(), original);
        }
        staged.write_all(&[42]).unwrap();
        staged.rewind().unwrap();
        let mut first = [0];
        staged.read_exact(&mut first).unwrap();
        assert_eq!(first, [42]);
        reader.read_exact(&mut first).unwrap();
        assert_eq!(first, [7]);
    }

    #[tokio::test]
    async fn spools_the_complete_stream_regardless_of_reported_size() {
        use std::io::Read;

        let store = InMemory::new();
        let path = Path::from("artifact");
        store.put(&path, vec![7; 256 * 1024].into()).await.unwrap();
        let mut result = store.get(&path).await.unwrap();
        result.meta.size = 1;
        let mut file = spool_object(result).await.unwrap();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, vec![7; 256 * 1024]);
    }

    #[tokio::test]
    async fn enforces_both_metadata_and_stream_limits() {
        let store = InMemory::new();
        let path = Path::from("image");
        store.put(&path, vec![1; 33].into()).await.unwrap();
        assert!(
            read_object(store.get(&path).await.unwrap(), 32)
                .await
                .is_err()
        );
        let mut dishonest = store.get(&path).await.unwrap();
        dishonest.meta.size = 1;
        assert!(read_object(dishonest, 32).await.is_err());
        assert_eq!(
            read_object(store.get(&path).await.unwrap(), 33)
                .await
                .unwrap()
                .len(),
            33
        );
    }

    #[tokio::test]
    async fn preserves_storage_errors_for_retry_classification() {
        let store = InMemory::new();
        let path = Path::from("image");
        store.put(&path, vec![1].into()).await.unwrap();
        let mut result = store.get(&path).await.unwrap();
        result.payload = object_store::GetResultPayload::Stream(
            futures::stream::once(async {
                Err(object_store::Error::Generic {
                    store: "test",
                    source: Box::new(std::io::Error::other("storage stream unavailable")),
                })
            })
            .boxed(),
        );
        let error = read_object(result, 32).await.unwrap_err();
        assert!(error.downcast::<object_store::Error>().is_ok());
    }
}
