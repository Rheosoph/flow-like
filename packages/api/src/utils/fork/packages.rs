use super::{SkippedItem, SkippedKind};
use crate::{
    entity::{
        app_package,
        sea_orm_active_enums::{WasmPackageStatus, WasmPackageVisibility},
        wasm_package, wasm_package_join_queue, wasm_package_user, wasm_package_version,
    },
    error::ApiError,
    package_license,
    permission::wasm_package_permission::WasmPackagePermission,
};
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, EntityTrait, FromQueryResult, QueryFilter, QueryOrder,
    QuerySelect, Select,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use utoipa::ToSchema;

/// Why the forker can't take a package the source app pins into their copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PackageBlock {
    /// Public and paid: buying it keeps it in the fork.
    Paid,
    /// Public on request: the author has to approve access first.
    RequestAccess,
    /// Private, and the forker is not a member.
    Private,
    /// No longer in the registry.
    Missing,
    /// Not active in the registry (in review, rejected, deprecated or
    /// disabled), so it can be neither bought nor downloaded.
    Unavailable,
    /// The forker has an access row without permissions. Buying or
    /// requesting access can't restore it; only the package's owner can.
    Revoked,
}

/// A package the fork drops: the forker doesn't hold it, or holds it without
/// a version they can download.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct BlockedPackage {
    pub package_id: String,
    /// Registry display name; the id when the package is private or missing,
    /// so a forker never learns the name of a private package.
    pub name: String,
    pub block: PackageBlock,
    /// Price in cents. Zero unless `block` is `paid`.
    pub price: i64,
    /// The forker already asked the author for access.
    pub request_pending: bool,
}

impl BlockedPackage {
    pub fn skipped_item(&self) -> SkippedItem {
        let name = &self.name;
        let reason = match self.block {
            PackageBlock::Paid => format!("{name} is a paid package you don't own"),
            PackageBlock::RequestAccess => {
                format!("{name} needs its author's approval before you can use it")
            }
            PackageBlock::Private => format!("{name} is private and you don't have access"),
            PackageBlock::Missing => format!("{name} no longer exists in the registry"),
            PackageBlock::Unavailable => format!("{name} isn't available in the registry"),
            PackageBlock::Revoked => format!("your access to {name} was revoked"),
        };
        SkippedItem {
            kind: SkippedKind::Package,
            source_id: self.package_id.clone(),
            reason,
        }
    }
}

/// A held package the fork pins at another version, because the forker can't
/// download the one the source app pins.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct RepinnedPackage {
    pub package_id: String,
    pub name: String,
    /// The version the source app pins.
    pub pinned_version: String,
    /// The newest version the forker can download, which the fork pins.
    pub version: String,
}

impl RepinnedPackage {
    pub fn warning(&self) -> String {
        let Self {
            name,
            pinned_version,
            version,
            ..
        } = self;
        format!(
            "{name} {pinned_version} isn't published, so your copy uses {version}. Flows built for {pinned_version} may need their nodes updated."
        )
    }
}

/// What a fork does with the source app's pins.
#[derive(Debug, Default)]
pub struct PinSplit {
    /// The pins the fork carries, each at a version the forker can download.
    pub held: Vec<app_package::Model>,
    pub blocked: Vec<BlockedPackage>,
    pub repinned: Vec<RepinnedPackage>,
}

