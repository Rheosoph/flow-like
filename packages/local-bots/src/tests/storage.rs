use super::*;
use crate::{codes, schema};

#[test]
fn the_schema_is_versioned_and_everything_survives_a_reopen() {
    let f = fixture();
    let bot = f.bot("Keeper");
    let direct = f.direct(&bot);
    f.text(&direct.id, &bot.id, "m_1");
    f.store
        .set_secret(&bot.id, "API_KEY", "sk-1", None)
        .unwrap();

    let version: i64 = f
        .raw()
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version as usize, schema::MIGRATIONS.len());
    let (journal, foreign_keys) = f
        .store
        .read(|db| {
            Ok((
                db.query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))?,
                db.query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))?,
            ))
        })
        .unwrap();
    assert_eq!((journal.as_str(), foreign_keys), ("wal", 1));

    let saved = f.store.get_bot(&bot.id).unwrap().unwrap();
    let again = f.open_again(TestKey::holding([7; 32])).unwrap();
    assert_eq!(again.list_bots().unwrap(), [saved]);
    let conversation = again.conversation(&direct.id).unwrap().unwrap();
    assert_eq!(conversation.unread_count, 1);
    assert_eq!(
        again.open_secrets(&bot.id).unwrap().readable[0]
            .value
            .as_str(),
        "sk-1"
    );
}

fn ciphertext(f: &Fixture, bot_id: &str) -> Vec<u8> {
    f.raw()
        .query_row(
            "SELECT ciphertext FROM bot_secrets WHERE bot_id = ?1",
            [bot_id],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn deleted_chats_and_secrets_are_gone_from_the_file_and_its_wal() {
    let f = fixture();
    let limit: i64 = f
        .store
        .read(|db| Ok(db.query_row("PRAGMA journal_size_limit", [], |row| row.get(0))?))
        .unwrap();
    assert_eq!(limit, 0, "an emptied WAL stays at zero bytes");

    let bot = f.bot("Keeper");
    let direct = f.direct(&bot);
    f.send(
        &direct.id,
        "local",
        "m_private",
        "message",
        json!({ "text": "private-words-4711" }),
    )
    .unwrap();
    assert!(f.on_disk(b"private-words-4711"), "written before");
    f.store.forget_conversation(&direct.id, None).unwrap();
    assert!(!f.on_disk(b"private-words-4711"), "a forgotten chat");

    f.store
        .set_secret(&bot.id, "API_KEY", "sk-1", None)
        .unwrap();
    let first = ciphertext(&f, &bot.id);
    assert!(f.on_disk(&first));
    f.store
        .set_secret(&bot.id, "API_KEY", "sk-2", None)
        .unwrap();
    assert!(!f.on_disk(&first), "a replaced value");
    let second = ciphertext(&f, &bot.id);
    f.store.delete_secret(&bot.id, "API_KEY").unwrap();
    assert!(!f.on_disk(&second), "a deleted secret");

    f.store
        .set_secret(&bot.id, "API_KEY", "sk-3", None)
        .unwrap();
    let third = ciphertext(&f, &bot.id);
    f.store.forget_bot(&bot.id, None).unwrap();
    assert!(!f.on_disk(&third), "a forgotten bot's secret");
}

#[test]
fn a_newer_schema_is_refused_instead_of_misread() {
    let f = fixture();
    f.raw().pragma_update(None, "user_version", 99).unwrap();
    let Err(error) = f.open_again(TestKey::default()) else {
        panic!("a schema from a newer version opened");
    };
    assert_eq!(error.code(), codes::STORAGE);
    assert!(error.message().contains("99"), "{error}");
    assert!(error.message().contains("update Flow-Like"), "{error}");
}

#[cfg(unix)]
#[test]
fn the_file_is_readable_by_its_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let f = fixture();
    let mode = std::fs::metadata(&f.path).unwrap().permissions().mode();
    assert_eq!(mode & 0o077, 0, "{mode:o}");
}

#[test]
fn changes_are_announced_after_their_commit_and_never_for_failures_or_reads() {
    let f = fixture();
    let bot = f.bot("Keeper");
    assert_eq!(
        f.changes.take_with_state(),
        [(
            Change::Bots(Some(bot.id.clone())),
            Committed {
                bots: 1,
                conversations: 0,
                entries: 0
            }
        )]
    );

    let direct = f.direct(&bot);
    f.text(&direct.id, &bot.id, "m_1");
    let announced = f.changes.take_with_state();
    let states: Vec<(Change, i64, i64)> = announced
        .into_iter()
        .map(|(change, state)| (change, state.conversations, state.entries))
        .collect();
    assert_eq!(
        states,
        [
            (Change::Inbox(Some(direct.id.clone())), 1, 0),
            (Change::Inbox(Some(direct.id.clone())), 1, 1)
        ]
    );

    let refused = f.store.create_bot(create(
        json!({ "name": " ", "appearance": { "kind": "pip" } }),
    ));
    assert!(refused.is_err());
    let stranger = f.send(
        &direct.id,
        "bot_stranger",
        "m_2",
        "message",
        json!({ "text": "Hi" }),
    );
    assert!(stranger.is_err());
    assert_eq!(
        code(
            f.store
                .update_bot(&bot.id, patch(json!({ "location": "server" })))
        ),
        codes::ACCOUNT_REQUIRED
    );
    f.store.list_bots().unwrap();
    f.store.conversations().unwrap();
    f.store
        .entries(&direct.id, &EntriesQuery::default())
        .unwrap();
    f.store.export_bot(&bot.id).unwrap();
    assert!(f.changes.take().is_empty());
    assert_eq!(f.store.get_bot(&bot.id).unwrap(), Some(bot));
}

#[test]
fn a_failed_write_rolls_back_what_it_did_and_announces_nothing() {
    let f = fixture();
    let bot = f.bot("Keeper");
    f.changes.take();
    let failed: Result<()> = f.store.write(|tx, changes| {
        tx.execute("DELETE FROM bots", [])?;
        changes.bots();
        Err(crate::Error::invalid("refused after writing"))
    });
    assert_eq!(code(failed), codes::INVALID);
    assert_eq!(committed(&f.path).bots, 1);
    assert_eq!(f.store.get_bot(&bot.id).unwrap(), Some(bot));
    assert!(f.changes.take().is_empty());
}
