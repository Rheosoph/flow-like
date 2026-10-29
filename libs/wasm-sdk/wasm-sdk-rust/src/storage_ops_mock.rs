//! Native in-memory storage for testing code that calls `Storage`.
pub use super::StorageError;
use super::{FileStat, ListPage, ListRequest, StorageResult, WriteOptions};
use crate::FlowPath;
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, VecDeque},
};

const MAX_BYTES: usize = 10 * 1024 * 1024;
#[derive(Default, Debug)]
pub struct MockStorage {
    files: BTreeMap<(String, String), (Vec<u8>, FileStat)>,
    writes: HashMap<String, (FlowPath, Option<u64>, Vec<u8>)>,
    listings: HashMap<String, (String, VecDeque<FileStat>)>,
    sequence: u64,
}

impl MockStorage {
    pub fn bytes(&self, path: &FlowPath) -> Option<&[u8]> {
        self.files
            .get(&key(path))
            .map(|(bytes, _)| bytes.as_slice())
    }
    pub fn pending_writes(&self) -> usize {
        self.writes.len()
    }
    pub fn open_listings(&self) -> usize {
        self.listings.len()
    }
    fn next_id(&mut self) -> String {
        self.sequence += 1;
        format!("mock-{}", self.sequence)
    }
    fn put(&mut self, path: FlowPath, data: Vec<u8>) -> FileStat {
        let tag = self.next_id();
        let stat = FileStat {
            path: path.clone(),
            size: data.len() as u64,
            e_tag: Some(tag.clone()),
            version: Some(tag),
            last_modified: "1970-01-01T00:00:00Z".into(),
        };
        self.files.insert(key(&path), (data, stat.clone()));
        stat
    }
    fn stat(&self, path: &FlowPath) -> StorageResult<FileStat> {
        self.files
            .get(&key(path))
            .map(|(_, stat)| stat.clone())
            .ok_or_else(|| StorageError::new("not_found", "File does not exist"))
    }
}

thread_local! { static ACTIVE: RefCell<Option<MockStorage>> = const { RefCell::new(None) }; }

/// Install a thread-local mock for this closure and restore any enclosing mock on return or panic.
pub fn with_mock<T>(storage: MockStorage, run: impl FnOnce() -> T) -> (T, MockStorage) {
    struct Restore(Option<MockStorage>);
    impl Drop for Restore {
        fn drop(&mut self) {
            ACTIVE.with(|active| {
                *active.borrow_mut() = self.0.take();
            });
        }
    }
    let previous = ACTIVE.with(|active| active.replace(Some(storage)));
    let restore = Restore(previous);
    let result = run();
    let storage = ACTIVE.with(|active| active.take().expect("mock is installed"));
    drop(restore);
    (result, storage)
}
fn with<T>(call: impl FnOnce(&mut MockStorage) -> StorageResult<T>) -> StorageResult<T> {
    ACTIVE.with(|active| {
        let mut value = active.borrow_mut();
        call(value.as_mut().ok_or_else(|| {
            StorageError::new("unsupported", "No native storage mock is installed")
        })?)
    })
}
fn key(path: &FlowPath) -> (String, String) {
    (path.store_ref.clone(), path.path.clone())
}
fn parse<T: serde::de::DeserializeOwned>(value: &str) -> StorageResult<T> {
    serde_json::from_str(value)
        .map_err(|error| StorageError::new("invalid_argument", error.to_string()))
}
fn json(value: &impl serde::Serialize) -> StorageResult<String> {
    super::encode(value)
}
fn conflict() -> StorageError {
    StorageError::new("conflict", "File changed or destination already exists")
}

