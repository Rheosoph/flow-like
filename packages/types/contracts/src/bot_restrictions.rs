//! What an owner lets a bot do in one attached app. The bot never holds more than the owner's
//! role there; these restrictions narrow it further, in every chat. Permissions are `RolePermissions`
//! bits, named here because this crate can't depend on the API.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const MAX_SCOPE_ENTRIES: usize = 200;
pub const MAX_SCOPE_ENTRY_LEN: usize = 512;

/// The refusal a bot gets in an app whose owner set it to refuse bots, on every host.
pub const APP_REFUSES_BOTS: &str = "APP_REFUSES_BOTS";

/// [`APP_REFUSES_BOTS`] as a sentence the bot can pass on to its owner.
pub fn app_refuses_bots_message(app_name: &str) -> String {
    format!("The app '{app_name}' does not allow bots; ask its owner.")
}

const OWNER: i64 = 1 << 0;
const ADMIN: i64 = 1 << 1;
const READ_ROLES: i64 = 1 << 3;
const WRITE_META: i64 = 1 << 7;
const READ_ANALYTICS: i64 = 1 << 16;
const WRITE_CONFIG: i64 = 1 << 18;
const WRITE_ROUTES: i64 = 1 << 25;

/// Bits a bot never holds, whatever the owner's role: `Owner`, `Admin`, `ReadRoles`,
/// `WriteMeta`, `ReadAnalytics`, `WriteConfig` and `WriteRoutes`.
pub const BOT_NEVER_BITS: i64 =
    OWNER | ADMIN | READ_ROLES | WRITE_META | READ_ANALYTICS | WRITE_CONFIG | WRITE_ROUTES;

/// A permission an owner may leave a bot. Each maps to the `RolePermissions` flag of the same name.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum BotPermission {
    ReadFiles,
    WriteFiles,
    ReadDatabase,
    WriteDatabase,
    InvokeApi,
    ReadTeam,
    ReadBoards,
    ExecuteBoards,
    WriteBoards,
    ListEvents,
    ReadEvents,
    ExecuteEvents,
    WriteEvents,
    ReadLogs,
    ReadConfig,
    ReadTemplates,
    WriteTemplates,
    ReadCourses,
    WriteCourses,
    ReadWidgets,
    WriteWidgets,
}

impl BotPermission {
    pub const ALL: [BotPermission; 21] = [
        Self::ReadFiles,
        Self::WriteFiles,
        Self::ReadDatabase,
        Self::WriteDatabase,
        Self::InvokeApi,
        Self::ReadTeam,
        Self::ReadBoards,
        Self::ExecuteBoards,
        Self::WriteBoards,
        Self::ListEvents,
        Self::ReadEvents,
        Self::ExecuteEvents,
        Self::WriteEvents,
        Self::ReadLogs,
        Self::ReadConfig,
        Self::ReadTemplates,
        Self::WriteTemplates,
        Self::ReadCourses,
        Self::WriteCourses,
        Self::ReadWidgets,
        Self::WriteWidgets,
    ];

    /// The `RolePermissions` bit.
    pub const fn bit(self) -> i64 {
        match self {
            Self::ReadTeam => 1 << 2,
            Self::ReadFiles => 1 << 4,
            Self::WriteFiles => 1 << 5,
            Self::InvokeApi => 1 << 6,
            Self::ReadBoards => 1 << 8,
            Self::ExecuteBoards => 1 << 9,
            Self::WriteBoards => 1 << 10,
            Self::ListEvents => 1 << 11,
            Self::ReadEvents => 1 << 12,
            Self::ExecuteEvents => 1 << 13,
            Self::WriteEvents => 1 << 14,
            Self::ReadLogs => 1 << 15,
            Self::ReadConfig => 1 << 17,
            Self::ReadTemplates => 1 << 19,
            Self::WriteTemplates => 1 << 20,
            Self::ReadCourses => 1 << 21,
            Self::WriteCourses => 1 << 22,
            Self::ReadWidgets => 1 << 23,
            Self::WriteWidgets => 1 << 24,
            Self::ReadDatabase => 1 << 26,
            Self::WriteDatabase => 1 << 27,
        }
    }
}

