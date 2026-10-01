//! Per-scope index of cloud object revisions and complete listings (design §4.7).

use anyhow::{Context, Result};
use flow_like::flow_like_storage::object_store::{ObjectMeta, path::Path as ObjectPath};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::Path,
    sync::{Mutex, MutexGuard},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const INDEX_FILE: &str = "objects.sqlite";
const MAX_OBJECTS: i64 = 200_000;
const MAX_INDEX_BYTES: i64 = 64 * 1024 * 1024;
const EVICTION_BATCH: i64 = 1000;
/// Complete listings above this many entry bytes are not recorded.
pub(crate) const MAX_LISTING_BYTES: usize = 4 * 1024 * 1024;
/// A row that only carries a queued-operation mark; its cloud state is unknown.
const QUEUED_ONLY: i64 = -1;
const BOUNDS_CHECK_EVERY: u32 = 256;

/// How a key relates to the run's app and account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeyClass {
    /// `apps/{app}/upload/…`, `apps/{app}/storage/…` or the run account's user folder, outside `db/`.
    Content,
    /// The project or user database of this app.
    Database { user: bool },
    /// Another app's upload or storage folder, a folder name a case-insensitive file system
    /// resolves to one, an ancestor of content roots (`apps`, `apps/{app}`), or any other
    /// `users/…` key.
    Foreign,
    /// Desktop-local app metadata under `apps/…` (boards, `metadata/`, templates).
    Metadata,
    /// Any other top-level key.
    Outside,
}

/// The content roots of one app and account.
#[derive(Clone, Debug)]
pub(crate) struct ContentRoots {
    app: String,
    user: Option<String>,
}

impl ContentRoots {
    pub(crate) fn new(app_id: &str, subject: Option<&str>) -> Self {
        Self {
            app: ObjectPath::from("apps").join(app_id).to_string(),
            user: subject.map(|subject| {
                ObjectPath::from("users")
                    .join(subject)
                    .join("apps")
                    .join(app_id)
                    .to_string()
            }),
        }
    }

    pub(crate) fn classify(&self, key: &str) -> KeyClass {
        if let Some(rest) = below(key, &self.app) {
            let (first, tail) = split_first(rest);
            return match first {
                "upload" => KeyClass::Content,
                "storage" if split_first(tail).0 == "db" => KeyClass::Database { user: false },
                "storage" => KeyClass::Content,
                _ if first.is_empty() || content_folder(first) => KeyClass::Foreign,
                _ => KeyClass::Metadata,
            };
        }
        if let Some(rest) = self.user.as_deref().and_then(|user| below(key, user)) {
            return if split_first(rest).0 == "db" {
                KeyClass::Database { user: true }
            } else {
                KeyClass::Content
            };
        }
        let mut parts = key.split('/');
        match parts.next() {
            Some("apps") => match parts.nth(1) {
                Some(folder) if !content_folder(folder) => KeyClass::Metadata,
                _ => KeyClass::Foreign,
            },
            Some("users") => KeyClass::Foreign,
            _ => KeyClass::Outside,
        }
    }

    pub(crate) fn is_content(&self, key: &str) -> bool {
        self.classify(key) == KeyClass::Content
    }

    /// True for keys of the run account's user folder.
    pub(crate) fn is_user(&self, key: &str) -> bool {
        self.user
            .as_deref()
            .is_some_and(|user| below(key, user).is_some())
    }
}

fn below<'a>(key: &'a str, root: &str) -> Option<&'a str> {
    if key == root {
        return Some("");
    }
    key.strip_prefix(root)?.strip_prefix('/')
}

fn split_first(rest: &str) -> (&str, &str) {
    rest.split_once('/').unwrap_or((rest, ""))
}

/// True for folder names the local file system may resolve to an `upload` or `storage`
/// folder: case-insensitive volumes fold case (upper-casing first also folds 'ſ' to 's'),
/// and Windows drops trailing dots and spaces.
fn content_folder(segment: &str) -> bool {
    let folded = segment
        .trim_end_matches(['.', ' '])
        .to_uppercase()
        .to_lowercase();
    matches!(folded.as_str(), "upload" | "storage")
}

fn parent(key: &str) -> Option<&str> {
    key.rsplit_once('/').map(|(parent, _)| parent)
}

/// The strict ancestors of `key`, nearest first.
fn ancestors(key: &str) -> impl Iterator<Item = &str> {
    std::iter::successors(parent(key), |&prefix| parent(prefix))
}

