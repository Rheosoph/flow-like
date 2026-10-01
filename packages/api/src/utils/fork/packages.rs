use super::{SkippedItem, SkippedKind};
use crate::{
    entity::{
        app_package,
        sea_orm_active_enums::{WasmPackageStatus, WasmPackageVisibility},
        wasm_package, wasm_package_join_queue, wasm_package_user,
    },
    error::ApiError,
    package_license,
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QuerySelect};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
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

/// A package the fork drops because the forker doesn't hold it.
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

/// Splits the source app's pins into the ones the forker holds and the ones
/// the fork drops. Holding follows the project licence rule
/// ([`package_license::user_holds`]): free public packages are held by
/// everyone, everything else needs a non-zero `WasmPackageUser` row. A
/// package that isn't active is held only through such a row, because the
/// registry serves it to nobody else. An anonymous forker (`None`) holds only
/// active free public packages.
pub async fn split_pins<C: ConnectionTrait>(
    db: &C,
    user_sub: Option<&str>,
    pins: &[app_package::Model],
) -> Result<(Vec<app_package::Model>, Vec<BlockedPackage>), ApiError> {
    let blocked = blocked_packages(db, user_sub, pins).await?;
    let blocked_ids: HashSet<&str> = blocked.iter().map(|b| b.package_id.as_str()).collect();
    let held = pins
        .iter()
        .filter(|pin| !blocked_ids.contains(pin.package_id.as_str()))
        .cloned()
        .collect();
    Ok((held, blocked))
}

/// The manifest's `package_id -> version` map for a set of pins.
pub fn pin_map(pins: &[app_package::Model]) -> HashMap<String, String> {
    pins.iter()
        .map(|pin| (pin.package_id.clone(), pin.version.clone()))
        .collect()
}

async fn blocked_packages<C: ConnectionTrait>(
    db: &C,
    user_sub: Option<&str>,
    pins: &[app_package::Model],
) -> Result<Vec<BlockedPackage>, ApiError> {
    let package_ids: Vec<String> = pins
        .iter()
        .map(|pin| pin.package_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if package_ids.is_empty() {
        return Ok(Vec::new());
    }

    let (packages, access, pending) = flow_like_types::tokio::try_join!(
        registry_rows(db, &package_ids),
        access_permissions(db, user_sub, &package_ids),
        pending_request_ids(db, user_sub, &package_ids),
    )?;

    Ok(package_ids
        .into_iter()
        .filter_map(|package_id| {
            let package = packages.get(&package_id);
            let permission = access.get(&package_id).copied();
            let pending = pending.contains(&package_id);
            classify(package_id, package, permission, pending)
        })
        .collect())
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
}
