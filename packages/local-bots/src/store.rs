use crate::{error::Result, schema};
use chrono::{DateTime, FixedOffset, Utc};
use rusqlite::{
    Connection, Row, Transaction, TransactionBehavior,
    types::{FromSql, FromSqlError, FromSqlResult, Type, ValueRef},
};
use serde::de::DeserializeOwned;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};
use zeroize::Zeroizing;

/// Why the host could not hand out the key; the message reaches the person as it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyError {
    /// There is no usable key: none was made yet, or it was lost or damaged. Nothing sealed
    /// before opens.
    Missing(String),
    /// The keychain can't be asked right now: locked, access denied or unavailable.
    Unavailable(String),
}

/// Holds the key local bot secrets are sealed with, e.g. in the OS keychain.
pub trait SecretKeyProvider: Send + Sync {
    /// The 32-byte key. With `create`, a missing key is made and kept; without, a missing key
    /// is [`KeyError::Missing`].
    fn key(&self, create: bool) -> std::result::Result<Zeroizing<[u8; 32]>, KeyError>;
}

/// Told after every committed write, outside the store's lock.
pub trait ChangeNotifier: Send + Sync {
    /// A bot, its apps or its secrets changed; `None` when several bots did.
    fn bots_changed(&self, bot_id: Option<&str>);
    /// A conversation, its members or its entries changed; `None` when several did.
    fn inbox_changed(&self, conversation_id: Option<&str>);
}

/// What one write changed, announced once it committed.
#[derive(Debug, Default)]
pub(crate) struct Changes {
    bots: Vec<Option<String>>,
    inbox: Vec<Option<String>>,
    scrub: bool,
    files: bool,
}

fn note(changes: &mut Vec<Option<String>>, id: Option<&str>) {
    if !changes.iter().any(|known| known.as_deref() == id) {
        changes.push(id.map(str::to_string));
    }
}

impl Changes {
    pub fn bot(&mut self, bot_id: &str) {
        note(&mut self.bots, Some(bot_id));
    }

    pub fn bots(&mut self) {
        note(&mut self.bots, None);
    }

    pub fn conversation(&mut self, conversation_id: &str) {
        note(&mut self.inbox, Some(conversation_id));
    }

    /// The write removed conversation file rows; their bytes go once it committed.
    pub fn files(&mut self) {
        self.files = true;
    }

    /// The write deleted or overwrote content, which the WAL still holds until it is emptied.
    pub fn scrub(&mut self) {
        self.scrub = true;
    }
}

/// Local bots and their inbox in one SQLite file. Every call is synchronous and takes the one
/// connection's lock; hosts call it off their async runtime.
pub struct LocalBotStore {
    db: Mutex<Connection>,
    key: Arc<dyn SecretKeyProvider>,
    key_lock: Mutex<()>,
    notify: Arc<dyn ChangeNotifier>,
    pub(crate) files_dir: PathBuf,
}

impl LocalBotStore {
    /// Opens or creates the store at `path` and brings its schema up to date. The key is only
    /// asked for when a secret is written or exported.
    pub fn open(
        path: impl AsRef<Path>,
        key: Arc<dyn SecretKeyProvider>,
        notify: Arc<dyn ChangeNotifier>,
    ) -> Result<Self> {
        let db = schema::open(path.as_ref())?;
        let files_dir = path
            .as_ref()
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(crate::inbox::FILES_DIR);
        crate::inbox::sweep_files(&db, &files_dir);
        Ok(Self {
            db: Mutex::new(db),
            key,
            key_lock: Mutex::new(()),
            notify,
            files_dir,
        })
    }

    fn connection(&self) -> MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn read<T>(&self, work: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        work(&self.connection())
    }

    /// Runs `work` in one transaction and announces what it changed once it committed. A
    /// failed write rolls back and announces nothing.
    pub(crate) fn write<T>(
        &self,
        work: impl FnOnce(&Transaction<'_>, &mut Changes) -> Result<T>,
    ) -> Result<T> {
        let mut changes = Changes::default();
        let value = {
            let mut db = self.connection();
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let value = work(&tx, &mut changes)?;
            tx.commit()?;
            if changes.scrub {
                schema::empty_wal(&db);
            }
            if changes.files {
                crate::inbox::sweep_files(&db, &self.files_dir);
            }
            value
        };
        for bot_id in &changes.bots {
            self.notify.bots_changed(bot_id.as_deref());
        }
        for conversation_id in &changes.inbox {
            self.notify.inbox_changed(conversation_id.as_deref());
        }
        Ok(value)
    }

    /// The secret key, one caller at a time so two first writes can't each create one.
    pub(crate) fn secret_key(
        &self,
        create: bool,
    ) -> std::result::Result<Zeroizing<[u8; 32]>, KeyError> {
        let _one_at_a_time = self.key_lock.lock().unwrap_or_else(PoisonError::into_inner);
        self.key.key(create)
    }
}

/// The smallest step between two writes of one thing: the API's millisecond precision.
pub(crate) const STEP_MICROS: i64 = 1_000;

/// Now in epoch microseconds, at the millisecond precision the API stores.
pub(crate) fn now_micros() -> i64 {
    Utc::now().timestamp_millis() * STEP_MICROS
}

pub(crate) fn timestamp(micros: i64) -> DateTime<FixedOffset> {
    DateTime::from_timestamp_micros(micros)
        .unwrap_or_default()
        .fixed_offset()
}

/// A JSON text column.
pub(crate) struct Json<T>(pub T);

impl<T: DeserializeOwned> FromSql for Json<T> {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        serde_json::from_str(value.as_str()?)
            .map(Json)
            .map_err(|error| FromSqlError::Other(Box::new(error)))
    }
}

/// A text column holding one of a wire enum's ids.
pub(crate) fn wire<T>(
    row: &Row<'_>,
    column: &str,
    parse: fn(&str) -> Option<T>,
) -> rusqlite::Result<T> {
    let text: String = row.get(column)?;
    parse(&text).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            row.as_ref().column_index(column).unwrap_or_default(),
            Type::Text,
            format!("'{text}' is not a known {column}").into(),
        )
    })
}
