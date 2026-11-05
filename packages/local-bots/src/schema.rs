//! The store's file, connection settings and schema migrations (`PRAGMA user_version`).

use crate::error::{Error, Result};
use rusqlite::Connection;
use std::{path::Path, time::Duration};

/// Migration `n` brings the schema from version `n` to `n + 1`.
pub(crate) const MIGRATIONS: &[&str] = &[
    "CREATE TABLE bots (
        id TEXT PRIMARY KEY NOT NULL,
        preset_id TEXT,
        name TEXT NOT NULL,
        description TEXT,
        instructions TEXT,
        appearance TEXT NOT NULL,
        model TEXT NOT NULL,
        classification TEXT,
        pii INTEGER NOT NULL DEFAULT 0,
        capabilities TEXT NOT NULL,
        profile_id TEXT,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    ) STRICT;
    CREATE INDEX bots_created ON bots (created_at, id);
    CREATE INDEX bots_profile ON bots (profile_id);

    CREATE TABLE bot_apps (
        bot_id TEXT NOT NULL REFERENCES bots (id) ON DELETE CASCADE,
        app_id TEXT NOT NULL,
        added_at INTEGER NOT NULL,
        PRIMARY KEY (bot_id, app_id)
    ) STRICT;

    CREATE TABLE bot_secrets (
        bot_id TEXT NOT NULL REFERENCES bots (id) ON DELETE CASCADE,
        name TEXT NOT NULL,
        ciphertext BLOB NOT NULL,
        description TEXT,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        last_used_at INTEGER,
        PRIMARY KEY (bot_id, name)
    ) STRICT;

    CREATE TABLE conversations (
        id TEXT PRIMARY KEY NOT NULL,
        kind TEXT NOT NULL,
        title TEXT,
        encryption TEXT NOT NULL,
        direct_key TEXT UNIQUE,
        created_by TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        last_activity_at INTEGER NOT NULL
    ) STRICT;
    CREATE INDEX conversations_activity ON conversations (last_activity_at, id);

    CREATE TABLE conversation_members (
        conversation_id TEXT NOT NULL REFERENCES conversations (id) ON DELETE CASCADE,
        principal_id TEXT NOT NULL,
        principal_kind TEXT NOT NULL,
        added_by TEXT,
        joined_at INTEGER NOT NULL,
        left_at INTEGER,
        last_read_at INTEGER,
        PRIMARY KEY (conversation_id, principal_id)
    ) STRICT;
    CREATE INDEX conversation_members_principal ON conversation_members (principal_id, left_at);

    CREATE TABLE entries (
        id TEXT PRIMARY KEY NOT NULL,
        conversation_id TEXT NOT NULL REFERENCES conversations (id) ON DELETE CASCADE,
        message_id TEXT NOT NULL UNIQUE,
        sender_id TEXT NOT NULL,
        kind TEXT NOT NULL,
        envelope TEXT NOT NULL,
        created_at INTEGER NOT NULL
    ) STRICT;
    CREATE INDEX entries_position ON entries (conversation_id, created_at, id);
    CREATE INDEX entries_kind_position ON entries (conversation_id, kind, created_at, id);",
    // Bots wave B. `bots` is rebuilt (SQLite can't drop NOT NULL) so a built-in's row (`system`
    // general | flux, one each) keeps a null name; foreign keys are off while it is swapped.
    // Existing events get positions 1, 2, … per conversation in the order they were written.
    "CREATE TABLE bots_v2 (
        id TEXT PRIMARY KEY NOT NULL,
        system TEXT,
        preset_id TEXT,
        name TEXT,
        description TEXT,
        instructions TEXT,
        appearance TEXT NOT NULL,
        model TEXT NOT NULL,
        embedding_model TEXT,
        classification TEXT,
        pii INTEGER NOT NULL DEFAULT 0,
        capabilities TEXT NOT NULL,
        profile_id TEXT,
        workspace_app_id TEXT,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    ) STRICT;
    INSERT INTO bots_v2 (id, preset_id, name, description, instructions, appearance, model,
        classification, pii, capabilities, profile_id, created_at, updated_at)
    SELECT id, preset_id, name, description, instructions, appearance, model,
        classification, pii, capabilities, profile_id, created_at, updated_at
    FROM bots;
    DROP TABLE bots;
    ALTER TABLE bots_v2 RENAME TO bots;
    CREATE INDEX bots_created ON bots (created_at, id);
    CREATE INDEX bots_profile ON bots (profile_id);
    CREATE UNIQUE INDEX bots_system ON bots (system);
    CREATE UNIQUE INDEX bots_workspace ON bots (workspace_app_id);

    ALTER TABLE conversations ADD COLUMN last_seq INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE entries ADD COLUMN seq INTEGER;
    UPDATE entries SET seq = numbered.position
    FROM (
        SELECT id, ROW_NUMBER() OVER (PARTITION BY conversation_id ORDER BY created_at, id)
            AS position
        FROM entries
    ) AS numbered
    WHERE numbered.id = entries.id;
    UPDATE conversations SET last_seq = (
        SELECT COALESCE(MAX(seq), 0) FROM entries WHERE entries.conversation_id = conversations.id
    );
    CREATE UNIQUE INDEX entries_seq ON entries (conversation_id, seq);

    CREATE TABLE permissions (
        id TEXT PRIMARY KEY NOT NULL,
        from_bot_id TEXT NOT NULL REFERENCES bots (id) ON DELETE CASCADE,
        to_bot_id TEXT NOT NULL REFERENCES bots (id) ON DELETE CASCADE,
        granted_at INTEGER NOT NULL,
        granted_by TEXT NOT NULL,
        UNIQUE (from_bot_id, to_bot_id)
    ) STRICT;
    CREATE INDEX permissions_to ON permissions (to_bot_id);",
    // Each bot keeps its own summaries of a conversation (`author_id`). Files that ran an
    // earlier draft of the previous migration hold an authorless `segments` nothing wrote to.
    "DROP TABLE IF EXISTS segments;
    CREATE TABLE segments (
        id TEXT PRIMARY KEY NOT NULL,
        conversation_id TEXT NOT NULL REFERENCES conversations (id) ON DELETE CASCADE,
        author_id TEXT NOT NULL REFERENCES bots (id) ON DELETE CASCADE,
        level INTEGER NOT NULL,
        seq_from INTEGER NOT NULL,
        seq_to INTEGER NOT NULL,
        t_from INTEGER NOT NULL,
        t_to INTEGER NOT NULL,
        title TEXT,
        summary TEXT NOT NULL,
        parent_id TEXT,
        version INTEGER NOT NULL,
        created_at INTEGER NOT NULL
    ) STRICT;
    CREATE INDEX segments_position ON segments (conversation_id, author_id, level, seq_from);
    CREATE INDEX segments_author ON segments (author_id);",
    // Standing tool approvals (and the short-lived answer tokens beside them) and the turns
    // parked behind an approval card.
    "CREATE TABLE tool_grants (
        bot_id TEXT NOT NULL REFERENCES bots (id) ON DELETE CASCADE,
        tool TEXT NOT NULL,
        scope TEXT NOT NULL,
        granted_at INTEGER NOT NULL,
        granted_by TEXT NOT NULL,
        PRIMARY KEY (bot_id, tool, scope)
    ) STRICT;
    CREATE TABLE parked_turns (
        turn_id TEXT PRIMARY KEY NOT NULL,
        bot_id TEXT NOT NULL REFERENCES bots (id) ON DELETE CASCADE,
        conversation_id TEXT NOT NULL,
        trigger_message_id TEXT NOT NULL,
        approval_id TEXT NOT NULL,
        parked_at INTEGER NOT NULL
    ) STRICT;
    CREATE INDEX parked_turns_bot ON parked_turns (bot_id);",
    // Scheduled tasks a bot runs for the person, once or on a cron schedule.
    "CREATE TABLE tasks (
        id TEXT PRIMARY KEY NOT NULL,
        bot_id TEXT NOT NULL REFERENCES bots (id) ON DELETE CASCADE,
        kind TEXT NOT NULL,
        run_at INTEGER,
        cron TEXT,
        timezone TEXT,
        instruction TEXT NOT NULL,
        title TEXT NOT NULL,
        next_run_at INTEGER,
        last_run_at INTEGER,
        enabled INTEGER NOT NULL DEFAULT 1,
        created_by TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    ) STRICT;
    CREATE INDEX tasks_bot ON tasks (bot_id);
    CREATE INDEX tasks_due ON tasks (enabled, next_run_at);",
    // What the person lets a bot do in one attached app: permission bits (null keeps the full
    // role) and scopes as JSON.
    "ALTER TABLE bot_apps ADD COLUMN permission_mask INTEGER;
    ALTER TABLE bot_apps ADD COLUMN scopes TEXT NOT NULL DEFAULT '{}';",
    // Room invitations mirror the API's: only ACTIVE members read and receive; a bot's room
    // limits (`BotRoomLimits` JSON) live on its membership.
    "ALTER TABLE conversation_members ADD COLUMN status TEXT NOT NULL DEFAULT 'ACTIVE';
    ALTER TABLE conversation_members ADD COLUMN limits TEXT;
    ALTER TABLE conversation_members ADD COLUMN invited_by TEXT;
    ALTER TABLE conversation_members ADD COLUMN invited_at INTEGER;
    ALTER TABLE conversation_members ADD COLUMN accepted_at INTEGER;",
    // Files a member put into a conversation; the bytes live in the `conversation_files` directory
    // next to this file, named by id.
    "CREATE TABLE conversation_files (
        id TEXT PRIMARY KEY NOT NULL,
        conversation_id TEXT NOT NULL REFERENCES conversations (id) ON DELETE CASCADE,
        uploader_id TEXT NOT NULL,
        name TEXT NOT NULL,
        mime TEXT NOT NULL,
        bytes INTEGER NOT NULL,
        sha256 TEXT NOT NULL,
        message_id TEXT,
        created_at INTEGER NOT NULL
    );
    CREATE INDEX conversation_files_message ON conversation_files (conversation_id, message_id);",
    // The person's pinned conversations, the messages pinned in a conversation and the messages
    // they starred. Times are epoch micros like every other column.
    "CREATE TABLE pinned_conversations (
        conversation_id TEXT PRIMARY KEY NOT NULL REFERENCES conversations (id) ON DELETE CASCADE,
        pinned_at INTEGER NOT NULL
    ) STRICT;
    CREATE TABLE pinned_messages (
        conversation_id TEXT NOT NULL REFERENCES conversations (id) ON DELETE CASCADE,
        message_id TEXT NOT NULL,
        pinned_by TEXT NOT NULL,
        pinned_at INTEGER NOT NULL,
        PRIMARY KEY (conversation_id, message_id)
    ) STRICT;
    CREATE INDEX pinned_messages_position ON pinned_messages (conversation_id, pinned_at, message_id);
    CREATE TABLE starred_messages (
        message_id TEXT PRIMARY KEY NOT NULL,
        conversation_id TEXT NOT NULL REFERENCES conversations (id) ON DELETE CASCADE,
        starred_at INTEGER NOT NULL
    ) STRICT;
    CREATE INDEX starred_messages_position ON starred_messages (starred_at, message_id);
    CREATE INDEX starred_messages_conversation ON starred_messages (conversation_id, starred_at, message_id);",
    // The speech-to-text, text-to-speech, image generation and vision bits a bot pins; null lets
    // the host choose.
    "ALTER TABLE bots ADD COLUMN stt_model TEXT;
    ALTER TABLE bots ADD COLUMN tts_model TEXT;
    ALTER TABLE bots ADD COLUMN image_model TEXT;
    ALTER TABLE bots ADD COLUMN vision_model TEXT;",
    // Skills the person installed on a bot; only ACTIVE ones reach the bot.
    "CREATE TABLE skills (
        id TEXT PRIMARY KEY NOT NULL,
        bot_id TEXT NOT NULL REFERENCES bots (id) ON DELETE CASCADE,
        name TEXT NOT NULL,
        description TEXT NOT NULL,
        body TEXT NOT NULL,
        source TEXT NOT NULL,
        status TEXT NOT NULL,
        tools TEXT NOT NULL DEFAULT '[]',
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        UNIQUE (bot_id, name)
    ) STRICT;
    CREATE INDEX skills_bot_status ON skills (bot_id, status);",
    // MCP servers and OAuth accounts the person attached to a bot. The account's sign-in stays
    // in the desktop's native OAuth store.
    "CREATE TABLE bot_connectors (
        id TEXT PRIMARY KEY NOT NULL,
        bot_id TEXT NOT NULL REFERENCES bots (id) ON DELETE CASCADE,
        kind TEXT NOT NULL,
        label TEXT NOT NULL,
        provider_id TEXT,
        url TEXT,
        auth TEXT NOT NULL DEFAULT '{\"type\":\"none\"}',
        scopes TEXT NOT NULL DEFAULT '[]',
        default_policy TEXT NOT NULL DEFAULT 'ask',
        tool_policies TEXT NOT NULL DEFAULT '{}',
        tools_snapshot TEXT NOT NULL DEFAULT '[]',
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    ) STRICT;
    CREATE INDEX bot_connectors_bot ON bot_connectors (bot_id, created_at);",
    // The schema hash each tool had when the person set or confirmed its policy.
    "ALTER TABLE bot_connectors ADD COLUMN confirmed_hashes TEXT NOT NULL DEFAULT '{}';",
    // The Bit a bot's sub-tasks run on; null uses the bot's own model.
    "ALTER TABLE bots ADD COLUMN subtask_model TEXT;",
    // A group's picture as a PNG data URL of at most 256 px.
    "ALTER TABLE conversations ADD COLUMN image TEXT;",
    // Packages this device installed only to give a bot its skills: removed again with the last skill.
    "CREATE TABLE skill_package_installs (
        package_id TEXT PRIMARY KEY NOT NULL,
        installed_at INTEGER NOT NULL
    ) STRICT;",
    // Clearing the owner's history keeps the shared events available to bots.
    "ALTER TABLE conversations ADD COLUMN owner_cleared_seq INTEGER;
    ALTER TABLE conversations ADD COLUMN owner_cleared_at INTEGER;
    ALTER TABLE conversations ADD COLUMN owner_hidden INTEGER NOT NULL DEFAULT 0;
    CREATE TABLE conversation_transfers (
        conversation_id TEXT NOT NULL REFERENCES conversations (id) ON DELETE CASCADE,
        bot_id TEXT NOT NULL REFERENCES bots (id) ON DELETE CASCADE,
        owner_id TEXT NOT NULL,
        account_bot_id TEXT NOT NULL,
        account_conversation_id TEXT NOT NULL,
        PRIMARY KEY (conversation_id, bot_id, owner_id, account_bot_id)
    ) STRICT;
    CREATE VIEW owner_entries AS
        SELECT e.* FROM entries e JOIN conversations c ON c.id = e.conversation_id
        WHERE c.owner_cleared_seq IS NULL OR (
            (e.seq > c.owner_cleared_seq OR (e.seq IS NULL AND e.created_at > c.owner_cleared_at))
            AND (e.kind NOT IN ('edit', 'reaction', 'delete') OR EXISTS (
                SELECT 1 FROM entries target
                WHERE target.conversation_id = e.conversation_id
                  AND target.message_id = json_extract(e.envelope, '$.body.target')
                  AND (target.seq > c.owner_cleared_seq
                       OR (target.seq IS NULL AND target.created_at > c.owner_cleared_at))
            ))
        );",
];

