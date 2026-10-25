//! Byte buffers held once per run and addressed by their content hash.
//!
//! A pin carrying bytes holds a handle `{"$bytes": "<blake3 hex>", "len": N}`
//! instead of one JSON number per byte, which costs 72 bytes per byte with
//! `serde_json/preserve_order` (an 8 MB photo became 583 MB per copy). Typed
//! reads resolve handles, so a handle deserializes exactly like the byte array
//! it stands for. Only raw reads (function bridges, overrides,
//! `evaluate_pin_raw`) see the handle itself.
//!
//! The store is a run resource: every context of a run shares it and it closes
//! with the run. Past a memory budget the least recently used buffers move to a
//! run-private temp directory, so a handle never dangles while its run is open.

use super::resources::RunResource;
use flow_like_types::json::Map;
use flow_like_types::{Bytes, Result, Value, anyhow, bail, tokio};
use parking_lot::Mutex;
use serde::de::{
    self, DeserializeOwned, DeserializeSeed, IntoDeserializer, MapAccess, SeqAccess, Visitor,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

pub const BYTES_HANDLE_KEY: &str = "$bytes";
const BYTES_LEN_KEY: &str = "len";
pub(crate) const BYTE_STORE_RESOURCE: &str = "flow_like::byte_store";
/// Buffers up to this size stay inline as JSON arrays.
pub const INLINE_BYTES_MAX: usize = 16 * 1024;
const MEMORY_LIMIT_ENV: &str = "FLOW_LIKE_RUN_BYTES_MEMORY_MB";
const DEFAULT_MEMORY_LIMIT: usize = 256 * 1024 * 1024;
const HASH_HEX_LEN: usize = 64;

/// The content hash `value` points at, when it is a byte handle.
pub fn handle_hash(value: &Value) -> Option<&str> {
    let map = value.as_object()?;
    if map.len() != 2 {
        return None;
    }
    map.get(BYTES_LEN_KEY)?.as_u64()?;
    let hash = map.get(BYTES_HANDLE_KEY)?.as_str()?;
    (hash.len() == HASH_HEX_LEN && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then_some(hash)
}

/// The length a byte handle records, without loading its buffer.
pub fn handle_len(value: &Value) -> Option<u64> {
    handle_hash(value)?;
    value.get(BYTES_LEN_KEY)?.as_u64()
}

fn handle_value(hash: &str, len: usize) -> Value {
    let mut map = Map::new();
    map.insert(BYTES_HANDLE_KEY.to_string(), Value::from(hash));
    map.insert(BYTES_LEN_KEY.to_string(), Value::from(len));
    Value::Object(map)
}

pub fn bytes_to_array(bytes: &[u8]) -> Value {
    Value::Array(bytes.iter().map(|byte| Value::from(*byte)).collect())
}

/// A plain JSON array of byte-sized numbers, as producers write `json!(bytes)`.
pub fn array_to_bytes(value: &Value) -> Option<Vec<u8>> {
    value
        .as_array()?
        .iter()
        .map(|item| item.as_u64().and_then(|number| u8::try_from(number).ok()))
        .collect()
}

fn memory_limit_from_env() -> usize {
    std::env::var(MEMORY_LIMIT_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|megabytes| *megabytes > 0)
        .map_or(DEFAULT_MEMORY_LIMIT, |megabytes| {
            megabytes.saturating_mul(1024 * 1024)
        })
}

struct Entry {
    memory: Option<Bytes>,
    disk: Option<PathBuf>,
    len: usize,
    last_used: u64,
    /// Written by a sensitive pin, so it never goes to disk.
    resident: bool,
}

#[derive(Default)]
struct State {
    entries: HashMap<String, Entry>,
    resident: usize,
    clock: u64,
    closed: bool,
    spill_dir: Option<tempfile::TempDir>,
}

impl State {
    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    fn spill_path(&mut self, hash: &str) -> Result<PathBuf> {
        if self.spill_dir.is_none() {
            self.spill_dir = Some(
                tempfile::Builder::new()
                    .prefix("flow-like-bytes-")
                    .tempdir()
                    .map_err(|e| anyhow!("Failed to create the run's byte spill directory: {e}"))?,
            );
        }
        Ok(self
            .spill_dir
            .as_ref()
            .map(|dir| dir.path().join(hash))
            .unwrap_or_default())
    }
}

pub struct ByteStore {
    state: Mutex<State>,
    memory_limit: usize,
    /// A child scope (one REST request, one socket message) reads through to its parent and
    /// frees its own buffers when it is dropped.
    parent: Option<Arc<ByteStore>>,
}

impl Default for ByteStore {
    fn default() -> Self {
        Self::with_memory_limit(memory_limit_from_env())
    }
}

enum SpillStep {
    Done,
    Drop(String),
    Write(String, Bytes, PathBuf),
}

impl ByteStore {
    pub fn with_memory_limit(memory_limit: usize) -> Self {
        Self {
            state: Mutex::new(State::default()),
            memory_limit,
            parent: None,
        }
    }

    /// A scope whose buffers live only as long as the returned store.
    pub fn child(self: &Arc<Self>) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State::default()),
            memory_limit: self.memory_limit,
            parent: Some(self.clone()),
        })
    }

    /// Store `bytes` and return the handle a pin carries; equal content shares one entry.
    pub async fn put(&self, bytes: Bytes) -> Result<Value> {
        self.insert(bytes, false).await
    }

    /// Like `put`, but the buffer is never written to the spill directory.
    pub async fn put_resident(&self, bytes: Bytes) -> Result<Value> {
        self.insert(bytes, true).await
    }

    async fn insert(&self, bytes: Bytes, resident: bool) -> Result<Value> {
        let hash = blake3::hash(&bytes).to_hex().to_string();
        self.insert_with_hash(hash, bytes, resident).await
    }

    /// Copy every buffer `value` refers to from this scope's chain into the run's root store,
    /// so the value stays readable after the scope ends (e.g. once written to a run variable).
    pub async fn promote(&self, value: &Value) -> Result<()> {
        let Some(mut root) = self.parent.clone() else {
            return Ok(());
        };
        while let Some(parent) = root.parent.clone() {
            root = parent;
        }
        let mut hashes = Vec::new();
        collect_handles(value, &mut hashes);
        for hash in hashes {
            if root.state.lock().entries.contains_key(hash) {
                continue;
            }
            let resident = self.is_resident_in_chain(hash);
            let bytes = self.get(hash).await?;
            root.insert_with_hash(hash.to_string(), bytes, resident)
                .await?;
        }
        Ok(())
    }

    fn is_resident_in_chain(&self, hash: &str) -> bool {
        let mut store = self;
        loop {
            if let Some(entry) = store.state.lock().entries.get(hash) {
                return entry.resident;
            }
            match &store.parent {
                Some(parent) => store = parent,
                None => return false,
            }
        }
    }

    async fn insert_with_hash(&self, hash: String, bytes: Bytes, resident: bool) -> Result<Value> {
        let len = bytes.len();
        {
            let mut state = self.state.lock();
            if state.closed {
                bail!("The run's byte store is closed");
            }
            let clock = state.tick();
            let State {
                entries,
                resident: resident_bytes,
                ..
            } = &mut *state;
            match entries.get_mut(&hash) {
                Some(entry) => {
                    entry.last_used = clock;
                    if resident && !entry.resident {
                        entry.resident = true;
                        if entry.memory.is_none() {
                            entry.memory = Some(bytes);
                            *resident_bytes += len;
                        }
                        if let Some(path) = entry.disk.take() {
                            let _ = std::fs::remove_file(path);
                        }
                    }
                }
                None => {
                    *resident_bytes += len;
                    entries.insert(
                        hash.clone(),
                        Entry {
                            memory: Some(bytes),
                            disk: None,
                            len,
                            last_used: clock,
                            resident,
                        },
                    );
                }
            }
        }
        self.spill_over_limit().await;
        Ok(handle_value(&hash, len))
    }

    pub async fn get(&self, hash: &str) -> Result<Bytes> {
        let mut store = self;
        loop {
            if let Some(bytes) = store.get_local(hash).await? {
                return Ok(bytes);
            }
            match &store.parent {
                Some(parent) => store = parent,
                None => bail!(
                    "Byte buffer {hash} is not in this run's store; it came from another run or \
                     from a request or message whose handler already finished"
                ),
            }
        }
    }

    async fn get_local(&self, hash: &str) -> Result<Option<Bytes>> {
        let path = {
            let mut state = self.state.lock();
            if state.closed {
                bail!("The run's byte store is closed, so byte buffer {hash} is gone");
            }
            let clock = state.tick();
            let Some(entry) = state.entries.get_mut(hash) else {
                return Ok(None);
            };
            entry.last_used = clock;
            match (&entry.memory, &entry.disk) {
                (Some(bytes), _) => return Ok(Some(bytes.clone())),
                (None, Some(path)) => path.clone(),
                (None, None) => bail!("Byte buffer {hash} has no data"),
            }
        };

        let bytes =
            Bytes::from(tokio::fs::read(&path).await.map_err(|e| {
                anyhow!("Failed to read spilled byte buffer {}: {e}", path.display())
            })?);
        {
            let mut state = self.state.lock();
            let State {
                entries, resident, ..
            } = &mut *state;
            if let Some(entry) = entries.get_mut(hash)
                && entry.memory.is_none()
            {
                entry.memory = Some(bytes.clone());
                *resident += entry.len;
            }
        }
        self.spill_over_limit().await;
        Ok(Some(bytes))
    }

    #[cfg(test)]
    pub(crate) fn is_resident(&self, hash: &str) -> bool {
        self.state
            .lock()
            .entries
            .get(hash)
            .is_some_and(|entry| entry.resident)
    }

    fn next_spill(&self) -> SpillStep {
        let mut state = self.state.lock();
        if state.closed || state.resident <= self.memory_limit {
            return SpillStep::Done;
        }
        let Some(hash) = state
            .entries
            .iter()
            .filter(|(_, entry)| entry.memory.is_some() && !entry.resident)
            .min_by_key(|(_, entry)| entry.last_used)
            .map(|(hash, _)| hash.clone())
        else {
            return SpillStep::Done;
        };
        let entry = &state.entries[&hash];
        if entry.disk.is_some() {
            return SpillStep::Drop(hash);
        }
        let Some(bytes) = entry.memory.clone() else {
            return SpillStep::Done;
        };
        match state.spill_path(&hash) {
            Ok(path) => SpillStep::Write(hash, bytes, path),
            Err(error) => {
                tracing::warn!(%error, "Keeping run byte buffers in memory");
                SpillStep::Done
            }
        }
    }

    fn release_memory(&self, hash: &str, disk: Option<PathBuf>) {
        let mut state = self.state.lock();
        let State {
            entries, resident, ..
        } = &mut *state;
        let Some(entry) = entries.get_mut(hash) else {
            return;
        };
        if entry.resident {
            if let Some(path) = disk {
                let _ = std::fs::remove_file(path);
            }
            return;
        }
        if let Some(path) = disk {
            entry.disk = Some(path);
        }
        if entry.disk.is_some() && entry.memory.take().is_some() {
            *resident -= entry.len;
        }
    }

    async fn spill_over_limit(&self) {
        loop {
            match self.next_spill() {
                SpillStep::Done => return,
                SpillStep::Drop(hash) => self.release_memory(&hash, None),
                SpillStep::Write(hash, bytes, path) => {
                    if let Err(error) = tokio::fs::write(&path, &bytes).await {
                        tracing::warn!(%error, path = %path.display(), "Keeping run byte buffers in memory");
                        return;
                    }
                    self.release_memory(&hash, Some(path));
                }
            }
        }
    }

    async fn resolve(&self, value: &Value) -> Result<Option<ResolvedBytes>> {
        let mut hashes = Vec::new();
        collect_handles(value, &mut hashes);
        if hashes.is_empty() {
            return Ok(None);
        }
        let mut resolved = HashMap::with_capacity(hashes.len());
        for hash in hashes {
            if !resolved.contains_key(hash) {
                resolved.insert(hash.to_string(), self.get(hash).await?);
            }
        }
        Ok(Some(ResolvedBytes(resolved)))
    }

    /// `value` with every handle replaced by the byte array it stands for.
    pub async fn materialize(&self, value: Value) -> Result<Value> {
        match self.resolve(&value).await? {
            Some(resolved) => Ok(resolved.materialize(value)),
            None => Ok(value),
        }
    }

    /// Deserialize `value`, reading each handle as the bytes it stands for.
    pub async fn deserialize<T: DeserializeOwned>(&self, value: Value) -> Result<T> {
        match self.resolve(&value).await? {
            Some(resolved) => Ok(T::deserialize(Resolved {
                value,
                bytes: &resolved,
            })?),
            None => Ok(flow_like_types::json::from_value(value)?),
        }
    }
}