/// Splits the source app's pins into the ones the fork carries and the ones
/// it drops. Holding follows the project licence rule
/// ([`package_license::user_holds`]): free public packages are held by
/// everyone, everything else needs a non-zero `WasmPackageUser` row. A
/// package that isn't active is held only through such a row, because the
/// registry serves it to nobody else. An anonymous forker (`None`) holds only
/// active free public packages.
///
/// A held pin keeps its version only when the forker can download it. A
/// version that was never approved is served to the package's managers alone,
/// so everyone else gets the newest version they can download, and the
/// package is dropped when there is none.
pub async fn split_pins<C: ConnectionTrait>(
    db: &C,
    user_sub: Option<&str>,
    pins: &[app_package::Model],
) -> Result<PinSplit, ApiError> {
    let pins: BTreeMap<&str, &app_package::Model> = pins
        .iter()
        .map(|pin| (pin.package_id.as_str(), pin))
        .collect();
    if pins.is_empty() {
        return Ok(PinSplit::default());
    }
    let package_ids: Vec<String> = pins.keys().map(|id| id.to_string()).collect();

    let (packages, access, pending, pinned) = flow_like_types::tokio::try_join!(
        registry_rows(db, &package_ids),
        access_permissions(db, user_sub, &package_ids),
        pending_request_ids(db, user_sub, &package_ids),
        pinned_version_statuses(db, pins.values().copied()),
    )?;

    let mut split = PinSplit::default();
    let mut stuck = Vec::new();
    for (package_id, pin) in pins {
        let package = packages.get(package_id);
        let permission = access.get(package_id).copied();
        let pending = pending.contains(package_id);
        if let Some(blocked) = classify(package_id.to_string(), package, permission, pending) {
            split.blocked.push(blocked);
            continue;
        }
        let Some(package) = package else { continue };
        let sees_all = sees_all_versions(package, permission);
        let status = pinned.get(&(pin.package_id.clone(), pin.version.clone()));
        if can_download(sees_all, status) {
            split.held.push(pin.clone());
        } else {
            stuck.push((pin, package, sees_all));
        }
    }
    repin(db, &stuck, &mut split).await?;
    Ok(split)
}

/// The manifest's `package_id -> version` map for a set of pins.
pub fn pin_map(pins: &[app_package::Model]) -> HashMap<String, String> {
    pins.iter()
        .map(|pin| (pin.package_id.clone(), pin.version.clone()))
        .collect()
}

/// The source pins a copy made earlier does not have.
pub fn missing_pins(
    source: &[app_package::Model],
    copy: &[app_package::Model],
) -> Vec<app_package::Model> {
    let pinned: HashSet<&str> = copy.iter().map(|pin| pin.package_id.as_str()).collect();
    source
        .iter()
        .filter(|pin| !pinned.contains(pin.package_id.as_str()))
        .cloned()
        .collect()
}

/// Moves each held pin the forker can't download to the newest version they
/// can, or drops the package when no version qualifies.
async fn repin<C: ConnectionTrait>(
    db: &C,
    stuck: &[(&app_package::Model, &wasm_package::Model, bool)],
    split: &mut PinSplit,
) -> Result<(), ApiError> {
    if stuck.is_empty() {
        return Ok(());
    }
    let package_ids = stuck.iter().map(|(pin, ..)| pin.package_id.clone());
    let newest = newest_versions(db, package_ids).await?;
    for (pin, package, sees_all) in stuck {
        let versions = newest.get(&pin.package_id).map(Vec::as_slice);
        match fallback_version(*sees_all, versions.unwrap_or_default()) {
            Some(version) => {
                split.repinned.push(RepinnedPackage {
                    package_id: pin.package_id.clone(),
                    name: package.name.clone(),
                    pinned_version: pin.version.clone(),
                    version: version.to_string(),
                });
                split.held.push(app_package::Model {
                    version: version.to_string(),
                    ..(*pin).clone()
                });
            }
            None => split.blocked.push(BlockedPackage {
                package_id: pin.package_id.clone(),
                name: package.name.clone(),
                block: PackageBlock::Unavailable,
                price: 0,
                request_pending: false,
            }),
        }
    }
    Ok(())
}

/// Whether the registry serves the forker every version of a package and not
/// only the approved ones: its owner and maintainers get them, and so does
/// anyone holding a private package.
fn sees_all_versions(package: &wasm_package::Model, permission: Option<i64>) -> bool {
    package.visibility == WasmPackageVisibility::Private
        || permission.is_some_and(|bits| {
            WasmPackagePermission::from_bits_truncate(bits)
                .has_permission(WasmPackagePermission::Maintainer)
        })
}