/// The bits of these permissions; never-bits can't appear.
pub fn permission_bits(permissions: &[BotPermission]) -> i64 {
    permissions.iter().fold(0, |bits, p| bits | p.bit())
}

/// The permissions in these bits, in declaration order; unknown and never-bits are ignored.
pub fn permissions_from_bits(bits: i64) -> Vec<BotPermission> {
    BotPermission::ALL
        .into_iter()
        .filter(|p| bits & p.bit() != 0)
        .collect()
}

/// Folder prefixes inside the app's files.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BotFileScope {
    /// Folders (no leading slash, e.g. `reports/2026`) the bot may reach; empty means the whole app.
    #[serde(default)]
    pub prefixes: Vec<String>,
    /// The bot reads but never writes, whatever its permissions say.
    #[serde(default)]
    pub read_only: bool,
}

/// Which parts of the app the bot may reach. `None` doesn't narrow; `Some([])` allows nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BotScopes {
    #[serde(default)]
    pub files: BotFileScope,
    /// Table names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tables: Option<Vec<String>>,
    /// Event ids.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<Vec<String>>,
    /// Board ids.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boards: Option<Vec<String>>,
}

/// One bot's restrictions in one app.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BotRestrictions {
    /// `None` keeps the owner's full role (minus [`BOT_NEVER_BITS`]); otherwise the most the bot may hold.
    #[serde(default)]
    pub permissions: Option<Vec<BotPermission>>,
    #[serde(default)]
    pub scopes: BotScopes,
}

/// Ready-made restrictions.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum BotPreset {
    /// Looks at files, tables, boards and events; changes nothing.
    ReadOnly,
    /// Everything of read only, and runs the app's events.
    ReadAndCallEvents,
    /// The owner's whole role.
    Full,
}

impl BotPreset {
    pub const ALL: [BotPreset; 3] = [Self::ReadOnly, Self::ReadAndCallEvents, Self::Full];

    pub fn restrictions(self) -> BotRestrictions {
        let read_only = [
            BotPermission::ReadFiles,
            BotPermission::ReadDatabase,
            BotPermission::ReadBoards,
            BotPermission::ListEvents,
            BotPermission::ReadEvents,
        ];
        match self {
            Self::Full => BotRestrictions::default(),
            Self::ReadOnly => BotRestrictions {
                permissions: Some(read_only.to_vec()),
                scopes: BotScopes::default(),
            },
            Self::ReadAndCallEvents => BotRestrictions {
                permissions: Some(
                    read_only
                        .into_iter()
                        .chain([BotPermission::ExecuteEvents])
                        .collect(),
                ),
                scopes: BotScopes::default(),
            },
        }
    }
}

fn check_list(label: &str, entries: &[String]) -> Result<(), String> {
    if entries.len() > MAX_SCOPE_ENTRIES {
        return Err(format!(
            "{label} lists {} entries; at most {MAX_SCOPE_ENTRIES} are allowed",
            entries.len()
        ));
    }
    for entry in entries {
        if entry.trim().is_empty() || entry.chars().count() > MAX_SCOPE_ENTRY_LEN {
            return Err(format!(
                "{label} entry '{}' must be 1 to {MAX_SCOPE_ENTRY_LEN} characters",
                entry.chars().take(80).collect::<String>()
            ));
        }
    }
    Ok(())
}

impl BotRestrictions {
    pub fn unrestricted() -> Self {
        Self::default()
    }

    /// The mask as stored: `None` when the permissions are not narrowed.
    pub fn permission_bits(&self) -> Option<i64> {
        self.permissions.as_deref().map(permission_bits)
    }

    pub fn from_bits(bits: Option<i64>, scopes: BotScopes) -> Self {
        Self {
            permissions: bits.map(permissions_from_bits),
            scopes,
        }
    }

    pub fn is_unrestricted(&self) -> bool {
        self.permissions.is_none() && self.scopes == BotScopes::default()
    }

