//! Who a remote sink may run as. One rule holds when a sink is armed and when it
//! fires: the app is ACTIVE and, when the sink runs as a user, that account is
//! ACTIVE, a member of the app, and its role allows the required permission.

use super::trigger::{PatRefusal, PatRow, UNREADABLE_PAT, pat_row_owner, stored_pat_parts};
use crate::entity::event_sink;
use crate::permission::role_permission::{RolePermissions, has_role_permission};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, Condition, ConnectionTrait, DbErr, EntityTrait, QueryFilter,
    QueryResult, Statement, UpdateMany, prelude::DateTimeWithTimeZone,
};

const INVALID_ROLE_BITS: &str = "Invalid role permission bits";

const RUN_AS_AUTHORITY: &str = r#"
SELECT a.status = 'ACTIVE' AS app_active,
       u.status = 'ACTIVE' AS account_active,
       m.id IS NOT NULL AS member,
       r.permissions AS permissions
FROM "App" a
LEFT JOIN "User" u ON u.id = $2
LEFT JOIN "Membership" m ON m."appId" = a.id AND m."userId" = $2
LEFT JOIN "Role" r ON r.id = m."roleId" AND r."appId" = m."appId"
WHERE a.id = $1"#;

const SINK_AUTHORITY: &str = r#"
SELECT s."runAsUserId" AS run_as_user_id,
       s."patEncrypted" AS pat_encrypted,
       a.status = 'ACTIVE' AS app_active,
       u.status = 'ACTIVE' AS account_active,
       m.id IS NOT NULL AS member,
       r.permissions AS permissions
FROM "EventSink" s
JOIN "App" a ON a.id = s."appId"
LEFT JOIN "User" u ON u.id = s."runAsUserId"
LEFT JOIN "Membership" m ON m."appId" = s."appId" AND m."userId" = s."runAsUserId"
LEFT JOIN "Role" r ON r.id = m."roleId" AND r."appId" = m."appId"
WHERE s."appId" = $1 AND s."eventId" = $2 AND s.active = TRUE"#;

/// A legacy sink's stored PAT, looked up by id and secret hash, and its owner's standing.
const LEGACY_PAT_AUTHORITY: &str = r#"
SELECT p."userId" AS pat_user_id,
       p."validUntil" AS pat_valid_until,
       p.permissions AS pat_permissions,
       a.status = 'ACTIVE' AS app_active,
       u.status = 'ACTIVE' AS account_active,
       m.id IS NOT NULL AS member,
       r.permissions AS permissions
FROM "App" a
LEFT JOIN "PAT" p ON p.id = $2 AND p.key = $3
LEFT JOIN "User" u ON u.id = p."userId"
LEFT JOIN "Membership" m ON m."appId" = a.id AND m."userId" = p."userId"
LEFT JOIN "Role" r ON r.id = m."roleId" AND r."appId" = m."appId"
WHERE a.id = $1"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RunAsDenial {
    AppInactive,
    AccountInactive,
    NotMember,
    CannotTrigger,
    TokenInvalid,
}

