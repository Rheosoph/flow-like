//! Usernames (`User.preferredUsername`): unique without regard to case, chosen by their
//! owner or derived for them. The sign-in provider seeds one for an account that has
//! none usable and never overwrites it afterwards.

use crate::{
    entity::user,
    routes::user::identity::{derive_handle, is_email_like, public_handle},
};
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, DbErr, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, SqlErr,
    sea_query::{Expr, ExprTrait, Func, SimpleExpr},
};
use serde::Serialize;
use std::collections::HashSet;
use utoipa::ToSchema;

/// Highest number appended to a derived username before giving up.
const MAX_SUFFIX: u32 = 9_999;

fn lowered_handle() -> SimpleExpr {
    Expr::expr(Func::lower(Expr::col(user::Column::PreferredUsername))).into()
}

pub(crate) fn is_unique_violation(err: &DbErr) -> bool {
    matches!(err.sql_err(), Some(SqlErr::UniqueConstraintViolation(_)))
}

/// Whether the stored username is one other people may see. Anything else (an address,
/// a provider id, nothing) is replaced.
pub(crate) fn usable_handle(stored: Option<&str>) -> bool {
    stored.and_then(public_handle).is_some()
}

/// Whether an account other than `owner` uses `handle`, ignoring case.
pub(crate) async fn handle_taken<C: ConnectionTrait>(
    db: &C,
    handle: &str,
    owner: &str,
) -> Result<bool, DbErr> {
    Ok(user::Entity::find()
        .select_only()
        .column(user::Column::Id)
        .filter(lowered_handle().eq(handle.to_lowercase()))
        .filter(user::Column::Id.ne(owner))
        .into_tuple::<String>()
        .one(db)
        .await?
        .is_some())
}

/// The first spelling of the derived `base` that no other account uses and `claimed`
/// does not hold: `base`, then `base-2`, `base-3`, … `base` is lowercase ASCII words
/// joined by `-`, so it carries no LIKE metacharacter.
pub(crate) async fn free_handle<C: ConnectionTrait>(
    db: &C,
    base: &str,
    owner: &str,
    claimed: &HashSet<String>,
) -> Result<Option<String>, DbErr> {
    let taken: HashSet<String> = user::Entity::find()
        .select_only()
        .column(user::Column::PreferredUsername)
        .filter(user::Column::Id.ne(owner))
        .filter(
            Condition::any()
                .add(lowered_handle().eq(base))
                .add(lowered_handle().like(format!("{base}-%"))),
        )
        .into_tuple::<Option<String>>()
        .all(db)
        .await?
        .into_iter()
        .flatten()
        .map(|handle| handle.to_lowercase())
        .collect();

    Ok(std::iter::once(base.to_string())
        .chain((2..=MAX_SUFFIX).map(|n| format!("{base}-{n}")))
        .find(|candidate| !taken.contains(candidate) && !claimed.contains(candidate)))
}

/// The username for an account without a usable one: the handle its sign-in carries when
/// that is usable and free, else one derived from its name or address, else none.
pub(crate) async fn handle_for<C: ConnectionTrait>(
    db: &C,
    owner: &str,
    signed_in_as: Option<&str>,
    name: Option<&str>,
    email: Option<&str>,
    claimed: &HashSet<String>,
) -> Result<Option<String>, DbErr> {
    if let Some(handle) = signed_in_as.and_then(public_handle)
        && !claimed.contains(&handle.to_lowercase())
        && !handle_taken(db, &handle, owner).await?
    {
        return Ok(Some(handle));
    }
    match derive_handle(name, email) {
        Some(base) => free_handle(db, &base, owner, claimed).await,
        None => Ok(None),
    }
}

/// What one pass over a page of accounts changed, or would change on a dry run.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
pub struct IdentityRepairReport {
    /// Accounts read.
    pub scanned: u64,
    /// Accounts given a username in place of an address, an account id or nothing.
    pub handles_set: u64,
    /// Accounts whose address or account id was removed from the username, with no
    /// usable name or address to derive a new one from. They choose one at sign-in.
    pub handles_cleared: u64,
    /// Accounts whose address was removed from their name.
    pub names_cleared: u64,
    /// Accounts that changed while the pass ran, or took a username another account
    /// took first. A later pass picks them up again.
    pub skipped: u64,
}

impl IdentityRepairReport {
    fn record(&mut self, before: Option<&str>, after: Option<&str>, name_cleared: bool) {
        if name_cleared {
            self.names_cleared += 1;
        }
        if before != after {
            match after {
                Some(_) => self.handles_set += 1,
                None => self.handles_cleared += 1,
            }
        }
    }
}

type AccountRow = (String, Option<String>, Option<String>, Option<String>);