pub(crate) fn open(path: &Path) -> Result<Connection> {
    create_private(path)?;
    let mut db = Connection::open(path)?;
    db.busy_timeout(Duration::from_secs(10))?;
    configure(&db)?;
    migrate(&mut db)?;
    Ok(db)
}

/// The file holds private chats, so on Unix it starts out readable by its owner only.
fn create_private(path: &Path) -> Result<()> {
    let failed = |error: std::io::Error| {
        Error::storage(format!(
            "Cannot create the local bots file '{}': {error}",
            path.display()
        ))
    };
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(failed)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map(drop).map_err(failed)
}

/// Row-returning PRAGMAs go through `pragma_update_and_check` or `query_row`: hosts that enable
/// rusqlite's `extra_check` refuse them in `execute_batch`, which `pragma_update` uses. With
/// `secure_delete` the main file zeroes deleted content; `journal_size_limit = 0` keeps an
/// emptied WAL at zero bytes, see [`empty_wal`].
fn configure(db: &Connection) -> Result<()> {
    db.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))?;
    db.pragma_update(None, "synchronous", "FULL")?;
    db.pragma_update(None, "foreign_keys", "ON")?;
    db.pragma_update_and_check(None, "secure_delete", "ON", |_| Ok(()))?;
    db.pragma_update_and_check(None, "journal_size_limit", 0, |_| Ok(()))?;
    Ok(())
}