/// The value a pin or struct field carries for `bytes`: a handle into `store`, or a plain
/// array when the buffer is small or there is no store.
pub(crate) async fn intern(store: Option<&Arc<ByteStore>>, bytes: Bytes, resident: bool) -> Value {
    if bytes.len() > INLINE_BYTES_MAX
        && let Some(store) = store
    {
        let stored = if resident {
            store.put_resident(bytes.clone()).await
        } else {
            store.put(bytes.clone()).await
        };
        match stored {
            Ok(handle) => return handle,
            Err(error) => tracing::warn!(%error, "Keeping a byte buffer inline"),
        }
    }
    bytes_to_array(&bytes)
}

/// Buffers made during one handler invocation inside a long-lived run (one REST request,
/// one socket message). They are freed once the scope and every context that entered it
/// are gone; values written to run variables are promoted to the run's store.
#[derive(Clone, Default)]
pub struct ByteScope(pub(crate) Option<Arc<ByteStore>>);

impl ByteScope {
    /// A nested scope, e.g. one per request of a server that holds this scope.
    pub fn child(&self) -> ByteScope {
        ByteScope(self.0.as_ref().map(|store| store.child()))
    }

    pub async fn bytes_value(&self, bytes: impl Into<Bytes>) -> Value {
        intern(self.0.as_ref(), bytes.into(), false).await
    }