    /// The preset these restrictions equal, if any.
    pub fn preset(&self) -> Option<BotPreset> {
        let mut own = self.clone();
        if let Some(permissions) = own.permissions.as_mut() {
            permissions.sort();
            permissions.dedup();
        }
        BotPreset::ALL.into_iter().find(|preset| {
            let mut candidate = preset.restrictions();
            if let Some(permissions) = candidate.permissions.as_mut() {
                permissions.sort();
            }
            candidate == own
        })
    }

    /// Rejects folders that climb out of the app and oversized or blank lists.
    pub fn validate(&self) -> Result<(), String> {
        let files = &self.scopes.files;
        check_list("Folders", &files.prefixes)?;
        if let Some(bad) = files
            .prefixes
            .iter()
            .find(|p| p.split('/').any(|part| part == ".."))
        {
            return Err(format!("Folder '{bad}' must stay inside the app"));
        }
        for (label, list) in [
            ("Tables", &self.scopes.tables),
            ("Events", &self.scopes.events),
            ("Boards", &self.scopes.boards),
        ] {
            if let Some(list) = list {
                check_list(label, list)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_round_trip_and_never_bits_are_unreachable() {
        let all = permission_bits(&BotPermission::ALL);
        assert_eq!(all & BOT_NEVER_BITS, 0);
        assert_eq!(permissions_from_bits(all), BotPermission::ALL.to_vec());
        assert_eq!(permissions_from_bits(BOT_NEVER_BITS | 1 << 40), vec![]);
        assert_eq!(
            permissions_from_bits(BotPermission::ReadFiles.bit() | ADMIN),
            vec![BotPermission::ReadFiles]
        );
    }

    #[test]
    fn restrictions_round_trip_through_json() {
        let restrictions = BotRestrictions {
            permissions: Some(vec![BotPermission::ReadFiles, BotPermission::ExecuteEvents]),
            scopes: BotScopes {
                files: BotFileScope {
                    prefixes: vec!["reports".into()],
                    read_only: true,
                },
                tables: Some(vec!["sales".into()]),
                events: Some(vec![]),
                boards: None,
            },
        };
        let json = serde_json::to_value(&restrictions).unwrap();
        assert_eq!(json["permissions"][0], "read_files");
        assert!(json["scopes"].get("boards").is_none());
        let back: BotRestrictions = serde_json::from_value(json).unwrap();
        assert_eq!(back, restrictions);
        let empty: BotRestrictions = serde_json::from_str("{}").unwrap();
        assert!(empty.is_unrestricted());
        let scopes: BotScopes = serde_json::from_str("{}").unwrap();
        assert_eq!(scopes, BotScopes::default());
    }

    #[test]
    fn presets_are_detected_and_stay_inside_the_grantable_bits() {
        for preset in BotPreset::ALL {
            let restrictions = preset.restrictions();
            assert_eq!(restrictions.preset(), Some(preset));
            assert_eq!(
                restrictions.permission_bits().unwrap_or(0) & BOT_NEVER_BITS,
                0
            );
        }
        assert_eq!(BotPreset::Full.restrictions().permission_bits(), None);
        let read_only = BotPreset::ReadOnly
            .restrictions()
            .permission_bits()
            .unwrap();
        let events = BotPreset::ReadAndCallEvents
            .restrictions()
            .permission_bits()
            .unwrap();
        assert_eq!(events & read_only, read_only);
        assert_eq!(events & !read_only, BotPermission::ExecuteEvents.bit());
        let mut custom = BotPreset::ReadOnly.restrictions();
        custom.scopes.files.read_only = true;
        assert_eq!(custom.preset(), None);
    }

    #[test]
    fn validation_rejects_escapes_and_blank_entries() {
        let mut restrictions = BotRestrictions::default();
        assert!(restrictions.validate().is_ok());
        restrictions.scopes.files.prefixes = vec!["a/../b".into()];
        assert!(restrictions.validate().is_err());
        restrictions.scopes.files.prefixes = vec!["a..b/c".into()];
        assert!(restrictions.validate().is_ok());
        restrictions.scopes.tables = Some(vec!["  ".into()]);
        assert!(restrictions.validate().is_err());
        restrictions.scopes.tables = Some(vec!["t".into(); MAX_SCOPE_ENTRIES + 1]);
        assert!(restrictions.validate().is_err());
    }
}