/// Copies every committed page into the main file and truncates the WAL, so what a delete
/// zeroed there is gone from the WAL as well. Best effort after a committed delete: a
/// checkpoint another connection blocks runs again after the next one.
pub(crate) fn empty_wal(db: &Connection) {
    let _ = db.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
}

pub(crate) fn schema_version(db: &Connection) -> Result<usize> {
    let version: i64 = db.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    usize::try_from(version)
        .map_err(|_| Error::storage(format!("The local bots file has schema version {version}")))
}

fn migrate(db: &mut Connection) -> Result<()> {
    let version = schema_version(db)?;
    if version > MIGRATIONS.len() {
        return Err(Error::storage(format!(
            "The local bots file has schema version {version}, newer than the {} this version of Flow-Like reads; update Flow-Like to open it",
            MIGRATIONS.len()
        )));
    }
    if version == MIGRATIONS.len() {
        return Ok(());
    }
    // A migration that rebuilds a table drops it, and with foreign keys on that deletes the rows
    // referencing it. The setting only changes outside a transaction.
    db.pragma_update(None, "foreign_keys", "OFF")?;
    let migrated = apply(db, version);
    db.pragma_update(None, "foreign_keys", "ON")?;
    migrated
}

fn apply(db: &mut Connection, version: usize) -> Result<()> {
    for (applied, migration) in MIGRATIONS.iter().enumerate().skip(version) {
        let tx = db.transaction()?;
        tx.execute_batch(migration)?;
        references_hold(&tx, applied + 1)?;
        tx.pragma_update(None, "user_version", applied as i64 + 1)?;
        tx.commit()?;
    }
    Ok(())
}