    /// `value` with this scope's handles expanded, for what the handler hands back.
    pub async fn materialize(&self, value: Value) -> Result<Value> {
        match &self.0 {
            Some(store) => store.materialize(value).await,
            None => Ok(value),
        }
    }

    /// Keep the buffers `value` refers to readable after this scope ends.
    pub async fn promote(&self, value: &Value) -> Result<()> {
        match &self.0 {
            Some(store) => store.promote(value).await,
            None => Ok(()),
        }
    }
}

impl RunResource for ByteStore {
    fn abort(&self) {
        let mut state = self.state.lock();
        state.closed = true;
        state.entries.clear();
        state.resident = 0;
        state.spill_dir = None;
    }
}

fn collect_handles<'a>(value: &'a Value, out: &mut Vec<&'a str>) {
    if let Some(hash) = handle_hash(value) {
        out.push(hash);
        return;
    }
    match value {
        Value::Array(items) => items.iter().for_each(|item| collect_handles(item, out)),
        Value::Object(map) => map.values().for_each(|item| collect_handles(item, out)),
        _ => {}
    }
}

/// Buffers for every handle in one value, fetched before a synchronous deserialize.
struct ResolvedBytes(HashMap<String, Bytes>);

impl ResolvedBytes {
    fn bytes_for(&self, value: &Value) -> Option<&Bytes> {
        handle_hash(value).and_then(|hash| self.0.get(hash))
    }