/// Removes addresses and account ids from the names and usernames of up to `limit`
/// accounts after `after` (by id), deriving a username where one can be derived.
/// Returns the cursor of the next page, if there may be one. Each write is guarded by
/// the values it read, so a change made meanwhile (a person choosing a username) wins.
pub(crate) async fn repair_identities<C: ConnectionTrait>(
    db: &C,
    after: Option<&str>,
    limit: u64,
    dry_run: bool,
) -> Result<(IdentityRepairReport, Option<String>), DbErr> {
    let mut page = user::Entity::find().select_only().columns([
        user::Column::Id,
        user::Column::Name,
        user::Column::Email,
        user::Column::PreferredUsername,
    ]);
    if let Some(after) = after {
        page = page.filter(user::Column::Id.gt(after));
    }
    let rows = page
        .order_by_asc(user::Column::Id)
        .limit(limit)
        .into_tuple::<AccountRow>()
        .all(db)
        .await?;
    let next_cursor = (rows.len() as u64 == limit)
        .then(|| rows.last().map(|(id, ..)| id.clone()))
        .flatten();

    let mut report = IdentityRepairReport::default();
    let mut claimed = HashSet::new();
    for (id, name, email, handle) in rows {
        report.scanned += 1;
        let clear_name = name.as_deref().is_some_and(is_email_like);
        let new_handle = if usable_handle(handle.as_deref()) {
            handle.clone()
        } else {
            handle_for(db, &id, None, name.as_deref(), email.as_deref(), &claimed).await?
        };
        if !clear_name && new_handle == handle {
            continue;
        }

        if !dry_run {
            let mut update = user::Entity::update_many()
                .col_expr(
                    user::Column::UpdatedAt,
                    Expr::value(chrono::Utc::now().fixed_offset()),
                )
                .filter(user::Column::Id.eq(&id))
                .filter(match &handle {
                    Some(handle) => user::Column::PreferredUsername.eq(handle),
                    None => user::Column::PreferredUsername.is_null(),
                })
                .filter(match &name {
                    Some(name) => user::Column::Name.eq(name),
                    None => user::Column::Name.is_null(),
                });
            if clear_name {
                update = update.col_expr(user::Column::Name, Expr::value(Option::<String>::None));
            }
            if new_handle != handle {
                update = update.col_expr(
                    user::Column::PreferredUsername,
                    Expr::value(new_handle.clone()),
                );
            }
            match update.exec(db).await {
                Ok(result) if result.rows_affected == 1 => {}
                Ok(_) => {
                    report.skipped += 1;
                    continue;
                }
                Err(err) if is_unique_violation(&err) => {
                    report.skipped += 1;
                    continue;
                }
                Err(err) => return Err(err),
            }
        }

        if let Some(new_handle) = &new_handle {
            claimed.insert(new_handle.to_lowercase());
        }
        report.record(handle.as_deref(), new_handle.as_deref(), clear_name);
    }
    Ok((report, next_cursor))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{Database, DatabaseConnection};

    async fn database(
        rows: &[(&str, Option<&str>, Option<&str>, Option<&str>)],
    ) -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute_unprepared("ATTACH DATABASE ':memory:' AS public")
            .await
            .unwrap();
        db.execute_unprepared(
            r#"CREATE TABLE public."User" ("id" TEXT PRIMARY KEY, "name" TEXT, "email" TEXT,
            "preferredUsername" TEXT UNIQUE, "updatedAt" TEXT)"#,
        )
        .await
        .unwrap();
        for (id, name, email, handle) in rows {
            db.execute_raw(sea_orm::Statement::from_sql_and_values(
                sea_orm::DbBackend::Sqlite,
                r#"INSERT INTO public."User" ("id", "name", "email", "preferredUsername") VALUES (?, ?, ?, ?)"#,
                [(*id).into(), (*name).into(), (*email).into(), (*handle).into()],
            ))
            .await
            .unwrap();
        }
        db
    }

    async fn row(db: &DatabaseConnection, id: &str) -> (Option<String>, Option<String>) {
        user::Entity::find_by_id(id)
            .select_only()
            .columns([user::Column::Name, user::Column::PreferredUsername])
            .into_tuple::<(Option<String>, Option<String>)>()
            .one(db)
            .await
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    async fn a_derived_username_takes_the_first_free_spelling_regardless_of_case() {
        let db = database(&[
            ("a", None, None, Some("Felix-Schultz")),
            ("b", None, None, Some("felix-schultz-2")),
            ("c", None, None, Some("felix-schultzer")),
        ])
        .await;
        let none = HashSet::new();
        assert_eq!(
            free_handle(&db, "felix-schultz", "new", &none)
                .await
                .unwrap()
                .as_deref(),
            Some("felix-schultz-3")
        );
        // An account keeps the spelling it already has.
        assert_eq!(
            free_handle(&db, "felix-schultz", "a", &none)
                .await
                .unwrap()
                .as_deref(),
            Some("felix-schultz")
        );
        let claimed = HashSet::from(["felix-schultz-3".to_string()]);
        assert_eq!(
            free_handle(&db, "felix-schultz", "new", &claimed)
                .await
                .unwrap()
                .as_deref(),
            Some("felix-schultz-4")
        );
        assert!(handle_taken(&db, "FELIX-schultz", "new").await.unwrap());
        assert!(!handle_taken(&db, "felix-schultz", "a").await.unwrap());
    }

    #[tokio::test]
    async fn a_sign_in_handle_is_kept_when_free_and_replaced_when_taken() {
        let db = database(&[("a", None, None, Some("felix"))]).await;
        let none = HashSet::new();
        assert_eq!(
            handle_for(&db, "new", Some("ada"), None, None, &none)
                .await
                .unwrap()
                .as_deref(),
            Some("ada")
        );
        assert_eq!(
            handle_for(
                &db,
                "new",
                Some("Felix"),
                Some("Felix Schultz"),
                None,
                &none
            )
            .await
            .unwrap()
            .as_deref(),
            Some("felix-schultz")
        );
        for unusable in ["felix.schultz@example.com", "110293847561029384756"] {
            assert_eq!(
                handle_for(
                    &db,
                    "new",
                    Some(unusable),
                    None,
                    Some("felix.schultz@example.com"),
                    &none
                )
                .await
                .unwrap()
                .as_deref(),
                Some("felix-schultz"),
                "{unusable}"
            );
        }
        assert_eq!(
            handle_for(
                &db,
                "new",
                None,
                None,
                Some("x1y2@privaterelay.appleid.com"),
                &none
            )
            .await
            .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn a_repair_pass_removes_addresses_and_ids_and_keeps_chosen_usernames() {
        let db = database(&[
            (
                "1-google",
                Some("felix.schultz@gmail.com"),
                Some("felix.schultz@gmail.com"),
                Some("felix.schultz@gmail.com"),
            ),
            (
                "2-google",
                Some("felix.schultz@web.de"),
                Some("felix.schultz@web.de"),
                Some("felix.schultz@web.de"),
            ),
            (
                "3-apple",
                None,
                Some("x1y2@privaterelay.appleid.com"),
                Some("001234.9f8e7d6c5b4a.0123"),
            ),
            (
                "4-chosen",
                Some("Ada Lovelace"),
                Some("ada@example.com"),
                Some("ada"),
            ),
            (
                "5-named",
                Some("Grace Hopper"),
                Some("gh@example.com"),
                None,
            ),
            ("6-empty", None, None, None),
        ])
        .await;

        let (dry, _) = repair_identities(&db, None, 100, true).await.unwrap();
        assert_eq!(
            row(&db, "1-google").await.1.as_deref(),
            Some("felix.schultz@gmail.com")
        );

        let (report, next) = repair_identities(&db, None, 100, false).await.unwrap();
        assert_eq!(report, dry);
        assert_eq!(
            report,
            IdentityRepairReport {
                scanned: 6,
                handles_set: 3,
                handles_cleared: 1,
                names_cleared: 2,
                skipped: 0,
            }
        );
        assert_eq!(next, None);
        assert_eq!(
            row(&db, "1-google").await,
            (None, Some("felix-schultz".into()))
        );
        assert_eq!(
            row(&db, "2-google").await,
            (None, Some("felix-schultz-2".into()))
        );
        assert_eq!(row(&db, "3-apple").await, (None, None));
        assert_eq!(
            row(&db, "4-chosen").await,
            (Some("Ada Lovelace".into()), Some("ada".into()))
        );
        assert_eq!(
            row(&db, "5-named").await,
            (Some("Grace Hopper".into()), Some("grace-hopper".into()))
        );
        assert_eq!(row(&db, "6-empty").await, (None, None));

        let (again, _) = repair_identities(&db, None, 100, false).await.unwrap();
        assert_eq!(
            again,
            IdentityRepairReport {
                scanned: 6,
                ..Default::default()
            }
        );
    }

    #[tokio::test]
    async fn a_repair_pass_pages_by_id() {
        let db = database(&[
            ("a", None, Some("anna.berg@example.com"), None),
            ("b", None, Some("bernd.berg@example.com"), None),
            ("c", None, Some("carla.berg@example.com"), None),
        ])
        .await;
        let (first, cursor) = repair_identities(&db, None, 2, false).await.unwrap();
        assert_eq!((first.scanned, first.handles_set), (2, 2));
        assert_eq!(cursor.as_deref(), Some("b"));
        let (second, cursor) = repair_identities(&db, Some("b"), 2, false).await.unwrap();
        assert_eq!((second.scanned, second.handles_set), (1, 1));
        assert_eq!(cursor, None);
        assert_eq!(row(&db, "c").await.1.as_deref(), Some("carla-berg"));
    }
}