/// Fails, so the migration rolls back, when a row references one that is gone.
fn references_hold(db: &Connection, version: usize) -> Result<()> {
    let mut check = db.prepare("PRAGMA foreign_key_check")?;
    let mut broken = check.query([])?;
    if let Some(row) = broken.next()? {
        let table: String = row.get(0)?;
        return Err(Error::storage(format!(
            "Upgrading the local bots file to schema version {version} left a row in '{table}' pointing at one that is gone"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        EntriesQuery, LocalBotStore,
        tests::{Recorder, TestKey},
    };
    use std::sync::Arc;

    const BOT_ROW: &str = r#"('scout', 'preset-1', 'Scout', 'Finds things', 'Be brief',
        '{"kind":"pip"}', '{"primary":null,"fallbacks":[]}', 'internal', 1, '[]', 'profile-1',
        10, 20)"#;

    /// A file as schema version 1 left it: a bot with an app and a secret, a direct chat whose
    /// events were written out of order (two in the same microsecond), a group with one event
    /// and a conversation without any.
    fn version_one(path: &Path) {
        create_private(path).unwrap();
        let mut db = Connection::open(path).unwrap();
        configure(&db).unwrap();
        let tx = db.transaction().unwrap();
        tx.execute_batch(MIGRATIONS[0]).unwrap();
        tx.pragma_update(None, "user_version", 1).unwrap();
        tx.commit().unwrap();
        let envelope = r#"'{"v":1,"enc":"none","body":{"text":"hi"}}'"#;
        db.execute_batch(&format!(
            r#"INSERT INTO bots (id, preset_id, name, description, instructions, appearance,
                   model, classification, pii, capabilities, profile_id, created_at, updated_at)
               VALUES {BOT_ROW};
               INSERT INTO bot_apps VALUES ('scout', 'app-1', 30);
               INSERT INTO bot_secrets VALUES ('scout', 'TOKEN', x'0102', 'api', 40, 41, NULL);
               INSERT INTO conversations VALUES
                   ('direct', 'direct', NULL, 'plain', 'local|scout', 'local', 50, 50, 90),
                   ('group', 'group', 'Team', 'plain', NULL, 'local', 51, 51, 51),
                   ('quiet', 'group', 'Quiet', 'plain', NULL, 'local', 52, 52, 52);
               INSERT INTO conversation_members VALUES
                   ('direct', 'local', 'user', NULL, 50, NULL, 60),
                   ('direct', 'scout', 'bot', 'local', 50, NULL, NULL),
                   ('group', 'local', 'user', NULL, 51, NULL, NULL),
                   ('quiet', 'local', 'user', NULL, 52, NULL, NULL);
               INSERT INTO entries VALUES
                   ('e-late', 'direct', 'm-3', 'scout', 'message', {envelope}, 90),
                   ('e-b', 'direct', 'm-2', 'local', 'message', {envelope}, 60),
                   ('e-a', 'direct', 'm-1', 'local', 'message', {envelope}, 60),
                   ('e-g', 'group', 'm-4', 'local', 'message', {envelope}, 70);"#
        ))
        .unwrap();
    }

    fn count(db: &Connection, sql: &str) -> i64 {
        db.query_row(sql, [], |row| row.get(0)).unwrap()
    }

    #[test]
    fn a_version_one_file_keeps_its_data_and_gets_positions_built_ins_grants_and_summaries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("local_bots.db");
        version_one(&path);

        let store = LocalBotStore::open(
            &path,
            Arc::new(TestKey::default()),
            Arc::new(Recorder::new(&path)),
        )
        .unwrap();
        let bots = store.list_bots().unwrap();
        assert_eq!(bots.len(), 1);
        assert_eq!(
            (bots[0].id.as_str(), bots[0].name.as_deref()),
            ("scout", Some("Scout"))
        );
        let page = store.entries("direct", &EntriesQuery::default()).unwrap();
        assert_eq!(page.entries.len(), 3);
        drop(store);

        let db = open(&path).unwrap();
        assert_eq!(schema_version(&db).unwrap(), MIGRATIONS.len());
        assert_eq!(count(&db, "PRAGMA foreign_keys"), 1);
        let kept: (
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            i64,
            i64,
        ) = db
            .query_row(
                "SELECT description, system, embedding_model, workspace_app_id, created_at,
                     updated_at FROM bots WHERE id = 'scout'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            kept,
            ("Finds things".into(), None, None, None, 10, 20),
            "the rebuilt row keeps its values; the new columns start empty"
        );
        assert_eq!(count(&db, "SELECT COUNT(*) FROM bot_apps"), 1);
        assert_eq!(count(&db, "SELECT COUNT(*) FROM bot_secrets"), 1);

        let positions: Vec<(String, i64)> = db
            .prepare("SELECT id, seq FROM entries ORDER BY conversation_id, seq")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            positions,
            [
                ("e-a".to_string(), 1),
                ("e-b".to_string(), 2),
                ("e-late".to_string(), 3),
                ("e-g".to_string(), 1),
            ]
        );
        let last: Vec<(String, i64)> = db
            .prepare("SELECT id, last_seq FROM conversations ORDER BY id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            last,
            [
                ("direct".to_string(), 3),
                ("group".to_string(), 1),
                ("quiet".to_string(), 0)
            ]
        );

        assert!(
            db.execute(
                "INSERT INTO entries (id, conversation_id, message_id, sender_id, kind, envelope,
                     created_at, seq) VALUES ('dup', 'direct', 'm-9', 'local', 'message', '{}', 99, 3)",
                [],
            )
            .is_err(),
            "a position is never handed out twice in one conversation"
        );
        db.execute_batch(
            "INSERT INTO entries (id, conversation_id, message_id, sender_id, kind, envelope,
                 created_at) VALUES ('n-1', 'quiet', 'm-10', 'local', 'message', '{}', 100),
                 ('n-2', 'quiet', 'm-11', 'local', 'message', '{}', 101);",
        )
        .unwrap();

        let built_in = "INSERT INTO bots (id, system, appearance, model, capabilities,
             created_at, updated_at) VALUES (?1, 'flux', '{}', '{}', '[]', 1, 1)";
        db.execute(built_in, ["flux-row"]).unwrap();
        assert!(
            db.execute(built_in, ["flux-again"]).is_err(),
            "one row per built-in"
        );
        db.execute(
            "UPDATE bots SET workspace_app_id = 'app-ws' WHERE id = 'scout'",
            [],
        )
        .unwrap();
        assert!(
            db.execute(
                "UPDATE bots SET workspace_app_id = 'app-ws' WHERE id = 'flux-row'",
                []
            )
            .is_err(),
            "one bot per workspace"
        );

        db.execute_batch(
            "INSERT INTO permissions VALUES ('g-1', 'scout', 'flux-row', 5, 'local'),
                 ('g-2', 'flux-row', 'scout', 5, 'local');
             INSERT INTO segments VALUES
                 ('s-scout', 'direct', 'scout', 0, 1, 3, 60, 90, 'Hello', 'summary', NULL, 1, 95),
                 ('s-flux', 'direct', 'flux-row', 0, 1, 3, 60, 90, NULL, 'summary', NULL, 1, 96);",
        )
        .unwrap();
        assert!(
            db.execute(
                "INSERT INTO segments VALUES ('s-x', 'direct', 'nobody', 0, 1, 3, 60, 90, NULL,
                     'summary', NULL, 1, 97)",
                []
            )
            .is_err(),
            "summaries are written by bots on this device"
        );
        assert!(
            db.execute(
                "INSERT INTO permissions VALUES ('g-3', 'scout', 'flux-row', 6, 'local')",
                []
            )
            .is_err(),
            "one grant per direction"
        );
        assert!(
            db.execute(
                "INSERT INTO permissions VALUES ('g-4', 'scout', 'nobody', 6, 'local')",
                []
            )
            .is_err(),
            "grants name bots on this device"
        );

        db.execute("DELETE FROM bots WHERE id = 'scout'", [])
            .unwrap();
        assert_eq!(count(&db, "SELECT COUNT(*) FROM bot_apps"), 0);
        assert_eq!(count(&db, "SELECT COUNT(*) FROM bot_secrets"), 0);
        assert_eq!(
            count(&db, "SELECT COUNT(*) FROM permissions"),
            0,
            "a deleted bot's grants go in both directions"
        );
        assert_eq!(
            count(
                &db,
                "SELECT COUNT(*) FROM segments WHERE author_id = 'scout'"
            ),
            0,
            "a deleted bot's summaries go; another bot's stay"
        );
        assert_eq!(count(&db, "SELECT COUNT(*) FROM segments"), 1);
        db.execute("DELETE FROM conversations WHERE id = 'direct'", [])
            .unwrap();
        assert_eq!(count(&db, "SELECT COUNT(*) FROM segments"), 0);
        drop(db);

        let reopened = open(&path).unwrap();
        assert_eq!(schema_version(&reopened).unwrap(), MIGRATIONS.len());
        assert_eq!(count(&reopened, "SELECT COUNT(*) FROM bots"), 1);
    }

    #[test]
    fn a_migration_that_breaks_a_reference_rolls_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("local_bots.db");
        version_one(&path);
        let db = Connection::open(&path).unwrap();
        db.pragma_update(None, "foreign_keys", "OFF").unwrap();
        db.execute("INSERT INTO bot_apps VALUES ('ghost', 'app-2', 31)", [])
            .unwrap();
        drop(db);

        let error = open(&path).expect_err("the upgrade refuses a dangling reference");
        assert!(error.to_string().contains("bot_apps"), "{error}");
        let db = Connection::open(&path).unwrap();
        assert_eq!(schema_version(&db).unwrap(), 1, "the file stays as it was");
        assert_eq!(count(&db, "SELECT COUNT(*) FROM bots"), 1);
        assert_eq!(count(&db, "SELECT COUNT(*) FROM bot_apps"), 2);
    }

    fn segment_columns(db: &Connection) -> Vec<String> {
        db.prepare("SELECT name FROM pragma_table_info('segments') ORDER BY cid")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    /// A file that ran the draft of the second migration has `segments` without authors; the
    /// next migration replaces it, so every file ends up with the same table.
    #[test]
    fn a_draft_version_two_file_gets_authored_summaries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("local_bots.db");
        version_one(&path);
        let mut db = Connection::open(&path).unwrap();
        db.pragma_update(None, "foreign_keys", "OFF").unwrap();
        let tx = db.transaction().unwrap();
        tx.execute_batch(MIGRATIONS[1]).unwrap();
        tx.execute_batch(
            "CREATE TABLE segments (
                id TEXT PRIMARY KEY NOT NULL,
                conversation_id TEXT NOT NULL REFERENCES conversations (id) ON DELETE CASCADE,
                level INTEGER NOT NULL,
                seq_from INTEGER NOT NULL,
                seq_to INTEGER NOT NULL,
                t_from INTEGER NOT NULL,
                t_to INTEGER NOT NULL,
                title TEXT,
                summary TEXT NOT NULL,
                parent_id TEXT,
                version INTEGER NOT NULL,
                created_at INTEGER NOT NULL
            ) STRICT;
            CREATE INDEX segments_position ON segments (conversation_id, seq_from);",
        )
        .unwrap();
        tx.pragma_update(None, "user_version", 2).unwrap();
        tx.commit().unwrap();
        assert!(!segment_columns(&db).contains(&"author_id".to_string()));
        drop(db);

        let db = open(&path).unwrap();
        assert_eq!(schema_version(&db).unwrap(), MIGRATIONS.len());
        let fresh_dir = tempfile::tempdir().unwrap();
        let fresh = open(&fresh_dir.path().join("fresh.db")).unwrap();
        assert_eq!(segment_columns(&db), segment_columns(&fresh));
        assert_eq!(segment_columns(&db)[2], "author_id");
        assert_eq!(count(&db, "SELECT COUNT(*) FROM bots"), 1);
    }
}