    fn materialize(&self, value: Value) -> Value {
        if let Some(bytes) = self.bytes_for(&value) {
            return bytes_to_array(bytes);
        }
        match value {
            Value::Array(items) => Value::Array(
                items
                    .into_iter()
                    .map(|item| self.materialize(item))
                    .collect(),
            ),
            Value::Object(map) => Value::Object(
                map.into_iter()
                    .map(|(key, item)| (key, self.materialize(item)))
                    .collect(),
            ),
            other => other,
        }
    }
}

/// A `Value` whose handles read as the byte sequences they stand for.
struct Resolved<'a> {
    value: Value,
    bytes: &'a ResolvedBytes,
}

type JsonError = serde_json::Error;

fn visit_byte_seq<'de, V: Visitor<'de>>(bytes: &Bytes, visitor: V) -> Result<V::Value, JsonError> {
    let mut seq = de::value::SeqDeserializer::<_, JsonError>::new(bytes.iter().copied());
    let value = visitor.visit_seq(&mut seq)?;
    seq.end()?;
    Ok(value)
}

fn visit_array<'de, V: Visitor<'de>>(
    items: Vec<Value>,
    bytes: &ResolvedBytes,
    visitor: V,
) -> Result<V::Value, JsonError> {
    let len = items.len();
    let mut seq = ResolvedSeq {
        items: items.into_iter(),
        bytes,
    };
    let value = visitor.visit_seq(&mut seq)?;
    if seq.items.len() == 0 {
        Ok(value)
    } else {
        Err(de::Error::invalid_length(len, &"fewer elements in array"))
    }
}

fn visit_object<'de, V: Visitor<'de>>(
    map: Map<String, Value>,
    bytes: &ResolvedBytes,
    visitor: V,
) -> Result<V::Value, JsonError> {
    let len = map.len();
    let mut access = ResolvedMap {
        entries: map.into_iter(),
        value: None,
        bytes,
    };
    let value = visitor.visit_map(&mut access)?;
    if access.entries.len() == 0 {
        Ok(value)
    } else {
        Err(de::Error::invalid_length(len, &"fewer elements in map"))
    }
}