/// The key range strictly below `prefix`: `prefix/` up to `prefix0`, '0' being the code point
/// after '/'. Unlike SQLite's LIKE it is case-sensitive.
fn subtree(prefix: &str) -> (String, String) {
    (format!("{prefix}/"), format!("{prefix}0"))
}

/// What the device can prove about a key.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Known {
    Present(ObjectMeta),
    Absent,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Entry {
    k: String,
    s: u64,
    m: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    e: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    v: Option<String>,
}

impl Entry {
    fn of(meta: &ObjectMeta) -> Self {
        Self {
            k: meta.location.to_string(),
            s: meta.size,
            m: meta.last_modified.timestamp_millis(),
            e: meta.e_tag.clone(),
            v: meta.version.clone(),
        }
    }

    fn meta(&self) -> Result<ObjectMeta> {
        Ok(ObjectMeta {
            location: ObjectPath::parse(&self.k)?,
            last_modified: millis_to_time(self.m),
            size: self.s,
            e_tag: self.e.clone(),
            version: self.v.clone(),
        })
    }
}

fn millis_to_time(millis: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp_millis(millis).unwrap_or_default()
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs() as i64
}

/// A stored listing snapshot by object key.
type Listing = BTreeMap<String, Entry>;

pub(crate) struct ObjectIndex {
    db: Mutex<Connection>,
    roots: ContentRoots,
    writes: std::sync::atomic::AtomicU32,
    max_objects: i64,
}