pub fn stat_file(path: &str) -> StorageResult<String> {
    with(|mock| json(&mock.stat(&parse(path)?)?))
}
pub fn read_file_range(
    path: &str,
    offset: u64,
    length: u64,
    if_match: Option<&str>,
) -> StorageResult<Vec<u8>> {
    offset
        .checked_add(length)
        .ok_or_else(|| StorageError::new("invalid_argument", "Read range overflows u64"))?;
    if length > MAX_BYTES as u64 {
        return Err(StorageError::new("resource_limit", "Range exceeds 10 MiB"));
    }
    with(|mock| {
        let path = parse(path)?;
        let stat = mock.stat(&path)?;
        if if_match.is_some() && if_match != stat.e_tag.as_deref() {
            return Err(conflict());
        }
        let data = mock.bytes(&path).expect("stat checked existence");
        if offset > data.len() as u64 {
            return Err(StorageError::new(
                "invalid_argument",
                "Read offset is beyond EOF",
            ));
        }
        let start = offset.min(data.len() as u64) as usize;
        let end = offset.saturating_add(length).min(data.len() as u64) as usize;
        Ok(data[start..end].to_vec())
    })
}
pub fn list_files_page(request: &str) -> StorageResult<String> {
    with(|mock| {
        let mut request: ListRequest = parse(request)?;
        let limit = request.limit.unwrap_or(100) as usize;
        if limit == 0 || limit > 1000 {
            return Err(StorageError::new(
                "invalid_argument",
                "Page limit must be between 1 and 1000",
            ));
        }
        let cursor = request.cursor.take();
        request.limit = None;
        request.exclude_directories.sort();
        request.exclude_directories.dedup();
        let binding = json(&request)?;
        let (cursor, mut entries) = if let Some(cursor) = cursor {
            let (expected, entries) = mock.listings.remove(&cursor).ok_or_else(|| {
                StorageError::new("invalid_cursor", "Listing is closed or expired")
            })?;
            if expected != binding {
                mock.listings.insert(cursor, (expected, entries));
                return Err(StorageError::new(
                    "invalid_cursor",
                    "Listing options changed",
                ));
            }
            (cursor, entries)
        } else {
            if mock.listings.len() >= 32 {
                return Err(StorageError::new(
                    "resource_limit",
                    "Too many open listings",
                ));
            }
            let prefix = request.path.path.trim_end_matches('/');
            let entries = mock
                .files
                .values()
                .filter_map(|(_, stat)| {
                    if stat.path.store_ref != request.path.store_ref {
                        return None;
                    }
                    let relative = if prefix.is_empty() {
                        stat.path.path.as_str()
                    } else {
                        stat.path.path.strip_prefix(&format!("{prefix}/"))?
                    };
                    let directories: Vec<_> = relative.split('/').collect();
                    if (!request.recursive && directories.len() > 1)
                        || directories[..directories.len().saturating_sub(1)]
                            .iter()
                            .any(|part| {
                                request
                                    .exclude_directories
                                    .iter()
                                    .any(|excluded| excluded == part)
                            })
                    {
                        return None;
                    }
                    Some(stat.clone())
                })
                .collect();
            (mock.next_id(), entries)
        };
        let page = entries.drain(..limit.min(entries.len())).collect();
        let cursor = if entries.is_empty() {
            None
        } else {
            mock.listings.insert(cursor.clone(), (binding, entries));
            Some(cursor)
        };
        json(&ListPage {
            entries: page,
            cursor,
        })
    })
}
pub fn close_list(cursor: &str) -> StorageResult<()> {
    with(|mock| {
        mock.listings
            .remove(cursor)
            .map(|_| ())
            .ok_or_else(|| StorageError::new("invalid_cursor", "Listing is closed or expired"))
    })
}
pub fn write_file(path: &str, data: &[u8], options: &str) -> StorageResult<String> {
    if data.len() > MAX_BYTES {
        return Err(StorageError::new(
            "resource_limit",
            "Write exceeds 10 MiB; use a writer",
        ));
    }
    with(|mock| {
        let path: FlowPath = parse(path)?;
        let options: WriteOptions = parse(options)?;
        if options.if_match.as_deref() == Some("") || options.version.as_deref() == Some("") {
            return Err(StorageError::new(
                "invalid_argument",
                "Write preconditions cannot be empty",
            ));
        }
        if options.create_only && (options.if_match.is_some() || options.version.is_some()) {
            return Err(StorageError::new(
                "invalid_argument",
                "create_only cannot be combined with a precondition",
            ));
        }
        let existing = mock.stat(&path).ok();
        if options.create_only && existing.is_some()
            || options.if_match.is_some()
                && options.if_match.as_deref() != existing.as_ref().and_then(|s| s.e_tag.as_deref())
            || options.version.is_some()
                && options.version.as_deref()
                    != existing.as_ref().and_then(|s| s.version.as_deref())
        {
            return Err(conflict());
        }
        json(&mock.put(path, data.to_vec()))
    })
}
pub fn write_file_start(path: &str, total_size: Option<u64>) -> StorageResult<String> {
    with(|mock| {
        if mock.writes.len() >= 8 {
            return Err(StorageError::new("resource_limit", "Too many open writers"));
        }
        let path = parse(path)?;
        let id = mock.next_id();
        mock.writes
            .insert(id.clone(), (path, total_size, Vec::new()));
        Ok(id)
    })
}
pub fn write_file_chunk(id: &str, data: &[u8]) -> StorageResult<()> {
    with(|mock| {
        let (_, total, buffer) = mock
            .writes
            .get_mut(id)
            .ok_or_else(|| StorageError::new("invalid_argument", "Unknown write handle"))?;
        let error = if data.len() > MAX_BYTES {
            Some(StorageError::new("resource_limit", "Chunk exceeds 10 MiB"))
        } else if total
            .is_some_and(|size| (buffer.len() as u64).saturating_add(data.len() as u64) > size)
        {
            Some(StorageError::new(
                "incomplete_write",
                "Declared size exceeded",
            ))
        } else {
            None
        };
        if let Some(error) = error {
            mock.writes.remove(id);
            return Err(error);
        }
        buffer.extend_from_slice(data);
        Ok(())
    })
}
pub fn write_file_finish(id: &str) -> StorageResult<String> {
    with(|mock| {
        let (path, total, data) = mock
            .writes
            .remove(id)
            .ok_or_else(|| StorageError::new("invalid_argument", "Unknown write handle"))?;
        if total.is_some_and(|size| size != data.len() as u64) {
            return Err(StorageError::new(
                "incomplete_write",
                "Declared size was not written",
            ));
        }
        json(&mock.put(path, data))
    })
}
pub fn write_file_abort(id: &str) -> StorageResult<()> {
    with(|mock| {
        mock.writes.remove(id);
        Ok(())
    })
}
pub fn delete_file(path: &str) -> StorageResult<()> {
    with(|mock| {
        mock.files.remove(&key(&parse(path)?));
        Ok(())
    })
}
pub fn copy_file(source: &str, destination: &str, overwrite: bool) -> StorageResult<()> {
    with(|mock| {
        let source = parse(source)?;
        let destination: FlowPath = parse(destination)?;
        mock.stat(&source)?;
        if key(&source) == key(&destination) {
            return Ok(());
        }
        if source.store_ref != destination.store_ref
            && !overwrite
            && mock.bytes(&source).unwrap().len() > MAX_BYTES
        {
            return Err(StorageError::new(
                "unsupported",
                "Create-only cross-store copies are limited to 10 MiB",
            ));
        }
        if !overwrite && mock.files.contains_key(&key(&destination)) {
            return Err(conflict());
        }
        let data = mock.bytes(&source).unwrap().to_vec();
        mock.put(destination, data);
        Ok(())
    })
}
pub fn move_file(source: &str, destination: &str, overwrite: bool) -> StorageResult<()> {
    let source_path: FlowPath = parse(source)?;
    let destination_path: FlowPath = parse(destination)?;
    if key(&source_path) == key(&destination_path) {
        return with(|mock| mock.stat(&source_path).map(|_| ()));
    }
    copy_file(source, destination, overwrite)?;
    delete_file(source)
}
pub fn file_url(path: &str) -> StorageResult<Option<String>> {
    with(|mock| {
        mock.stat(&parse(path)?)?;
        Ok(None)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage_ops::Storage;
    fn path(name: &str) -> FlowPath {
        FlowPath::new(name.into(), "mock".into(), None)
    }
    #[test]
    fn conditional_writes_and_range_reads_detect_conflicts() {
        with_mock(MockStorage::default(), || {
            let file = path("project/a");
            let stat = Storage
                .write(
                    &file,
                    b"abcdef",
                    &WriteOptions {
                        create_only: true,
                        ..Default::default()
                    },
                )
                .unwrap();
            assert_eq!(
                Storage
                    .read_range(&file, 1, 3, stat.e_tag.as_deref())
                    .unwrap(),
                b"bcd"
            );
            assert_eq!(
                Storage
                    .write(
                        &file,
                        b"x",
                        &WriteOptions {
                            create_only: true,
                            ..Default::default()
                        }
                    )
                    .unwrap_err()
                    .code,
                "conflict"
            );
            Storage
                .write(
                    &file,
                    b"new",
                    &WriteOptions {
                        if_match: stat.e_tag.clone(),
                        ..Default::default()
                    },
                )
                .unwrap();
            assert_eq!(
                Storage
                    .read_range(&file, 0, 1, stat.e_tag.as_deref())
                    .unwrap_err()
                    .code,
                "conflict"
            );
        });
    }
    #[test]
    fn streaming_finish_is_exact_and_drop_aborts() {
        let (_, mock) = with_mock(MockStorage::default(), || {
            let file = path("a");
            let mut writer = Storage.writer(&file, Some(4)).unwrap();
            writer.write_chunk(b"ab").unwrap();
            assert_eq!(writer.finish().unwrap_err().code, "incomplete_write");
            assert_eq!(Storage.stat(&file).unwrap_err().code, "not_found");
            let mut writer = Storage.writer(&file, None).unwrap();
            writer.write_chunk(b"ab").unwrap();
            writer.write_chunk(b"cd").unwrap();
            assert_eq!(writer.finish().unwrap().size, 4);
            let _abandoned = Storage.writer(&path("abandoned"), Some(20)).unwrap();
        });
        assert_eq!(mock.pending_writes(), 0);
        assert_eq!(mock.bytes(&path("a")), Some(b"abcd".as_slice()));
    }
    #[test]
    fn pagination_filters_scopes_and_closes_cursors() {
        let (_, mock) = with_mock(MockStorage::default(), || {
            for file in [
                "project/a",
                "project/b",
                "project/node_modules/c",
                "project-other/a",
            ] {
                Storage
                    .write(&path(file), b"a", &WriteOptions::default())
                    .unwrap();
            }
            let mut request = ListRequest::new(path("project"));
            request.limit = Some(1);
            request.exclude_directories.push("node_modules".into());
            let first = Storage.list_page(&request).unwrap();
            assert_eq!(first.entries[0].path.path, "project/a");
            request.cursor = first.cursor;
            let second = Storage.list_page(&request).unwrap();
            assert_eq!(second.entries[0].path.path, "project/b");
            assert!(second.cursor.is_none());
            assert_eq!(
                Storage.list_page(&request).unwrap_err().code,
                "invalid_cursor"
            );
        });
        assert_eq!(mock.open_listings(), 0);
    }
}