/// Whether the forker can download the version a pin names. A version without
/// a row can't be downloaded by anyone.
fn can_download(sees_all: bool, pinned: Option<&WasmPackageStatus>) -> bool {
    match pinned {
        Some(WasmPackageStatus::Active) => true,
        Some(_) => sees_all,
        None => false,
    }
}

/// The newest version the forker can download, from rows ordered newest first.
fn fallback_version(sees_all: bool, newest_first: &[VersionStatus]) -> Option<&str> {
    newest_first
        .iter()
        .find(|row| sees_all || row.status == WasmPackageStatus::Active)
        .map(|row| row.version.as_str())
}

/// `permission` is the forker's `WasmPackageUser` permission bits, `None`
/// without a row.
fn classify(
    package_id: String,
    package: Option<&wasm_package::Model>,
    permission: Option<i64>,
    pending: bool,
) -> Option<BlockedPackage> {
    let Some(package) = package else {
        return Some(BlockedPackage {
            name: package_id.clone(),
            package_id,
            block: PackageBlock::Missing,
            price: 0,
            request_pending: false,
        });
    };
    let block = block_for(package, permission)?;
    let name = if package.visibility == WasmPackageVisibility::Private {
        package_id.clone()
    } else {
        package.name.clone()
    };
    Some(BlockedPackage {
        name,
        price: if block == PackageBlock::Paid {
            package.price
        } else {
            0
        },
        request_pending: pending && block == PackageBlock::RequestAccess,
        package_id,
        block,
    })
}

/// Why a registry package is out of the forker's reach, `None` when they
/// hold it.
fn block_for(package: &wasm_package::Model, permission: Option<i64>) -> Option<PackageBlock> {
    if permission.is_some_and(|bits| bits != 0) {
        return None;
    }
    if package.status != WasmPackageStatus::Active {
        return Some(PackageBlock::Unavailable);
    }
    if !package_license::requires_license(package) {
        return None;
    }
    if permission.is_some() {
        return Some(PackageBlock::Revoked);
    }
    Some(match package.visibility {
        WasmPackageVisibility::Public => PackageBlock::Paid,
        WasmPackageVisibility::PublicRequestAccess => PackageBlock::RequestAccess,
        WasmPackageVisibility::Private => PackageBlock::Private,
    })
}

async fn registry_rows<C: ConnectionTrait>(
    db: &C,
    package_ids: &[String],
) -> Result<HashMap<String, wasm_package::Model>, ApiError> {
    Ok(wasm_package::Entity::find()
        .filter(wasm_package::Column::Id.is_in(package_ids.iter().cloned()))
        .all(db)
        .await?
        .into_iter()
        .map(|package| (package.id.clone(), package))
        .collect())
}

/// The forker's permission bits per package, including rows without
/// permissions.
async fn access_permissions<C: ConnectionTrait>(
    db: &C,
    user_sub: Option<&str>,
    package_ids: &[String],
) -> Result<HashMap<String, i64>, ApiError> {
    let Some(user_sub) = user_sub else {
        return Ok(HashMap::new());
    };
    let rows: Vec<(String, i64)> = wasm_package_user::Entity::find()
        .select_only()
        .column(wasm_package_user::Column::PackageId)
        .column(wasm_package_user::Column::Permission)
        .filter(wasm_package_user::Column::PackageId.is_in(package_ids.iter().cloned()))
        .filter(wasm_package_user::Column::UserId.eq(user_sub))
        .into_tuple()
        .all(db)
        .await?;
    let mut access = HashMap::with_capacity(rows.len());
    for (package_id, permission) in rows {
        *access.entry(package_id).or_insert(0) |= permission;
    }
    Ok(access)
}