impl ObjectIndex {
    /// Opens `<dir>/objects.sqlite` (WAL, 0600), creating it when absent.
    pub(crate) fn open(dir: &Path, roots: ContentRoots) -> Result<Self> {
        let path = dir.join(INDEX_FILE);
        create_private_file(&path)?;
        let db = Connection::open(&path).with_context(|| {
            format!(
                "Could not open the offline object index at {}",
                path.display()
            )
        })?;
        db.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))?;
        db.pragma_update(None, "synchronous", "NORMAL")?;
        db.busy_timeout(Duration::from_secs(5))?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS objects(
               key TEXT PRIMARY KEY, present INTEGER NOT NULL, size INTEGER, last_modified INTEGER,
               e_tag TEXT, version TEXT, queued_operation TEXT, observed_at INTEGER NOT NULL);
             CREATE INDEX IF NOT EXISTS objects_observed ON objects(observed_at);
             CREATE TABLE IF NOT EXISTS listings(prefix TEXT PRIMARY KEY, entries BLOB NOT NULL, observed_at INTEGER NOT NULL);",
        )?;
        Ok(Self {
            db: Mutex::new(db),
            roots,
            writes: std::sync::atomic::AtomicU32::new(0),
            max_objects: MAX_OBJECTS,
        })
    }

    #[cfg(test)]
    pub(crate) fn with_max_objects(mut self, max_objects: i64) -> Self {
        self.max_objects = max_objects;
        self
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        self.db
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Provider metadata seen online (HEAD/GET/PUT results, listing entries, acknowledged replays).
    pub(crate) fn observe_present(&self, meta: &ObjectMeta) -> Result<()> {
        self.observe_present_all(std::slice::from_ref(meta))
    }

    /// Provider metadata of several objects (one level of a listing), in one transaction.
    pub(crate) fn observe_present_all(&self, metas: &[ObjectMeta]) -> Result<()> {
        let entries: Vec<Entry> = metas
            .iter()
            .filter(|meta| self.roots.is_content(meta.location.as_ref()))
            .map(Entry::of)
            .collect();
        if entries.is_empty() {
            return Ok(());
        }
        {
            let mut db = self.lock();
            let tx = db.transaction()?;
            let at = now();
            let mut snapshots = Snapshots::default();
            for entry in &entries {
                let unchanged = stored(&tx, &entry.k)?
                    .is_some_and(|(present, row)| present == 1 && row == *entry);
                if !unchanged {
                    snapshots.apply(&tx, &entry.k, Some(entry))?;
                }
                upsert_present(&tx, entry, at)?;
            }
            snapshots.save(&tx)?;
            tx.commit()?;
        }
        self.after_write()
    }

    pub(crate) fn observe_absent(&self, key: &ObjectPath) -> Result<()> {
        let key = key.to_string();
        if !self.roots.is_content(&key) {
            return Ok(());
        }
        {
            let mut db = self.lock();
            let tx = db.transaction()?;
            if !stored(&tx, &key)?.is_some_and(|(present, _)| present == 0) {
                let mut snapshots = Snapshots::default();
                snapshots.apply(&tx, &key, None)?;
                snapshots.save(&tx)?;
            }
            tx.execute(
                "INSERT INTO objects(key,present,size,last_modified,e_tag,version,observed_at) VALUES(?1,0,NULL,NULL,NULL,NULL,?2)
                 ON CONFLICT(key) DO UPDATE SET present=0,size=NULL,last_modified=NULL,e_tag=NULL,version=NULL,observed_at=?2",
                params![key, now()],
            )?;
            tx.commit()?;
        }
        self.after_write()
    }

    /// The key exists with a revision this device does not know (a copy or rename target).
    pub(crate) fn forget(&self, key: &ObjectPath) -> Result<()> {
        let key = key.to_string();
        if !self.roots.is_content(&key) {
            return Ok(());
        }
        let mut db = self.lock();
        let tx = db.transaction()?;
        tx.execute(
            "DELETE FROM objects WHERE key=?1 AND queued_operation IS NULL",
            [&key],
        )?;
        tx.execute(
            "UPDATE objects SET present=?2,size=NULL,last_modified=NULL,e_tag=NULL,version=NULL WHERE key=?1",
            params![key, QUEUED_ONLY],
        )?;
        for prefix in ancestors(&key) {
            tx.execute("DELETE FROM listings WHERE prefix=?1", [prefix])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// A complete recursive listing; replaces the prefix's snapshot and upserts its entries.
    /// Older snapshots below the prefix are dropped, and snapshots above it take the new
    /// listing's contents for the prefix.
    pub(crate) fn record_listing(&self, prefix: &ObjectPath, entries: &[ObjectMeta]) -> Result<()> {
        let prefix = prefix.to_string();
        if !self.roots.is_content(&prefix) {
            return Ok(());
        }
        let listed: Listing = entries
            .iter()
            .filter(|meta| {
                let key = meta.location.as_ref();
                below(key, &prefix).is_some_and(|rest| !rest.is_empty())
                    && self.roots.is_content(key)
            })
            .map(|meta| (meta.location.to_string(), Entry::of(meta)))
            .collect();
        let blob = encode(&listed)?;
        if blob.len() > MAX_LISTING_BYTES {
            return Ok(());
        }
        {
            let mut db = self.lock();
            let tx = db.transaction()?;
            let at = now();
            let (from, to) = subtree(&prefix);
            let stale: Vec<String> = {
                let mut statement = tx.prepare(
                    "SELECT key FROM objects WHERE present=1 AND queued_operation IS NULL AND key >= ?1 AND key < ?2",
                )?;
                let rows = statement
                    .query_map(params![from, to], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows.into_iter()
                    .filter(|key| !listed.contains_key(key))
                    .collect()
            };
            for key in stale {
                tx.execute(
                    "UPDATE objects SET present=0,size=NULL,last_modified=NULL,e_tag=NULL,version=NULL,observed_at=?2 WHERE key=?1",
                    params![key, at],
                )?;
            }
            for entry in listed.values() {
                upsert_present(&tx, entry, at)?;
            }
            tx.execute(
                "DELETE FROM listings WHERE prefix >= ?1 AND prefix < ?2",
                params![from, to],
            )?;
            let mut snapshots = Snapshots::default();
            snapshots.replace_below(&tx, &prefix, &listed)?;
            snapshots.save(&tx)?;
            tx.execute(
                "INSERT INTO listings(prefix,entries,observed_at) VALUES(?1,?2,?3)
                 ON CONFLICT(prefix) DO UPDATE SET entries=?2,observed_at=?3",
                params![prefix, blob, at],
            )?;
            tx.commit()?;
        }
        self.enforce_bounds()
    }

    pub(crate) fn lookup(&self, key: &ObjectPath) -> Result<Known> {
        let key = key.to_string();
        let db = self.lock();
        match stored(&db, &key)? {
            Some((1, entry)) => return Ok(Known::Present(entry.meta()?)),
            Some((0, _)) => return Ok(Known::Absent),
            _ => {}
        }
        let Some(listing) = nearest_listing(&db, parent(&key))? else {
            return Ok(Known::Unknown);
        };
        match listing.get(&key) {
            Some(entry) => Ok(Known::Present(entry.meta()?)),
            None => Ok(Known::Absent),
        }
    }

    /// The snapshot of `prefix` or of its nearest ancestor, filtered to `prefix`. None: no snapshot.
    pub(crate) fn listing(&self, prefix: &ObjectPath) -> Result<Option<Vec<ObjectMeta>>> {
        let prefix = prefix.to_string();
        let db = self.lock();
        let Some(listing) = nearest_listing(&db, Some(&prefix))? else {
            return Ok(None);
        };
        let (from, to) = subtree(&prefix);
        listing
            .range(from..to)
            .map(|(_, entry)| entry.meta())
            .collect::<Result<Vec<_>>>()
            .map(Some)
    }

    /// FlowPath may have cached the queued bytes of `operation_id` at `key`.
    pub(crate) fn mark_queued(&self, key: &ObjectPath, operation_id: &str) -> Result<()> {
        let key = key.to_string();
        if !self.roots.is_content(&key) {
            return Ok(());
        }
        self.lock().execute(
            "INSERT INTO objects(key,present,queued_operation,observed_at) VALUES(?1,?2,?3,?4)
             ON CONFLICT(key) DO UPDATE SET queued_operation=?3",
            params![key, QUEUED_ONLY, operation_id, now()],
        )?;
        self.after_write()
    }

    /// Clears the mark; true when `operation_id` was the marked one.
    pub(crate) fn forget_queued(&self, key: &ObjectPath, operation_id: &str) -> Result<bool> {
        let key = key.to_string();
        let db = self.lock();
        let marked: Option<(i64, Option<String>)> = db
            .query_row(
                "SELECT present,queued_operation FROM objects WHERE key=?1",
                [&key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((present, Some(marked))) = marked else {
            return Ok(false);
        };
        if marked != operation_id {
            return Ok(false);
        }
        if present == QUEUED_ONLY {
            db.execute("DELETE FROM objects WHERE key=?1", [&key])?;
        } else {
            db.execute(
                "UPDATE objects SET queued_operation=NULL WHERE key=?1",
                [&key],
            )?;
        }
        Ok(true)
    }

    fn after_write(&self) -> Result<()> {
        let writes = self
            .writes
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        if writes.is_multiple_of(BOUNDS_CHECK_EVERY) {
            self.enforce_bounds()?;
        }
        Ok(())
    }

    /// At most 200 000 object rows and 64 MiB. Over the byte bound the oldest listings go
    /// first, then objects; over the row bound only the oldest objects go.
    fn enforce_bounds(&self) -> Result<()> {
        let db = self.lock();
        loop {
            let rows: i64 = db.query_row("SELECT COUNT(*) FROM objects", [], |row| row.get(0))?;
            let pages: i64 = db.query_row(
                "SELECT (SELECT page_count FROM pragma_page_count()) - (SELECT freelist_count FROM pragma_freelist_count())",
                [],
                |row| row.get(0),
            )?;
            let page_size: i64 =
                db.query_row("SELECT page_size FROM pragma_page_size()", [], |row| {
                    row.get(0)
                })?;
            let over_bytes = pages.saturating_mul(page_size) > MAX_INDEX_BYTES;
            if rows <= self.max_objects && !over_bytes {
                return Ok(());
            }
            if over_bytes
                && db.execute(
                    "DELETE FROM listings WHERE prefix=(SELECT prefix FROM listings ORDER BY observed_at LIMIT 1)",
                    [],
                )? > 0
            {
                continue;
            }
            let batch = if over_bytes {
                EVICTION_BATCH
            } else {
                (rows - self.max_objects).min(EVICTION_BATCH)
            };
            let removed = db.execute(
                "DELETE FROM objects WHERE key IN (SELECT key FROM objects WHERE queued_operation IS NULL ORDER BY observed_at LIMIT ?1)",
                [batch],
            )?;
            if removed == 0 {
                return Ok(());
            }
        }
    }
}

fn create_private_file(path: &Path) -> Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).with_context(|| {
        format!(
            "Could not create the offline object index at {}",
            path.display()
        )
    })?;
    Ok(())
}

fn upsert_present(db: &Connection, entry: &Entry, at: i64) -> Result<()> {
    db.execute(
        "INSERT INTO objects(key,present,size,last_modified,e_tag,version,observed_at) VALUES(?1,1,?2,?3,?4,?5,?6)
         ON CONFLICT(key) DO UPDATE SET present=1,size=?2,last_modified=?3,e_tag=?4,version=?5,observed_at=?6",
        params![entry.k, entry.s as i64, entry.m, entry.e, entry.v, at],
    )?;
    Ok(())
}

/// The key's row: its `present` flag and recorded metadata.
fn stored(db: &Connection, key: &str) -> Result<Option<(i64, Entry)>> {
    Ok(db
        .query_row(
            "SELECT present,size,last_modified,e_tag,version FROM objects WHERE key=?1",
            [key],
            |row| {
                let size: Option<i64> = row.get(1)?;
                let modified: Option<i64> = row.get(2)?;
                Ok((
                    row.get(0)?,
                    Entry {
                        k: key.to_owned(),
                        s: size.unwrap_or_default().max(0) as u64,
                        m: modified.unwrap_or_default(),
                        e: row.get(3)?,
                        v: row.get(4)?,
                    },
                ))
            },
        )
        .optional()?)
}

fn read_listing(db: &Connection, prefix: &str) -> Result<Option<Listing>> {
    let blob: Option<Vec<u8>> = db
        .query_row(
            "SELECT entries FROM listings WHERE prefix=?1",
            [prefix],
            |row| row.get(0),
        )
        .optional()?;
    let Some(blob) = blob else {
        return Ok(None);
    };
    let entries: Vec<Entry> = serde_json::from_slice(&blob)?;
    Ok(Some(
        entries
            .into_iter()
            .map(|entry| (entry.k.clone(), entry))
            .collect(),
    ))
}

fn encode(listing: &Listing) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&listing.values().collect::<Vec<_>>())?)
}

