//! What a bot turn may do in one app after every layer narrowed it. Pure bit and list math on
//! `RolePermissions` bits, shared by the API, the core runtime and the desktop host.

use crate::bot_restrictions::{BOT_NEVER_BITS, BotPermission, BotScopes};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Every defined `RolePermissions` bit.
pub const ALL_ROLE_BITS: i64 = (1 << 28) - 1;

const SUPERBITS: i64 = 0b11;

/// What a room invitation lets the bot do there, on top of the bot's own restrictions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BotRoomLimits {
    /// Most permissions the bot holds in the room; `None` adds no limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<i64>,
    /// Apps the bot may use in the room; `None` adds no limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apps: Option<Vec<String>>,
    /// Further scopes per app.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub scopes: BTreeMap<String, BotScopes>,
    /// The bot answers only when a message names it or replies to it; it doesn't read along.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mentioned_only: bool,
}

impl BotRoomLimits {
    /// Whether a bot with these limits (`None`: none set) may wake on a message that doesn't address it.
    pub fn reads_along(limits: Option<&Self>) -> bool {
        !limits.is_some_and(|limits| limits.mentioned_only)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedBotPermissions {
    pub role_bits: i64,
    pub scopes: BotScopes,
}

/// A mask holding `Owner` or `Admin` means every bit, so intersecting it cannot keep a bypass
/// that the other layers did not grant.
pub fn expand_role_bits(bits: i64) -> i64 {
    if bits & SUPERBITS != 0 {
        ALL_ROLE_BITS
    } else {
        bits & ALL_ROLE_BITS
    }
}

fn normalized(prefix: &str) -> &str {
    prefix.trim_matches('/')
}

fn is_under(path: &str, ancestor: &str) -> bool {
    ancestor.is_empty()
        || path == ancestor
        || path
            .strip_prefix(ancestor)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn intersect_prefixes(a: &[String], b: &[String]) -> Vec<String> {
    let mut kept = Vec::new();
    for left in a.iter().map(|p| normalized(p)) {
        for right in b.iter().map(|p| normalized(p)) {
            let narrower = if is_under(left, right) {
                left
            } else if is_under(right, left) {
                right
            } else {
                continue;
            };
            if !kept.iter().any(|k: &String| k == narrower) {
                kept.push(narrower.to_string());
            }
        }
    }
    kept
}

fn intersect_lists(a: &Option<Vec<String>>, b: &Option<Vec<String>>) -> Option<Vec<String>> {
    match (a, b) {
        (None, other) | (other, None) => other.clone(),
        (Some(a), Some(b)) => Some(a.iter().filter(|e| b.contains(e)).cloned().collect()),
    }
}

/// Both scopes at once; the second value is `true` when the folders share no path, which the
/// folder list cannot express (empty means the whole app).
pub fn intersect_scopes(a: &BotScopes, b: &BotScopes) -> (BotScopes, bool) {
    let (prefixes, disjoint) = match (a.files.prefixes.is_empty(), b.files.prefixes.is_empty()) {
        (true, _) => (b.files.prefixes.clone(), false),
        (_, true) => (a.files.prefixes.clone(), false),
        _ => {
            let kept = intersect_prefixes(&a.files.prefixes, &b.files.prefixes);
            let disjoint = kept.is_empty();
            (kept, disjoint)
        }
    };
    let scopes = BotScopes {
        files: crate::bot_restrictions::BotFileScope {
            prefixes,
            read_only: a.files.read_only || b.files.read_only,
        },
        tables: intersect_lists(&a.tables, &b.tables),
        events: intersect_lists(&a.events, &b.events),
        boards: intersect_lists(&a.boards, &b.boards),
    };
    (scopes, disjoint)
}

fn bits_of(permissions: &[BotPermission]) -> i64 {
    crate::bot_restrictions::permission_bits(permissions)
}

/// `owner_bits` (live) → superbits expanded → minus [`BOT_NEVER_BITS`] → `mask` of the bot's
/// app row (`None` = whole role) → the room's limits → resource scopes. Never wider than the
/// owner; `Owner` and `Admin` never survive.
pub fn resolve_bot_permissions(
    owner_bits: i64,
    app_id: &str,
    mask: Option<i64>,
    scopes: &BotScopes,
    room: Option<&BotRoomLimits>,
) -> ResolvedBotPermissions {
    let mut bits = expand_role_bits(owner_bits) & !BOT_NEVER_BITS;
    if let Some(mask) = mask {
        bits &= expand_role_bits(mask);
    }
    let mut scopes = scopes.clone();
    let mut files_disjoint = false;

    if let Some(room) = room {
        if room
            .apps
            .as_ref()
            .is_some_and(|apps| !apps.iter().any(|a| a == app_id))
        {
            return ResolvedBotPermissions::default();
        }
        if let Some(room_mask) = room.mask {
            bits &= expand_role_bits(room_mask);
        }
        if let Some(room_scopes) = room.scopes.get(app_id) {
            (scopes, files_disjoint) = intersect_scopes(&scopes, room_scopes);
        }
    }

    use BotPermission::*;
    if files_disjoint {
        bits &= !bits_of(&[ReadFiles, WriteFiles]);
    }
    if scopes.files.read_only {
        bits &= !bits_of(&[WriteFiles]);
    }
    let emptied: [(bool, &[BotPermission]); 3] = [
        (
            scopes.tables.as_ref().is_some_and(Vec::is_empty),
            &[ReadDatabase, WriteDatabase],
        ),
        (
            scopes.events.as_ref().is_some_and(Vec::is_empty),
            &[ListEvents, ReadEvents, ExecuteEvents, WriteEvents],
        ),
        (
            scopes.boards.as_ref().is_some_and(Vec::is_empty),
            &[ReadBoards, ExecuteBoards, WriteBoards],
        ),
    ];
    for (is_empty, family) in emptied {
        if is_empty {
            bits &= !bits_of(family);
        }
    }

    ResolvedBotPermissions {
        role_bits: bits & !BOT_NEVER_BITS,
        scopes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot_restrictions::BotFileScope;
    use BotPermission::*;

    const OWNER: i64 = 1;
    const ADMIN: i64 = 2;

    fn bits(permissions: &[BotPermission]) -> i64 {
        bits_of(permissions)
    }

    fn resolve(owner: i64, mask: Option<i64>, room: Option<&BotRoomLimits>) -> i64 {
        resolve_bot_permissions(owner, "app", mask, &BotScopes::default(), room).role_bits
    }

    #[test]
    fn owner_roles_by_masks_table() {
        let editor = bits(&[ReadFiles, WriteFiles, ReadBoards, ExecuteEvents]);
        let cases: [(&str, i64, Option<i64>, i64); 8] = [
            ("editor, no mask", editor, None, editor),
            (
                "editor, read mask",
                editor,
                Some(bits(&[ReadFiles])),
                bits(&[ReadFiles]),
            ),
            (
                "mask wider than role",
                bits(&[ReadFiles]),
                Some(editor),
                bits(&[ReadFiles]),
            ),
            (
                "disjoint mask",
                bits(&[ReadFiles]),
                Some(bits(&[WriteFiles])),
                0,
            ),
            ("empty mask", editor, Some(0), 0),
            (
                "admin, no mask is every grantable bit",
                ADMIN,
                None,
                ALL_ROLE_BITS & !BOT_NEVER_BITS,
            ),
            (
                "owner, read mask",
                OWNER,
                Some(bits(&[ReadFiles])),
                bits(&[ReadFiles]),
            ),
            ("no role bits", 0, None, 0),
        ];
        for (name, owner, mask, expected) in cases {
            assert_eq!(resolve(owner, mask, None), expected, "{name}");
        }
    }

    #[test]
    fn never_bits_survive_no_layer() {
        for owner in [OWNER, ADMIN, ALL_ROLE_BITS, BOT_NEVER_BITS] {
            for mask in [None, Some(ALL_ROLE_BITS), Some(ADMIN), Some(BOT_NEVER_BITS)] {
                assert_eq!(resolve(owner, mask, None) & BOT_NEVER_BITS, 0);
            }
        }
        let wide = BotRoomLimits {
            mask: Some(ALL_ROLE_BITS),
            ..Default::default()
        };
        assert_eq!(
            resolve(ALL_ROLE_BITS, None, Some(&wide)) & BOT_NEVER_BITS,
            0
        );
    }

    #[test]
    fn mask_with_superbit_cannot_widen_the_owner() {
        let owner = bits(&[ReadFiles]);
        assert_eq!(resolve(owner, Some(ADMIN), None), owner);
    }

    #[test]
    fn room_limits_only_narrow() {
        let owner = bits(&[ReadFiles, WriteFiles, ExecuteEvents]);
        let read = Some(bits(&[ReadFiles]));
        let cases: [(&str, BotRoomLimits, i64); 5] = [
            ("no limit", BotRoomLimits::default(), owner),
            (
                "room mask",
                BotRoomLimits {
                    mask: read,
                    ..Default::default()
                },
                bits(&[ReadFiles]),
            ),
            (
                "room mask wider than the bot",
                BotRoomLimits {
                    mask: Some(ALL_ROLE_BITS),
                    ..Default::default()
                },
                owner,
            ),
            (
                "app not in room list",
                BotRoomLimits {
                    apps: Some(vec!["other".into()]),
                    ..Default::default()
                },
                0,
            ),
            (
                "app in room list",
                BotRoomLimits {
                    apps: Some(vec!["app".into()]),
                    ..Default::default()
                },
                owner,
            ),
        ];
        for (name, room, expected) in cases {
            assert_eq!(resolve(owner, None, Some(&room)), expected, "{name}");
        }
        let both = BotRoomLimits {
            mask: read,
            ..Default::default()
        };
        assert_eq!(resolve(owner, Some(bits(&[WriteFiles])), Some(&both)), 0);
    }

    fn files(prefixes: &[&str], read_only: bool) -> BotScopes {
        BotScopes {
            files: BotFileScope {
                prefixes: prefixes.iter().map(|p| p.to_string()).collect(),
                read_only,
            },
            ..Default::default()
        }
    }

    #[test]
    fn scopes_intersect() {
        let (merged, disjoint) =
            intersect_scopes(&files(&["a/b", "c"], false), &files(&["a"], true));
        assert_eq!(merged.files.prefixes, vec!["a/b".to_string()]);
        assert!(merged.files.read_only && !disjoint);

        let (merged, disjoint) = intersect_scopes(&files(&[], false), &files(&["x/"], false));
        assert_eq!(merged.files.prefixes, vec!["x/".to_string()]);
        assert!(!disjoint);

        let (_, disjoint) = intersect_scopes(&files(&["ab"], false), &files(&["a"], false));
        assert!(
            disjoint,
            "a sibling with a shared string prefix is not under it"
        );

        let some = |v: &[&str]| Some(v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        let a = BotScopes {
            tables: some(&["t1", "t2"]),
            events: None,
            ..Default::default()
        };
        let b = BotScopes {
            tables: some(&["t2", "t3"]),
            events: some(&["e"]),
            ..Default::default()
        };
        let (merged, _) = intersect_scopes(&a, &b);
        assert_eq!(merged.tables, some(&["t2"]));
        assert_eq!(merged.events, some(&["e"]));
    }

    #[test]
    fn scopes_close_the_matching_permissions() {
        let owner = bits(&[
            ReadFiles,
            WriteFiles,
            ReadDatabase,
            ExecuteEvents,
            ReadBoards,
        ]);
        let scopes = BotScopes {
            tables: Some(vec![]),
            events: Some(vec![]),
            files: BotFileScope {
                prefixes: vec![],
                read_only: true,
            },
            ..Default::default()
        };
        let resolved = resolve_bot_permissions(owner, "app", None, &scopes, None);
        assert_eq!(resolved.role_bits, bits(&[ReadFiles, ReadBoards]));

        let room = BotRoomLimits {
            scopes: [("app".to_string(), files(&["b"], false))].into(),
            ..Default::default()
        };
        let disjoint =
            resolve_bot_permissions(owner, "app", None, &files(&["a"], false), Some(&room));
        assert_eq!(disjoint.role_bits & bits(&[ReadFiles, WriteFiles]), 0);
        assert_ne!(disjoint.role_bits & bits(&[ReadBoards]), 0);
    }

    #[test]
    fn room_limits_round_trip() {
        let limits: BotRoomLimits = serde_json::from_str("{}").unwrap();
        assert_eq!(limits, BotRoomLimits::default());
        assert!(BotRoomLimits::reads_along(Some(&limits)) && BotRoomLimits::reads_along(None));
        assert_eq!(serde_json::to_string(&limits).unwrap(), "{}");
    }

    #[test]
    fn mentioned_only_round_trips_and_stops_reading_along() {
        let limits: BotRoomLimits = serde_json::from_str(r#"{"mentioned_only":true}"#).unwrap();
        assert!(limits.mentioned_only && !BotRoomLimits::reads_along(Some(&limits)));
        assert_eq!(
            serde_json::to_string(&limits).unwrap(),
            r#"{"mentioned_only":true}"#
        );
    }
}