impl RunAsDenial {
    /// Member-facing reason for rejection records, logs and suite errors.
    pub(crate) const fn reason(self) -> &'static str {
        match self {
            Self::AppInactive => "The project is not active",
            Self::AccountInactive => {
                "The account this event runs as is no longer active. Turn the event back on to run it as you."
            }
            Self::NotMember => {
                "The user this event runs as is no longer a project member. Turn the event back on to run it as you."
            }
            Self::CannotTrigger => {
                "The role of the user this event runs as does not allow triggering events. Turn the event back on to run it as you."
            }
            Self::TokenInvalid => {
                "The access token this event ran with expired, was revoked or no longer allows running workflows. Turn the event back on to run it as you."
            }
        }
    }

    /// What callers of the sink see; never names a user or an id.
    pub(crate) const fn public_message(self) -> &'static str {
        if self.pauses() {
            "This event is paused"
        } else {
            "This event is not available"
        }
    }

    /// Whether the refusal pauses the sink. An inactive app is the project's
    /// state, not the run-as user's fault, so its sinks stay on.
    pub(crate) const fn pauses(self) -> bool {
        !matches!(self, Self::AppInactive)
    }

    /// Body of the notification to the app's owners and admins.
    pub(crate) fn notice(self, who: &str, events: &[String]) -> String {
        const PAUSED_FOLLOW_UP: &str =
            "Runs that people start keep working; turn an event back on to run the rest as you.";
        let events = events.join(", ");
        match self {
            Self::AppInactive => {
                format!("These events did not run because the project is not active: {events}.")
            }
            Self::AccountInactive => format!(
                "{who}'s account is no longer active and these events ran as them: {events}. {PAUSED_FOLLOW_UP}"
            ),
            Self::NotMember => format!(
                "{who} is no longer a member and these events ran as them: {events}. {PAUSED_FOLLOW_UP}"
            ),
            Self::CannotTrigger => format!(
                "{who}'s role does not allow triggering events and these events ran as them: {events}. {PAUSED_FOLLOW_UP}"
            ),
            Self::TokenInvalid => format!(
                "The access token these events ran with expired or was revoked: {events}. {PAUSED_FOLLOW_UP}"
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RunAsVerdict {
    /// The run-as user's role permissions; empty when no user was given.
    Allowed(RolePermissions),
    Denied(RunAsDenial),
    /// A server-side fault that may clear on retry. It never pauses a sink.
    Transient(&'static str),
}

/// One row of the authority statement. The default stands for a missing app.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RunAsFacts {
    pub(crate) app_active: bool,
    pub(crate) account_active: bool,
    pub(crate) member: bool,
    pub(crate) permissions: Option<i64>,
}

/// The run-as user of an active sink and whether it may trigger events now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SinkAuthority {
    pub(crate) run_as_user_id: Option<String>,
    pub(crate) verdict: RunAsVerdict,
}

/// An inactive app beats any user problem, and without a user only the app counts.
/// Owner and Admin imply `required`.
pub(crate) fn classify_run_as(
    facts: RunAsFacts,
    has_user: bool,
    required: RolePermissions,
) -> RunAsVerdict {
    if !facts.app_active {
        return RunAsVerdict::Denied(RunAsDenial::AppInactive);
    }
    if !has_user {
        return RunAsVerdict::Allowed(RolePermissions::empty());
    }
    if !facts.account_active {
        return RunAsVerdict::Denied(RunAsDenial::AccountInactive);
    }
    if !facts.member {
        return RunAsVerdict::Denied(RunAsDenial::NotMember);
    }
    let Some(bits) = facts.permissions else {
        return RunAsVerdict::Denied(RunAsDenial::CannotTrigger);
    };
    match RolePermissions::from_bits(bits) {
        None => RunAsVerdict::Transient(INVALID_ROLE_BITS),
        Some(role) if has_role_permission(&role, required) => RunAsVerdict::Allowed(role),
        Some(_) => RunAsVerdict::Denied(RunAsDenial::CannotTrigger),
    }
}

fn facts(row: &QueryResult) -> Result<RunAsFacts, DbErr> {
    let flag = |column: &str| {
        row.try_get::<Option<bool>>("", column)
            .map(|value| value.unwrap_or(false))
    };
    Ok(RunAsFacts {
        app_active: flag("app_active")?,
        account_active: flag("account_active")?,
        member: flag("member")?,
        permissions: row.try_get("", "permissions")?,
    })
}

async fn authority_facts<C: ConnectionTrait>(
    db: &C,
    app_id: &str,
    user_id: Option<&str>,
) -> Result<RunAsFacts, DbErr> {
    let row = db
        .query_one_raw(Statement::from_sql_and_values(
            db.get_database_backend(),
            RUN_AS_AUTHORITY,
            [app_id.into(), user_id.map(str::to_owned).into()],
        ))
        .await?;
    Ok(row.as_ref().map(facts).transpose()?.unwrap_or_default())
}

/// Fresh from the database on every call, so a revocation applies on the next
/// check on every replica.
pub(crate) async fn run_as_authority<C: ConnectionTrait>(
    db: &C,
    app_id: &str,
    user_id: Option<&str>,
    required: RolePermissions,
) -> Result<RunAsVerdict, DbErr> {
    let facts = authority_facts(db, app_id, user_id).await?;
    Ok(classify_run_as(facts, user_id.is_some(), required))
}

/// The actor's own standing when they arm a sink, whatever state the app is in: a
/// sink armed while the project is not active waits and fires once it is active.
pub(crate) async fn actor_authority<C: ConnectionTrait>(
    db: &C,
    app_id: &str,
    actor: &str,
    required: RolePermissions,
) -> Result<RunAsVerdict, DbErr> {
    let facts = authority_facts(db, app_id, Some(actor)).await?;
    Ok(classify_run_as(arming_facts(facts), true, required))
}

const fn arming_facts(facts: RunAsFacts) -> RunAsFacts {
    RunAsFacts {
        app_active: true,
        ..facts
    }
}

/// `None` when the event has no active sink in the app. A sink saved before run-as
/// identities runs as the owner of its stored PAT while that PAT validates, as on every
/// fire, at the cost of one more read.
pub(crate) async fn sink_authority<C: ConnectionTrait>(
    db: &C,
    encryption_key: &[u8; 32],
    app_id: &str,
    event_id: &str,
) -> Result<Option<SinkAuthority>, DbErr> {
    let Some(row) = db
        .query_one_raw(Statement::from_sql_and_values(
            db.get_database_backend(),
            SINK_AUTHORITY,
            [app_id.into(), event_id.into()],
        ))
        .await?
    else {
        return Ok(None);
    };
    let run_as_user_id: Option<String> = row.try_get("", "run_as_user_id")?;
    let pat_encrypted: Option<String> = row.try_get("", "pat_encrypted")?;
    let facts = facts(&row)?;
    if let (None, Some(ciphertext), true) = (&run_as_user_id, &pat_encrypted, facts.app_active) {
        let owner = legacy_owner(db, encryption_key, app_id, ciphertext).await?;
        return Ok(Some(legacy_authority(owner)));
    }
    let verdict = classify_run_as(
        facts,
        run_as_user_id.is_some(),
        RolePermissions::ExecuteEvents,
    );
    Ok(Some(SinkAuthority {
        run_as_user_id,
        verdict,
    }))
}

/// The owner a legacy sink's stored PAT still names, checked like the fire checks it, with
/// that owner's standing in the app.
async fn legacy_owner<C: ConnectionTrait>(
    db: &C,
    encryption_key: &[u8; 32],
    app_id: &str,
    ciphertext: &str,
) -> Result<Result<(String, RunAsFacts), PatRefusal>, DbErr> {
    let token = crate::routes::app::events::db::decrypt_token(ciphertext, encryption_key);
    let (pat_id, secret) = match stored_pat_parts(token.as_deref()) {
        Ok(parts) => parts,
        Err(refusal) => return Ok(Err(refusal)),
    };
    let secret_hash = blake3::hash(secret.as_bytes()).to_hex().to_string();
    let row = db
        .query_one_raw(Statement::from_sql_and_values(
            db.get_database_backend(),
            LEGACY_PAT_AUTHORITY,
            [app_id.into(), pat_id.into(), secret_hash.into()],
        ))
        .await?;
    let (pat, owner) = match &row {
        Some(row) => (legacy_pat(row)?, facts(row)?),
        None => (None, RunAsFacts::default()),
    };
    Ok(pat_row_owner(pat, chrono::Utc::now().fixed_offset()).map(|user_id| (user_id, owner)))
}

fn legacy_pat(row: &QueryResult) -> Result<Option<PatRow>, DbErr> {
    let user_id: Option<String> = row.try_get("", "pat_user_id")?;
    let valid_until: Option<DateTimeWithTimeZone> = row.try_get("", "pat_valid_until")?;
    let permissions: Option<i64> = row.try_get("", "pat_permissions")?;
    Ok(user_id
        .zip(permissions)
        .map(|(user_id, permissions)| (user_id, valid_until, permissions)))
}

/// A legacy sink runs as the owner its PAT names, judged like any run-as user. A PAT that
/// names nobody lends nothing, and one this server cannot read is a fault, not a refusal.
pub(crate) fn legacy_authority(owner: Result<(String, RunAsFacts), PatRefusal>) -> SinkAuthority {
    match owner {
        Ok((user_id, facts)) => SinkAuthority {
            run_as_user_id: Some(user_id),
            verdict: classify_run_as(facts, true, RolePermissions::ExecuteEvents),
        },
        Err(PatRefusal::Invalid) => SinkAuthority {
            run_as_user_id: None,
            verdict: RunAsVerdict::Denied(RunAsDenial::TokenInvalid),
        },
        Err(PatRefusal::Unreadable) => SinkAuthority {
            run_as_user_id: None,
            verdict: RunAsVerdict::Transient(UNREADABLE_PAT),
        },
    }
}

/// Makes `actor` the run-as user of the app's active sinks for `event_ids` and drops
/// their legacy PAT. Sinks that ran as someone else, or as no one, also lose the
/// OAuth grants and profile snapshot of that identity. Returns how many sinks now
/// run as `actor`.
pub(crate) async fn bind_run_as<C: ConnectionTrait>(
    db: &C,
    app_id: &str,
    event_ids: &[String],
    actor: &str,
) -> Result<u64, DbErr> {
    if event_ids.is_empty() {
        return Ok(0);
    }
    let now = chrono::Utc::now().fixed_offset();
    let kept = rebind_same_person(app_id, event_ids, actor, now)
        .exec(db)
        .await?
        .rows_affected;
    let rebound = rebind_other_person(app_id, event_ids, actor, now)
        .exec(db)
        .await?
        .rows_affected;
    Ok(kept + rebound)
}

fn active_sinks(app_id: &str, event_ids: &[String]) -> UpdateMany<event_sink::Entity> {
    event_sink::Entity::update_many()
        .filter(event_sink::Column::AppId.eq(app_id))
        .filter(event_sink::Column::EventId.is_in(event_ids.iter().map(String::as_str)))
        .filter(event_sink::Column::Active.eq(true))
}

fn rebind_same_person(
    app_id: &str,
    event_ids: &[String],
    actor: &str,
    now: DateTimeWithTimeZone,
) -> UpdateMany<event_sink::Entity> {
    active_sinks(app_id, event_ids)
        .set(event_sink::ActiveModel {
            pat_encrypted: Set(None),
            updated_at: Set(now),
            ..Default::default()
        })
        .filter(event_sink::Column::RunAsUserId.eq(actor))
}

fn rebind_other_person(
    app_id: &str,
    event_ids: &[String],
    actor: &str,
    now: DateTimeWithTimeZone,
) -> UpdateMany<event_sink::Entity> {
    active_sinks(app_id, event_ids)
        .set(event_sink::ActiveModel {
            run_as_user_id: Set(Some(actor.to_owned())),
            pat_encrypted: Set(None),
            oauth_tokens_encrypted: Set(None),
            profile_json: Set(None),
            updated_at: Set(now),
            ..Default::default()
        })
        .filter(
            Condition::any()
                .add(event_sink::Column::RunAsUserId.is_null())
                .add(event_sink::Column::RunAsUserId.ne(actor)),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{DbBackend, QueryTrait};

    const EXECUTE: RolePermissions = RolePermissions::ExecuteEvents;
    const EDITOR: RolePermissions = RolePermissions::ListEvents
        .union(RolePermissions::ReadEvents)
        .union(RolePermissions::WriteEvents);
    const INVALID_BITS: i64 = 1 << 62;

    fn member(permissions: RolePermissions) -> RunAsFacts {
        RunAsFacts {
            app_active: true,
            account_active: true,
            member: true,
            permissions: Some(permissions.bits()),
        }
    }

    #[test]
    fn classify_run_as_matrix() {
        let denied = RunAsVerdict::Denied;
        let inactive_app = RunAsFacts {
            app_active: false,
            ..member(EXECUTE)
        };
        let outsider = RunAsFacts {
            member: false,
            permissions: None,
            ..member(EXECUTE)
        };
        for (case, facts, has_user, required, expected) in [
            (
                "inactive app beats a banned user",
                RunAsFacts {
                    account_active: false,
                    ..inactive_app
                },
                true,
                EXECUTE,
                denied(RunAsDenial::AppInactive),
            ),
            (
                "missing app",
                RunAsFacts::default(),
                true,
                EXECUTE,
                denied(RunAsDenial::AppInactive),
            ),
            (
                "inactive app without a user",
                inactive_app,
                false,
                EXECUTE,
                denied(RunAsDenial::AppInactive),
            ),
            (
                "banned, inactive or missing account",
                RunAsFacts {
                    account_active: false,
                    ..member(EXECUTE)
                },
                true,
                EXECUTE,
                denied(RunAsDenial::AccountInactive),
            ),
            (
                "an inactive account beats a missing membership",
                RunAsFacts {
                    account_active: false,
                    ..outsider
                },
                true,
                EXECUTE,
                denied(RunAsDenial::AccountInactive),
            ),
            (
                "non-member",
                outsider,
                true,
                EXECUTE,
                denied(RunAsDenial::NotMember),
            ),
            (
                "write without execute",
                member(EDITOR),
                true,
                EXECUTE,
                denied(RunAsDenial::CannotTrigger),
            ),
            (
                "membership without a role in the app",
                RunAsFacts {
                    permissions: None,
                    ..member(EXECUTE)
                },
                true,
                EXECUTE,
                denied(RunAsDenial::CannotTrigger),
            ),
            (
                "execute",
                member(EXECUTE),
                true,
                EXECUTE,
                RunAsVerdict::Allowed(EXECUTE),
            ),
            (
                "owner",
                member(RolePermissions::Owner),
                true,
                EXECUTE,
                RunAsVerdict::Allowed(RolePermissions::Owner),
            ),
            (
                "admin",
                member(RolePermissions::Admin),
                true,
                EXECUTE,
                RunAsVerdict::Allowed(RolePermissions::Admin),
            ),
            (
                "invalid bits",
                RunAsFacts {
                    permissions: Some(INVALID_BITS),
                    ..member(EXECUTE)
                },
                true,
                EXECUTE,
                RunAsVerdict::Transient("Invalid role permission bits"),
            ),
            (
                "no user with an active app",
                RunAsFacts {
                    account_active: false,
                    ..outsider
                },
                false,
                EXECUTE,
                RunAsVerdict::Allowed(RolePermissions::empty()),
            ),
            (
                "membership alone when nothing is required",
                member(EDITOR),
                true,
                RolePermissions::empty(),
                RunAsVerdict::Allowed(EDITOR),
            ),
        ] {
            assert_eq!(
                classify_run_as(facts, has_user, required),
                expected,
                "{case}"
            );
        }
    }

    /// A legacy sink lends what the owner its PAT names may do, like a run-as user, and
    /// nobody once the PAT names nobody; an unreadable PAT is a fault, never a refusal.
    #[test]
    fn legacy_sinks_run_as_the_owner_their_token_still_names() {
        use RunAsDenial::{AccountInactive, CannotTrigger, NotMember, TokenInvalid};
        use RunAsVerdict::{Allowed, Denied, Transient};
        let owner = RolePermissions::Owner;
        let left = RunAsFacts {
            member: false,
            permissions: None,
            ..member(EXECUTE)
        };
        let banned = RunAsFacts {
            account_active: false,
            ..member(EXECUTE)
        };
        for (pat, run_as, verdict) in [
            (Ok(member(EXECUTE)), Some("owner"), Allowed(EXECUTE)),
            (Ok(member(owner)), Some("owner"), Allowed(owner)),
            (Ok(member(EDITOR)), Some("owner"), Denied(CannotTrigger)),
            (Ok(left), Some("owner"), Denied(NotMember)),
            (Ok(banned), Some("owner"), Denied(AccountInactive)),
            (Err(PatRefusal::Invalid), None, Denied(TokenInvalid)),
            (Err(PatRefusal::Unreadable), None, Transient(UNREADABLE_PAT)),
        ] {
            let expected = SinkAuthority {
                run_as_user_id: run_as.map(str::to_owned),
                verdict,
            };
            let named = pat.map(|facts| ("owner".to_owned(), facts));
            assert_eq!(legacy_authority(named), expected, "{pat:?}");
        }
    }

    /// Arming judges the actor alone: the project's state is the fire's business, so an
    /// actor who could fire the sink may arm it while the project is not active, and one
    /// who could not is refused whatever the project's state.
    #[test]
    fn arming_ignores_only_the_project_state() {
        let inactive = |facts: RunAsFacts| RunAsFacts {
            app_active: false,
            ..facts
        };
        assert_eq!(
            classify_run_as(arming_facts(inactive(member(EXECUTE))), true, EXECUTE),
            RunAsVerdict::Allowed(EXECUTE)
        );
        assert_eq!(
            classify_run_as(arming_facts(inactive(member(EDITOR))), true, EXECUTE),
            RunAsVerdict::Denied(RunAsDenial::CannotTrigger)
        );
        let banned = RunAsFacts {
            account_active: false,
            ..member(EXECUTE)
        };
        assert_eq!(
            classify_run_as(arming_facts(inactive(banned)), true, EXECUTE),
            RunAsVerdict::Denied(RunAsDenial::AccountInactive)
        );
        assert_eq!(
            classify_run_as(arming_facts(RunAsFacts::default()), true, EXECUTE),
            RunAsVerdict::Denied(RunAsDenial::AccountInactive),
            "a missing project row arms nobody"
        );
    }

    #[test]
    fn denials_explain_themselves_per_audience() {
        let events = ["Nightly".to_string(), "Inbox".to_string()];
        for (denial, reason, notice) in [
            (
                RunAsDenial::AppInactive,
                "The project is not active",
                "These events did not run because the project is not active: Nightly, Inbox.",
            ),
            (
                RunAsDenial::AccountInactive,
                "The account this event runs as is no longer active. Turn the event back on to run it as you.",
                "Ada's account is no longer active and these events ran as them: Nightly, Inbox. Runs that people start keep working; turn an event back on to run the rest as you.",
            ),
            (
                RunAsDenial::NotMember,
                "The user this event runs as is no longer a project member. Turn the event back on to run it as you.",
                "Ada is no longer a member and these events ran as them: Nightly, Inbox. Runs that people start keep working; turn an event back on to run the rest as you.",
            ),
            (
                RunAsDenial::CannotTrigger,
                "The role of the user this event runs as does not allow triggering events. Turn the event back on to run it as you.",
                "Ada's role does not allow triggering events and these events ran as them: Nightly, Inbox. Runs that people start keep working; turn an event back on to run the rest as you.",
            ),
            (
                RunAsDenial::TokenInvalid,
                "The access token this event ran with expired, was revoked or no longer allows running workflows. Turn the event back on to run it as you.",
                "The access token these events ran with expired or was revoked: Nightly, Inbox. Runs that people start keep working; turn an event back on to run the rest as you.",
            ),
        ] {
            assert_eq!(denial.reason(), reason, "{denial:?}");
            assert_eq!(denial.notice("Ada", &events), notice, "{denial:?}");
            let pauses = denial != RunAsDenial::AppInactive;
            assert_eq!(denial.pauses(), pauses, "{denial:?}");
            let public = if pauses {
                "This event is paused"
            } else {
                "This event is not available"
            };
            assert_eq!(denial.public_message(), public, "{denial:?}");
        }
    }

    #[test]
    fn rebinding_keeps_credentials_only_for_the_same_person() {
        let events = ["event-1".to_string(), "event-2".to_string()];
        let now = chrono::Utc::now().fixed_offset();
        let statement = |same_person| {
            let rebind = if same_person {
                rebind_same_person
            } else {
                rebind_other_person
            };
            let sql = rebind("app-1", &events, "actor", now)
                .build(DbBackend::Postgres)
                .to_string();
            assert!(!sql.contains("CASE"), "{sql}");
            let (set, filter) = sql.split_once(" WHERE ").expect("filtered update");
            for fragment in [
                r#""appId" = 'app-1'"#,
                r#""eventId" IN ('event-1', 'event-2')"#,
                r#""active" = TRUE"#,
            ] {
                assert!(filter.contains(fragment), "{fragment} missing from {sql}");
            }
            assert!(set.contains(r#""patEncrypted" = NULL"#), "{sql}");
            (set.to_owned(), filter.to_owned())
        };

        let (set, filter) = statement(true);
        assert!(filter.contains(r#""runAsUserId" = 'actor'"#), "{filter}");
        for column in ["runAsUserId", "oauthTokensEncrypted", "profileJson"] {
            assert!(!set.contains(column), "{column} in {set}");
        }

        let (set, filter) = statement(false);
        for fragment in [
            r#""runAsUserId" = 'actor'"#,
            r#""oauthTokensEncrypted" = NULL"#,
            r#""profileJson" = NULL"#,
        ] {
            assert!(set.contains(fragment), "{fragment} missing from {set}");
        }
        for fragment in [
            r#""runAsUserId" IS NULL OR "#,
            r#""runAsUserId" <> 'actor'"#,
        ] {
            assert!(
                filter.contains(fragment),
                "{fragment} missing from {filter}"
            );
        }
    }

    mod postgres {
        use super::*;
        use sea_orm::{Database, DatabaseConnection, DatabaseTransaction, TransactionTrait};

        const KEY: [u8; 32] = [7; 32];

        const SCHEMA: &str = r#"
            CREATE TABLE public."App" (id text PRIMARY KEY, status text NOT NULL);
            CREATE TABLE public."User" (id text PRIMARY KEY, status text NOT NULL);
            CREATE TABLE public."Role" (id text PRIMARY KEY, "appId" text, permissions bigint NOT NULL);
            CREATE TABLE public."Membership" (
                id text PRIMARY KEY, "userId" text NOT NULL, "appId" text NOT NULL, "roleId" text NOT NULL
            );
            CREATE TABLE public."EventSink" (
                id text PRIMARY KEY, "eventId" text NOT NULL UNIQUE, "appId" text NOT NULL,
                active boolean NOT NULL, "runAsUserId" text, "patEncrypted" text,
                "oauthTokensEncrypted" text, "profileJson" jsonb,
                "updatedAt" timestamptz(3) NOT NULL DEFAULT now()
            );
            CREATE TABLE public."PAT" (
                id text PRIMARY KEY, key text NOT NULL UNIQUE, permissions bigint NOT NULL,
                "userId" text NOT NULL, "validUntil" timestamptz(3)
            );
            INSERT INTO public."App" VALUES ('app', 'ACTIVE'), ('archived', 'ARCHIVED'), ('other-app', 'ACTIVE');
            INSERT INTO public."User" VALUES
                ('executor', 'ACTIVE'), ('owner', 'ACTIVE'), ('admin', 'ACTIVE'), ('editor', 'ACTIVE'),
                ('banned', 'BANNED'), ('deactivated', 'INACTIVE'), ('outsider', 'ACTIVE'),
                ('corrupt', 'ACTIVE'), ('foreign-role', 'ACTIVE');
            INSERT INTO public."Membership" VALUES
                ('m-executor', 'executor', 'app', 'role-execute'),
                ('m-owner', 'owner', 'app', 'role-owner'),
                ('m-admin', 'admin', 'app', 'role-admin'),
                ('m-editor', 'editor', 'app', 'role-editor'),
                ('m-banned', 'banned', 'app', 'role-execute'),
                ('m-deactivated', 'deactivated', 'app', 'role-execute'),
                ('m-missing', 'missing', 'app', 'role-execute'),
                ('m-corrupt', 'corrupt', 'app', 'role-corrupt'),
                ('m-foreign-role', 'foreign-role', 'app', 'role-other-app'),
                ('m-archived-executor', 'executor', 'archived', 'role-archived'),
                ('m-archived-banned', 'banned', 'archived', 'role-archived');
        "#;

        async fn seeded() -> (DatabaseConnection, DatabaseTransaction) {
            let url = std::env::var("FLOW_LIKE_SINK_TEST_DATABASE_URL").unwrap();
            let db = Database::connect(url).await.unwrap();
            let txn = db.begin().await.unwrap();
            txn.execute_unprepared(SCHEMA).await.unwrap();
            for (id, app, bits) in [
                ("role-execute", "app", EXECUTE.bits()),
                ("role-owner", "app", RolePermissions::Owner.bits()),
                ("role-admin", "app", RolePermissions::Admin.bits()),
                ("role-editor", "app", EDITOR.bits()),
                ("role-corrupt", "app", INVALID_BITS),
                ("role-other-app", "other-app", EXECUTE.bits()),
                ("role-archived", "archived", EXECUTE.bits()),
            ] {
                txn.execute_raw(Statement::from_sql_and_values(
                    DbBackend::Postgres,
                    r#"INSERT INTO public."Role" VALUES ($1, $2, $3)"#,
                    [id.into(), app.into(), bits.into()],
                ))
                .await
                .unwrap();
            }
            (db, txn)
        }

        fn matrix() -> Vec<(&'static str, Option<&'static str>, RunAsVerdict)> {
            let denied = RunAsVerdict::Denied;
            vec![
                ("app", Some("executor"), RunAsVerdict::Allowed(EXECUTE)),
                (
                    "app",
                    Some("owner"),
                    RunAsVerdict::Allowed(RolePermissions::Owner),
                ),
                (
                    "app",
                    Some("admin"),
                    RunAsVerdict::Allowed(RolePermissions::Admin),
                ),
                ("app", Some("editor"), denied(RunAsDenial::CannotTrigger)),
                ("app", Some("banned"), denied(RunAsDenial::AccountInactive)),
                (
                    "app",
                    Some("deactivated"),
                    denied(RunAsDenial::AccountInactive),
                ),
                ("app", Some("missing"), denied(RunAsDenial::AccountInactive)),
                ("app", Some("outsider"), denied(RunAsDenial::NotMember)),
                (
                    "app",
                    Some("foreign-role"),
                    denied(RunAsDenial::CannotTrigger),
                ),
                (
                    "app",
                    Some("corrupt"),
                    RunAsVerdict::Transient("Invalid role permission bits"),
                ),
                ("app", None, RunAsVerdict::Allowed(RolePermissions::empty())),
                ("archived", Some("banned"), denied(RunAsDenial::AppInactive)),
                (
                    "archived",
                    Some("executor"),
                    denied(RunAsDenial::AppInactive),
                ),
                ("archived", None, denied(RunAsDenial::AppInactive)),
            ]
        }

        #[tokio::test]
        #[ignore = "requires FLOW_LIKE_SINK_TEST_DATABASE_URL pointing to disposable PostgreSQL"]
        async fn run_as_authority_reads_the_matrix_from_the_database() {
            let (_db, txn) = seeded().await;
            for (app, user, expected) in matrix() {
                assert_eq!(
                    run_as_authority(&txn, app, user, EXECUTE).await.unwrap(),
                    expected,
                    "{app} {user:?}"
                );
            }
            assert_eq!(
                run_as_authority(&txn, "no-such-app", None, EXECUTE)
                    .await
                    .unwrap(),
                RunAsVerdict::Denied(RunAsDenial::AppInactive)
            );
            assert_eq!(
                run_as_authority(&txn, "app", Some("editor"), RolePermissions::empty())
                    .await
                    .unwrap(),
                RunAsVerdict::Allowed(EDITOR)
            );
            txn.rollback().await.unwrap();
        }

        #[tokio::test]
        #[ignore = "requires FLOW_LIKE_SINK_TEST_DATABASE_URL pointing to disposable PostgreSQL"]
        async fn actor_authority_reads_the_actor_whatever_the_project_state() {
            let (_db, txn) = seeded().await;
            for (app, actor, expected) in [
                ("archived", "executor", RunAsVerdict::Allowed(EXECUTE)),
                (
                    "archived",
                    "banned",
                    RunAsVerdict::Denied(RunAsDenial::AccountInactive),
                ),
                (
                    "app",
                    "editor",
                    RunAsVerdict::Denied(RunAsDenial::CannotTrigger),
                ),
                (
                    "app",
                    "outsider",
                    RunAsVerdict::Denied(RunAsDenial::NotMember),
                ),
                (
                    "app",
                    "owner",
                    RunAsVerdict::Allowed(RolePermissions::Owner),
                ),
            ] {
                assert_eq!(
                    actor_authority(&txn, app, actor, EXECUTE).await.unwrap(),
                    expected,
                    "{app} {actor}"
                );
            }
            txn.rollback().await.unwrap();
        }

        #[tokio::test]
        #[ignore = "requires FLOW_LIKE_SINK_TEST_DATABASE_URL pointing to disposable PostgreSQL"]
        async fn sink_authority_reads_the_run_as_user_of_active_sinks_only() {
            let (_db, txn) = seeded().await;
            for (index, (app, user, _)) in matrix().into_iter().enumerate() {
                txn.execute_raw(Statement::from_sql_and_values(
                    DbBackend::Postgres,
                    r#"INSERT INTO public."EventSink" (id, "eventId", "appId", active, "runAsUserId")
                       VALUES ($1, $1, $2, TRUE, $3)"#,
                    [
                        format!("event-{index}").into(),
                        app.into(),
                        user.map(str::to_owned).into(),
                    ],
                ))
                .await
                .unwrap();
            }
            txn.execute_unprepared(
                r#"INSERT INTO public."EventSink" (id, "eventId", "appId", active, "runAsUserId")
                   VALUES ('paused', 'paused', 'app', FALSE, 'executor')"#,
            )
            .await
            .unwrap();

            for (index, (app, user, expected)) in matrix().into_iter().enumerate() {
                assert_eq!(
                    sink_authority(&txn, &KEY, app, &format!("event-{index}"))
                        .await
                        .unwrap(),
                    Some(SinkAuthority {
                        run_as_user_id: user.map(str::to_owned),
                        verdict: expected,
                    }),
                    "{app} {user:?}"
                );
            }
            for (app, event) in [
                ("app", "paused"),
                ("other-app", "event-0"),
                ("app", "unknown"),
            ] {
                assert_eq!(
                    sink_authority(&txn, &KEY, app, event).await.unwrap(),
                    None,
                    "{app} {event}"
                );
            }
            txn.rollback().await.unwrap();
        }

        /// A sink saved before run-as identities runs as the owner of its stored PAT, judged
        /// like any run-as user; a run-as user beats a stored PAT, an inactive project both.
        #[tokio::test]
        #[ignore = "requires FLOW_LIKE_SINK_TEST_DATABASE_URL pointing to disposable PostgreSQL"]
        async fn sink_authority_runs_legacy_sinks_as_the_owner_of_a_current_token() {
            use RunAsVerdict::{Allowed, Denied};
            let (_db, txn) = seeded().await;
            seed_pats(&txn).await;
            let sink = |user: Option<&str>, verdict| SinkAuthority {
                run_as_user_id: user.map(str::to_owned),
                verdict,
            };
            for (event, app, run_as, pat_id, expected) in [
                (
                    "legacy",
                    "app",
                    None,
                    "pat-executor",
                    sink(Some("executor"), Allowed(EXECUTE)),
                ),
                (
                    "editor",
                    "app",
                    None,
                    "pat-editor",
                    sink(Some("editor"), Denied(RunAsDenial::CannotTrigger)),
                ),
                (
                    "bound",
                    "app",
                    Some("owner"),
                    "pat-executor",
                    sink(Some("owner"), Allowed(RolePermissions::Owner)),
                ),
                (
                    "archived",
                    "archived",
                    None,
                    "pat-executor",
                    sink(None, Denied(RunAsDenial::AppInactive)),
                ),
            ] {
                legacy_sink(&txn, event, app, run_as, current_pat(pat_id)).await;
                let authority = sink_authority(&txn, &KEY, app, event).await.unwrap();
                assert_eq!(authority, Some(expected), "{event}");
            }
            txn.rollback().await.unwrap();
        }

        /// An expired, read-only, revoked or mistyped stored PAT names nobody, and one this
        /// server cannot decrypt is a fault the caller may retry.
        #[tokio::test]
        #[ignore = "requires FLOW_LIKE_SINK_TEST_DATABASE_URL pointing to disposable PostgreSQL"]
        async fn sink_authority_lends_nothing_for_a_token_that_names_nobody() {
            use crate::routes::app::events::db::encrypt_token;
            let (_db, txn) = seeded().await;
            seed_pats(&txn).await;
            let invalid = RunAsVerdict::Denied(RunAsDenial::TokenInvalid);
            for (event, pat, verdict) in [
                ("expired", current_pat("pat-expired"), invalid),
                ("read-only", current_pat("pat-read-only"), invalid),
                ("revoked", current_pat("pat-deleted"), invalid),
                (
                    "wrong-secret",
                    stored_pat("pat-executor", "pat-editor-secret"),
                    invalid,
                ),
                ("malformed", encrypt_token("not-a-pat", &KEY), invalid),
                (
                    "unreadable",
                    "not-a-ciphertext".to_owned(),
                    RunAsVerdict::Transient(UNREADABLE_PAT),
                ),
            ] {
                legacy_sink(&txn, event, "app", None, pat).await;
                let authority = sink_authority(&txn, &KEY, "app", event).await.unwrap();
                let expected = SinkAuthority {
                    run_as_user_id: None,
                    verdict,
                };
                assert_eq!(authority, Some(expected), "{event}");
            }
            txn.rollback().await.unwrap();
        }

        #[tokio::test]
        #[ignore = "requires FLOW_LIKE_SINK_TEST_DATABASE_URL pointing to disposable PostgreSQL"]
        async fn bind_run_as_clears_credentials_only_when_the_person_changes() {
            let (_db, txn) = seeded().await;
            txn.execute_unprepared(
                r#"INSERT INTO public."EventSink"
                       (id, "eventId", "appId", active, "runAsUserId", "patEncrypted", "oauthTokensEncrypted", "profileJson")
                   VALUES
                       ('same', 'same', 'app', TRUE, 'actor', 'pat', 'oauth', '{"id":"profile"}'),
                       ('other', 'other', 'app', TRUE, 'previous', NULL, 'oauth', '{"id":"profile"}'),
                       ('legacy', 'legacy', 'app', TRUE, NULL, 'pat', 'oauth', '{"id":"profile"}'),
                       ('paused', 'paused', 'app', FALSE, 'previous', 'pat', 'oauth', '{"id":"profile"}'),
                       ('foreign', 'foreign', 'other-app', TRUE, 'previous', 'pat', 'oauth', '{"id":"profile"}'),
                       ('unlisted', 'unlisted', 'app', TRUE, 'previous', 'pat', 'oauth', '{"id":"profile"}')"#,
            )
            .await
            .unwrap();
            let listed: Vec<String> = ["same", "other", "legacy", "paused", "foreign"]
                .map(str::to_owned)
                .into();

            assert_eq!(bind_run_as(&txn, "app", &listed, "actor").await.unwrap(), 3);
            assert_eq!(bind_run_as(&txn, "app", &[], "actor").await.unwrap(), 0);

            let row = |id: &str, run_as: &str, pat: bool, oauth: bool, profile: bool| SinkRow {
                id: id.to_owned(),
                run_as: run_as.to_owned(),
                pat,
                oauth,
                profile,
            };
            let expected = [
                row("foreign", "previous", true, true, true),
                row("legacy", "actor", false, false, false),
                row("other", "actor", false, false, false),
                row("paused", "previous", true, true, true),
                row("same", "actor", false, true, true),
                row("unlisted", "previous", true, true, true),
            ];
            assert_eq!(sink_rows(&txn).await, expected);

            assert_eq!(bind_run_as(&txn, "app", &listed, "actor").await.unwrap(), 3);
            assert_eq!(sink_rows(&txn).await, expected);
            txn.rollback().await.unwrap();
        }

        /// PATs whose secret is `{id}-secret`: two current ones that may run events, one
        /// expired and one read-only.
        async fn seed_pats(txn: &DatabaseTransaction) {
            use crate::permission::pat_permission::PatPermission::{Admin, ReadOnly, ReadWrite};
            let now = chrono::Utc::now().fixed_offset();
            let hour = chrono::Duration::hours(1);
            for (id, user, permission, valid_until) in [
                ("pat-executor", "executor", ReadWrite, None),
                ("pat-editor", "editor", Admin, Some(now + hour)),
                ("pat-expired", "executor", ReadWrite, Some(now - hour)),
                ("pat-read-only", "executor", ReadOnly, None),
            ] {
                let secret_hash = blake3::hash(format!("{id}-secret").as_bytes()).to_hex();
                txn.execute_raw(Statement::from_sql_and_values(
                    DbBackend::Postgres,
                    r#"INSERT INTO public."PAT" (id, key, permissions, "userId", "validUntil")
                       VALUES ($1, $2, $3, $4, $5)"#,
                    [
                        id.into(),
                        secret_hash.to_string().into(),
                        permission.bits().into(),
                        user.into(),
                        valid_until.into(),
                    ],
                ))
                .await
                .unwrap();
            }
        }

        fn stored_pat(pat_id: &str, secret: &str) -> String {
            crate::routes::app::events::db::encrypt_token(&format!("pat_{pat_id}.{secret}"), &KEY)
        }

        fn current_pat(pat_id: &str) -> String {
            stored_pat(pat_id, &format!("{pat_id}-secret"))
        }

        async fn legacy_sink(
            txn: &DatabaseTransaction,
            event: &str,
            app: &str,
            run_as: Option<&str>,
            pat_encrypted: String,
        ) {
            txn.execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                r#"INSERT INTO public."EventSink" (id, "eventId", "appId", active, "runAsUserId", "patEncrypted")
                   VALUES ($1, $1, $2, TRUE, $3, $4)"#,
                [
                    event.into(),
                    app.into(),
                    run_as.map(str::to_owned).into(),
                    pat_encrypted.into(),
                ],
            ))
            .await
            .unwrap();
        }

        #[derive(Debug, PartialEq)]
        struct SinkRow {
            id: String,
            run_as: String,
            pat: bool,
            oauth: bool,
            profile: bool,
        }

        async fn sink_rows(txn: &DatabaseTransaction) -> Vec<SinkRow> {
            txn.query_all_raw(Statement::from_string(
                DbBackend::Postgres,
                r#"SELECT id, "runAsUserId", "patEncrypted" IS NOT NULL AS pat,
                          "oauthTokensEncrypted" IS NOT NULL AS oauth, "profileJson" IS NOT NULL AS profile
                   FROM public."EventSink" ORDER BY id"#,
            ))
            .await
            .unwrap()
            .iter()
            .map(|row| {
                let flag = |name: &str| row.try_get::<bool>("", name).unwrap();
                SinkRow {
                    id: row.try_get("", "id").unwrap(),
                    run_as: row.try_get("", "runAsUserId").unwrap(),
                    pat: flag("pat"),
                    oauth: flag("oauth"),
                    profile: flag("profile"),
                }
            })
            .collect()
        }
    }
}