/// The stored listing of `from` or of its nearest ancestor.
fn nearest_listing(db: &Connection, from: Option<&str>) -> Result<Option<Listing>> {
    let mut candidate = from;
    while let Some(prefix) = candidate {
        if let Some(listing) = read_listing(db, prefix)? {
            return Ok(Some(listing));
        }
        candidate = parent(prefix);
    }
    Ok(None)
}

/// Stored listings changed within one transaction, each read and written back once.
#[derive(Default)]
struct Snapshots {
    loaded: HashMap<String, Option<Listing>>,
    changed: BTreeSet<String>,
}

impl Snapshots {
    fn load(&mut self, db: &Connection, prefix: &str) -> Result<Option<&mut Listing>> {
        if !self.loaded.contains_key(prefix) {
            let listing = read_listing(db, prefix)?;
            self.loaded.insert(prefix.to_owned(), listing);
        }
        Ok(self.loaded.get_mut(prefix).and_then(Option::as_mut))
    }

    /// Keeps every stored listing that covers `key` current with what this device observed.
    fn apply(&mut self, db: &Connection, key: &str, entry: Option<&Entry>) -> Result<()> {
        for prefix in ancestors(key) {
            let Some(listing) = self.load(db, prefix)? else {
                continue;
            };
            let changed = match entry {
                Some(entry) => {
                    listing.insert(key.to_owned(), entry.clone()).as_ref() != Some(entry)
                }
                None => listing.remove(key).is_some(),
            };
            if changed {
                self.changed.insert(prefix.to_owned());
            }
        }
        Ok(())
    }

    /// Gives every stored listing above `prefix` the newer complete listing's contents below it.
    fn replace_below(&mut self, db: &Connection, prefix: &str, listed: &Listing) -> Result<()> {
        let (from, to) = subtree(prefix);
        for ancestor in ancestors(prefix) {
            let Some(listing) = self.load(db, ancestor)? else {
                continue;
            };
            let mut replaced = listing.split_off(from.as_str());
            listing.append(&mut replaced.split_off(to.as_str()));
            listing.extend(
                listed
                    .iter()
                    .map(|(key, entry)| (key.clone(), entry.clone())),
            );
            self.changed.insert(ancestor.to_owned());
        }
        Ok(())
    }

    fn save(self, db: &Connection) -> Result<()> {
        let Self { loaded, changed } = self;
        for prefix in changed {
            let Some(Some(listing)) = loaded.get(&prefix) else {
                continue;
            };
            let blob = encode(listing)?;
            if blob.len() > MAX_LISTING_BYTES {
                db.execute("DELETE FROM listings WHERE prefix=?1", [&prefix])?;
            } else {
                db.execute(
                    "UPDATE listings SET entries=?2 WHERE prefix=?1",
                    params![prefix, blob],
                )?;
            }
        }
        Ok(())
    }
}