impl<'de> de::Deserializer<'de> for Resolved<'_> {
    type Error = JsonError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        if let Some(bytes) = self.bytes.bytes_for(&self.value) {
            return visit_byte_seq(bytes, visitor);
        }
        match self.value {
            Value::Array(items) => visit_array(items, self.bytes, visitor),
            Value::Object(map) => visit_object(map, self.bytes, visitor),
            other => other.deserialize_any(visitor),
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.value {
            Value::Null => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_bytes<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.bytes.bytes_for(&self.value) {
            Some(bytes) => visitor.visit_bytes(bytes),
            None => self.deserialize_any(visitor),
        }
    }

    fn deserialize_byte_buf<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.bytes.bytes_for(&self.value) {
            Some(bytes) => visitor.visit_byte_buf(bytes.to_vec()),
            None => self.deserialize_any(visitor),
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.bytes
            .materialize(self.value)
            .deserialize_enum(name, variants, visitor)
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_unit()
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        unit unit_struct seq tuple tuple_struct map struct identifier
    }
}

struct ResolvedSeq<'a> {
    items: std::vec::IntoIter<Value>,
    bytes: &'a ResolvedBytes,
}

impl<'de> SeqAccess<'de> for ResolvedSeq<'_> {
    type Error = JsonError;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Self::Error> {
        self.items
            .next()
            .map(|value| {
                seed.deserialize(Resolved {
                    value,
                    bytes: self.bytes,
                })
            })
            .transpose()
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.items.len())
    }
}

struct ResolvedMap<'a> {
    entries: serde_json::map::IntoIter,
    value: Option<Value>,
    bytes: &'a ResolvedBytes,
}