async fn pending_request_ids<C: ConnectionTrait>(
    db: &C,
    user_sub: Option<&str>,
    package_ids: &[String],
) -> Result<HashSet<String>, ApiError> {
    let Some(user_sub) = user_sub else {
        return Ok(HashSet::new());
    };
    let pending: Vec<String> = wasm_package_join_queue::Entity::find()
        .select_only()
        .column(wasm_package_join_queue::Column::PackageId)
        .filter(wasm_package_join_queue::Column::PackageId.is_in(package_ids.iter().cloned()))
        .filter(wasm_package_join_queue::Column::UserId.eq(user_sub))
        .into_tuple()
        .all(db)
        .await?;
    Ok(pending.into_iter().collect())
}

/// A version row without its node and widget blobs.
#[derive(Debug, FromQueryResult)]
struct VersionStatus {
    package_id: String,
    version: String,
    status: WasmPackageStatus,
}

fn version_statuses() -> Select<wasm_package_version::Entity> {
    use wasm_package_version::Column;
    wasm_package_version::Entity::find()
        .select_only()
        .column_as(Column::PackageId, "package_id")
        .column_as(Column::Version, "version")
        .column_as(Column::Status, "status")
}

/// The status of exactly the versions the pins name. A pin whose version has
/// no row is absent.
async fn pinned_version_statuses<'a, C: ConnectionTrait>(
    db: &C,
    pins: impl Iterator<Item = &'a app_package::Model>,
) -> Result<HashMap<(String, String), WasmPackageStatus>, ApiError> {
    use wasm_package_version::Column;
    let named = pins.fold(Condition::any(), |named, pin| {
        named.add(
            Condition::all()
                .add(Column::PackageId.eq(&pin.package_id))
                .add(Column::Version.eq(&pin.version)),
        )
    });
    let rows = version_statuses()
        .filter(named)
        .into_model::<VersionStatus>()
        .all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| ((row.package_id, row.version), row.status))
        .collect())
}

/// Every version of those packages that was not yanked, newest first.
async fn newest_versions<C: ConnectionTrait>(
    db: &C,
    package_ids: impl Iterator<Item = String>,
) -> Result<HashMap<String, Vec<VersionStatus>>, ApiError> {
    use wasm_package_version::Column;
    let rows = version_statuses()
        .filter(Column::PackageId.is_in(package_ids))
        .filter(Column::Yanked.eq(false))
        .order_by_desc(Column::PublishedAt)
        .into_model::<VersionStatus>()
        .all(db)
        .await?;
    let mut newest: HashMap<String, Vec<VersionStatus>> = HashMap::new();
    for row in rows {
        newest.entry(row.package_id.clone()).or_default().push(row);
    }
    Ok(newest)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUYER: i64 = 0b1000;

    fn package(
        visibility: WasmPackageVisibility,
        price: i64,
        status: WasmPackageStatus,
    ) -> wasm_package::Model {
        let now = chrono::Utc::now().fixed_offset();
        wasm_package::Model {
            id: "chart-kit".to_string(),
            name: "Chart Kit".to_string(),
            description: String::new(),
            version: "1.0.0".to_string(),
            license: None,
            homepage: None,
            repository: None,
            verified: false,
            download_count: 0,
            wasm_path: String::new(),
            wasm_hash: String::new(),
            wasm_size: 0,
            nodes: Default::default(),
            permissions: Default::default(),
            created_at: now,
            updated_at: now,
            published_at: None,
            price,
            readme: None,
            avg_rating: None,
            rating_count: 0,
            rating_sum: 0,
            widget_bundle_hash: None,
            widget_bundle_size: None,
            widgets: Default::default(),
            keywords: None,
            status,
            visibility,
            primary_category: None,
            secondary_category: None,
        }
    }

    fn block(
        package: &wasm_package::Model,
        permission: Option<i64>,
        pending: bool,
    ) -> Option<BlockedPackage> {
        classify(package.id.clone(), Some(package), permission, pending)
    }

    #[test]
    fn a_missing_package_is_reported_by_id() {
        let blocked = classify("gone".to_string(), None, Some(BUYER), false).unwrap();
        assert_eq!(blocked.block, PackageBlock::Missing);
        assert_eq!(blocked.name, "gone");
    }

    #[test]
    fn active_free_public_packages_are_held_by_everyone() {
        let free = package(WasmPackageVisibility::Public, 0, WasmPackageStatus::Active);
        assert!(block(&free, None, false).is_none());
        assert!(block(&free, Some(0), false).is_none());
    }

    #[test]
    fn a_non_zero_access_row_holds_any_package() {
        for visibility in [
            WasmPackageVisibility::Public,
            WasmPackageVisibility::PublicRequestAccess,
            WasmPackageVisibility::Private,
        ] {
            let paid = package(visibility, 499, WasmPackageStatus::Disabled);
            assert!(block(&paid, Some(BUYER), false).is_none());
        }
    }

    #[test]
    fn paid_public_packages_can_be_bought() {
        let paid = package(
            WasmPackageVisibility::Public,
            499,
            WasmPackageStatus::Active,
        );
        let blocked = block(&paid, None, true).unwrap();
        assert_eq!(blocked.block, PackageBlock::Paid);
        assert_eq!(blocked.price, 499);
        assert_eq!(blocked.name, "Chart Kit");
        assert!(!blocked.request_pending);
    }

    #[test]
    fn request_access_packages_never_carry_a_price() {
        let paid = package(
            WasmPackageVisibility::PublicRequestAccess,
            499,
            WasmPackageStatus::Active,
        );
        let blocked = block(&paid, None, true).unwrap();
        assert_eq!(blocked.block, PackageBlock::RequestAccess);
        assert_eq!(blocked.price, 0);
        assert!(blocked.request_pending);
    }

    #[test]
    fn private_packages_hide_their_registry_name() {
        let private = package(WasmPackageVisibility::Private, 0, WasmPackageStatus::Active);
        let blocked = block(&private, None, false).unwrap();
        assert_eq!(blocked.block, PackageBlock::Private);
        assert_eq!(blocked.name, "chart-kit");
        assert!(blocked.skipped_item().reason.starts_with("chart-kit "));
    }

    #[test]
    fn inactive_packages_are_unavailable_even_when_free() {
        for status in [
            WasmPackageStatus::PendingReview,
            WasmPackageStatus::Rejected,
            WasmPackageStatus::Deprecated,
            WasmPackageStatus::Disabled,
        ] {
            let free = package(WasmPackageVisibility::Public, 0, status.clone());
            let paid = package(WasmPackageVisibility::Public, 499, status);
            for blocked in [block(&free, None, false), block(&paid, Some(0), false)] {
                let blocked = blocked.unwrap();
                assert_eq!(blocked.block, PackageBlock::Unavailable);
                assert_eq!(blocked.price, 0);
            }
        }
    }

    #[test]
    fn a_zero_permission_row_is_revoked_access() {
        for visibility in [
            WasmPackageVisibility::Public,
            WasmPackageVisibility::PublicRequestAccess,
            WasmPackageVisibility::Private,
        ] {
            let paid = package(visibility, 499, WasmPackageStatus::Active);
            let blocked = block(&paid, Some(0), true).unwrap();
            assert_eq!(blocked.block, PackageBlock::Revoked);
            assert_eq!(blocked.price, 0);
            assert!(!blocked.request_pending);
        }
    }

    #[test]
    fn skipped_items_name_the_package_and_the_reason() {
        let paid = package(
            WasmPackageVisibility::Public,
            499,
            WasmPackageStatus::Active,
        );
        let item = block(&paid, None, false).unwrap().skipped_item();
        assert!(matches!(item.kind, SkippedKind::Package));
        assert_eq!(item.source_id, "chart-kit");
        assert_eq!(item.reason, "Chart Kit is a paid package you don't own");
    }

    fn pin(app_id: &str, package_id: &str) -> app_package::Model {
        app_package::Model {
            id: format!("{app_id}:{package_id}"),
            app_id: app_id.to_string(),
            membership_id: None,
            package_id: package_id.to_string(),
            version: "1.0.0".to_string(),
            added_at: chrono::Utc::now().fixed_offset(),
            auto_update: true,
            stale: false,
            stale_since: None,
        }
    }

    #[test]
    fn missing_pins_are_the_source_pins_the_copy_lacks() {
        let source = [pin("template", "chart-kit"), pin("template", "geo-tools")];
        let copy = [pin("copy", "geo-tools"), pin("copy", "own-addition")];
        let missing = missing_pins(&source, &copy);
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].package_id, "chart-kit");
        assert!(missing_pins(&source, &source).is_empty());
    }

    const OWNER: i64 = 0b0001;
    const MAINTAINER: i64 = 0b0010;

    fn version(version: &str, status: WasmPackageStatus) -> VersionStatus {
        VersionStatus {
            package_id: "chart-kit".to_string(),
            version: version.to_string(),
            status,
        }
    }

    #[test]
    fn only_managers_and_private_packages_see_unapproved_versions() {
        let public = package(
            WasmPackageVisibility::Public,
            499,
            WasmPackageStatus::Active,
        );
        assert!(!sees_all_versions(&public, None));
        assert!(!sees_all_versions(&public, Some(BUYER)));
        assert!(sees_all_versions(&public, Some(OWNER)));
        assert!(sees_all_versions(&public, Some(MAINTAINER | BUYER)));

        let private = package(WasmPackageVisibility::Private, 0, WasmPackageStatus::Active);
        assert!(sees_all_versions(&private, Some(BUYER)));
    }

    #[test]
    fn a_pinned_version_must_be_approved_unless_the_forker_sees_all() {
        assert!(can_download(false, Some(&WasmPackageStatus::Active)));
        assert!(can_download(true, Some(&WasmPackageStatus::Active)));
        for status in [
            WasmPackageStatus::PendingReview,
            WasmPackageStatus::Rejected,
        ] {
            assert!(!can_download(false, Some(&status)));
            assert!(can_download(true, Some(&status)));
        }
    }

    #[test]
    fn a_version_without_a_row_is_downloadable_by_nobody() {
        assert!(!can_download(false, None));
        assert!(!can_download(true, None));
    }

    #[test]
    fn the_fallback_is_the_newest_approved_version() {
        let newest_first = [
            version("2.0.0", WasmPackageStatus::PendingReview),
            version("1.5.0", WasmPackageStatus::Rejected),
            version("1.4.0", WasmPackageStatus::Active),
            version("1.3.0", WasmPackageStatus::Active),
        ];
        assert_eq!(fallback_version(false, &newest_first), Some("1.4.0"));
        assert_eq!(fallback_version(true, &newest_first), Some("2.0.0"));
    }

    #[test]
    fn a_package_without_an_approved_version_has_no_fallback() {
        let unapproved = [version("0.1.0", WasmPackageStatus::PendingReview)];
        assert_eq!(fallback_version(false, &unapproved), None);
        assert_eq!(fallback_version(false, &[]), None);
        assert_eq!(fallback_version(true, &[]), None);
    }

    #[test]
    fn a_repinned_package_warns_with_both_versions() {
        let repinned = RepinnedPackage {
            package_id: "chart-kit".to_string(),
            name: "Chart Kit".to_string(),
            pinned_version: "2.0.0".to_string(),
            version: "1.4.0".to_string(),
        };
        assert_eq!(
            repinned.warning(),
            "Chart Kit 2.0.0 isn't published, so your copy uses 1.4.0. Flows built for 2.0.0 may need their nodes updated."
        );
    }
}