impl<'de> MapAccess<'de> for ResolvedMap<'_> {
    type Error = JsonError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Self::Error> {
        let Some((key, value)) = self.entries.next() else {
            return Ok(None);
        };
        self.value = Some(value);
        seed.deserialize(<String as IntoDeserializer<'de, JsonError>>::into_deserializer(key))
            .map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> Result<V::Value, Self::Error> {
        let value = self
            .value
            .take()
            .ok_or_else(|| <JsonError as de::Error>::custom("map value is missing"))?;
        seed.deserialize(Resolved {
            value,
            bytes: self.bytes,
        })
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.entries.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::json::json;
    use serde::Deserialize;

    fn sample(len: usize, seed: u8) -> Bytes {
        Bytes::from(
            (0..len)
                .map(|index| (index as u8).wrapping_mul(31).wrapping_add(seed))
                .collect::<Vec<u8>>(),
        )
    }

    #[tokio::test]
    async fn equal_content_shares_one_handle() {
        let store = ByteStore::with_memory_limit(usize::MAX);
        let first = store.put(sample(64 * 1024, 1)).await.unwrap();
        let second = store.put(sample(64 * 1024, 1)).await.unwrap();
        assert_eq!(first, second);
        assert_eq!(handle_len(&first), Some(64 * 1024));
        assert_eq!(store.state.lock().resident, 64 * 1024);
    }

    #[tokio::test]
    async fn handles_read_like_the_arrays_they_replace() {
        #[derive(Deserialize)]
        struct Envelope {
            name: String,
            data: Vec<u8>,
            tail: Option<Vec<u8>>,
        }

        let store = ByteStore::with_memory_limit(usize::MAX);
        let bytes = sample(20_000, 7);
        let handle = store.put(bytes.clone()).await.unwrap();

        let typed: Vec<u8> = store.deserialize(handle.clone()).await.unwrap();
        assert_eq!(typed, bytes.to_vec());

        let generic: Vec<Value> = store.deserialize(handle.clone()).await.unwrap();
        assert_eq!(generic.len(), bytes.len());
        assert_eq!(generic[3], json!(bytes[3]));

        let value: Value = store.deserialize(handle.clone()).await.unwrap();
        assert_eq!(value, bytes_to_array(&bytes));

        let envelope: Envelope = store
            .deserialize(json!({ "name": "photo", "data": handle, "tail": handle }))
            .await
            .unwrap();
        assert_eq!(envelope.name, "photo");
        assert_eq!(envelope.data, bytes.to_vec());
        assert_eq!(envelope.tail.as_deref(), Some(&bytes[..]));

        let materialized = store
            .materialize(json!({ "nested": [handle, 1] }))
            .await
            .unwrap();
        assert_eq!(
            materialized,
            json!({ "nested": [bytes_to_array(&bytes), 1] })
        );
    }

    #[tokio::test]
    async fn values_without_handles_deserialize_unchanged() {
        let store = ByteStore::with_memory_limit(usize::MAX);
        let value = json!({ "$bytes": "not-a-hash", "len": 3, "extra": true });
        assert!(handle_hash(&value).is_none());
        let read: Value = store.deserialize(value.clone()).await.unwrap();
        assert_eq!(read, value);
        let numbers: Vec<u8> = store.deserialize(json!([1, 2, 3])).await.unwrap();
        assert_eq!(numbers, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn buffers_over_the_budget_spill_and_come_back() {
        let store = ByteStore::with_memory_limit(100 * 1024);
        let first = sample(80 * 1024, 1);
        let second = sample(80 * 1024, 2);
        let first_handle = store.put(first.clone()).await.unwrap();
        let second_handle = store.put(second.clone()).await.unwrap();
        {
            let state = store.state.lock();
            assert!(state.resident <= 100 * 1024);
            let spilled = &state.entries[handle_hash(&first_handle).unwrap()];
            assert!(spilled.memory.is_none() && spilled.disk.is_some());
        }

        let read_first: Vec<u8> = store.deserialize(first_handle).await.unwrap();
        assert_eq!(read_first, first.to_vec());
        let read_second: Vec<u8> = store.deserialize(second_handle).await.unwrap();
        assert_eq!(read_second, second.to_vec());
        assert!(store.state.lock().resident <= 100 * 1024);
    }

    #[tokio::test]
    async fn a_closed_store_refuses_reads_and_removes_its_spill_files() {
        let store = ByteStore::with_memory_limit(1);
        let handle = store.put(sample(32 * 1024, 3)).await.unwrap();
        let dir = store
            .state
            .lock()
            .spill_dir
            .as_ref()
            .map(|dir| dir.path().to_path_buf())
            .unwrap();
        assert!(dir.exists());

        store.abort();
        assert!(!dir.exists());
        assert!(store.deserialize::<Vec<u8>>(handle).await.is_err());
        assert!(store.put(sample(32 * 1024, 4)).await.is_err());
    }

    #[tokio::test]
    async fn resident_buffers_never_reach_the_spill_directory() {
        let store = ByteStore::with_memory_limit(1);
        let secret = sample(32 * 1024, 5);
        let handle = store.put_resident(secret.clone()).await.unwrap();
        store.put(sample(32 * 1024, 6)).await.unwrap();
        {
            let state = store.state.lock();
            let entry = &state.entries[handle_hash(&handle).unwrap()];
            assert!(entry.memory.is_some() && entry.disk.is_none());
        }

        let spilled = store.put(sample(32 * 1024, 7)).await.unwrap();
        let promoted = store.put_resident(sample(32 * 1024, 7)).await.unwrap();
        assert_eq!(spilled, promoted);
        let state = store.state.lock();
        let entry = &state.entries[handle_hash(&promoted).unwrap()];
        assert!(entry.memory.is_some() && entry.disk.is_none());
    }

    #[tokio::test]
    async fn a_scope_reads_through_and_frees_its_own_buffers() {
        let root = Arc::new(ByteStore::with_memory_limit(usize::MAX));
        let shared = root.put(sample(32 * 1024, 1)).await.unwrap();

        let scope = root.child();
        let request = scope.put(sample(32 * 1024, 2)).await.unwrap();
        let kept = scope.put(sample(32 * 1024, 3)).await.unwrap();
        assert_eq!(
            scope.deserialize::<Vec<u8>>(shared.clone()).await.unwrap(),
            sample(32 * 1024, 1).to_vec()
        );
        assert!(root.deserialize::<Vec<u8>>(request.clone()).await.is_err());

        scope.promote(&json!({ "kept": kept })).await.unwrap();
        drop(scope);

        assert_eq!(
            root.deserialize::<Vec<u8>>(kept).await.unwrap(),
            sample(32 * 1024, 3).to_vec()
        );
        let missing = root.deserialize::<Vec<u8>>(request).await.unwrap_err();
        assert!(
            missing.to_string().contains("already finished"),
            "{missing}"
        );
    }

    #[test]
    fn only_byte_sized_arrays_convert() {
        assert_eq!(array_to_bytes(&json!([0, 255, 7])), Some(vec![0, 255, 7]));
        assert_eq!(array_to_bytes(&json!([0, 256])), None);
        assert_eq!(array_to_bytes(&json!([0, -1])), None);
        assert_eq!(array_to_bytes(&json!("text")), None);
    }
}
