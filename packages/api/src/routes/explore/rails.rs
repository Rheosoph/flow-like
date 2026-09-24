use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use futures::future::try_join_all;
use sea_orm::{ConnectionTrait, Select};

use super::audience::DEFAULT_LANGUAGE;
use super::model::RailKey;
use super::query::{self, AppFilter, ExploreSort, PackageCategoryPair, PackageFilter};
use super::resolve::{RailCategory, RailData, RailStat, TypeCounts};
use crate::entity::{app, wasm_package};
use crate::error::ApiError;

/// "New" means created (apps) or published (packages) within this many days.
pub const RECENT_DAYS: i64 = 7;
/// Top paid ranks by purchases within this many days.
pub const SALES_DAYS: i64 = 30;
const TRENDING_APPS: u64 = 6;
const TRENDING_PACKAGES: u64 = 4;
const NEW_ITEMS: u64 = 8;
const TOP_PAID_ITEMS: u64 = 4;
const BUILDER_PACKAGES: u64 = 4;
const CATEGORY_TILES: u64 = 4;

fn saturate(value: u64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// The system rails `keys` names, concurrently. Package parts only run for developer viewers.
pub async fn load<C: ConnectionTrait>(
    db: &C,
    keys: &[RailKey],
    dev: bool,
    now: DateTime<Utc>,
) -> Result<HashMap<RailKey, RailData>, ApiError> {
    let rails = try_join_all(keys.iter().map(|key| async move {
        Ok::<_, ApiError>((*key, rail(db, *key, dev, now).await?))
    }))
    .await?;
    Ok(rails.into_iter().collect())
}

pub async fn rail<C: ConnectionTrait>(
    db: &C,
    key: RailKey,
    dev: bool,
    now: DateTime<Utc>,
) -> Result<RailData, ApiError> {
    let recent = now - Duration::days(RECENT_DAYS);
    let sales_since = now - Duration::days(SALES_DAYS);
    match key {
        RailKey::Trending => {
            ranked(
                db,
                Some((query::app_ids(&AppFilter::new(DEFAULT_LANGUAGE), ExploreSort::Best), TRENDING_APPS)),
                dev.then(|| (query::package_ids(&PackageFilter::default(), ExploreSort::Best), TRENDING_PACKAGES)),
            )
            .await
        }
        RailKey::New => {
            let apps = AppFilter {
                created_since: Some(recent),
                ..AppFilter::new(DEFAULT_LANGUAGE)
            };
            let packages = PackageFilter {
                published_since: Some(recent),
                ..PackageFilter::default()
            };
            ranked(
                db,
                Some((query::app_ids(&apps, ExploreSort::Newest), NEW_ITEMS)),
                dev.then(|| (query::package_ids(&packages, ExploreSort::Newest), NEW_ITEMS)),
            )
            .await
        }
        RailKey::TopPaid => {
            ranked(
                db,
                Some((query::top_paid_apps(sales_since.date_naive()), TOP_PAID_ITEMS)),
                dev.then(|| (query::top_paid_packages(sales_since), TOP_PAID_ITEMS)),
            )
            .await
        }
        RailKey::ForBuilders => {
            ranked(db, None, dev.then(|| (query::builder_packages(), BUILDER_PACKAGES))).await
        }
        RailKey::NewCount => new_count(db, recent, dev).await,
        RailKey::ByCategory => by_category(db, dev).await,
        RailKey::Suites => Ok(RailData::default()),
    }
}

async fn ranked<C: ConnectionTrait>(
    db: &C,
    apps: Option<(Select<app::Entity>, u64)>,
    packages: Option<(Select<wasm_package::Entity>, u64)>,
) -> Result<RailData, ApiError> {
    let (apps, packages) = futures::try_join!(
        async {
            match apps {
                Some((select, limit)) => query::ids(db, select, 0, limit).await,
                None => Ok(Vec::new()),
            }
        },
        async {
            match packages {
                Some((select, limit)) => query::ids(db, select, 0, limit).await,
                None => Ok(Vec::new()),
            }
        },
    )?;
    Ok(RailData {
        apps,
        packages,
        ..RailData::default()
    })
}

async fn new_count<C: ConnectionTrait>(
    db: &C,
    since: DateTime<Utc>,
    dev: bool,
) -> Result<RailData, ApiError> {
    let apps = AppFilter {
        created_since: Some(since),
        ..AppFilter::new(DEFAULT_LANGUAGE)
    };
    let packages = PackageFilter {
        published_since: Some(since),
        ..PackageFilter::default()
    };
    let ((apps, free_apps, paid_apps), (packages, verified_packages)) = futures::try_join!(
        query::app_price_split(db, &apps),
        async {
            if dev {
                query::package_verified_counts(db, &packages).await
            } else {
                Ok((0, 0))
            }
        },
    )?;
    Ok(RailData {
        stat: Some(RailStat {
            total: saturate(apps + packages),
            apps: saturate(apps),
            packages: saturate(packages),
            free_apps: saturate(free_apps),
            paid_apps: saturate(paid_apps),
            verified_packages: saturate(verified_packages),
        }),
        ..RailData::default()
    })
}

/// Packages per app category (serde name): a package counts once for every app category its primary or
/// secondary category maps to.
pub fn packages_per_app_category(pairs: &[PackageCategoryPair]) -> HashMap<&'static str, u64> {
    let mut counts = HashMap::new();
    for (primary, secondary, count) in pairs {
        for name in query::package_pair_app_names(primary.as_ref(), secondary.as_ref()) {
            *counts.entry(name).or_default() += count;
        }
    }
    counts
}

async fn by_category<C: ConnectionTrait>(db: &C, dev: bool) -> Result<RailData, ApiError> {
    let (top, pairs) = futures::try_join!(query::top_app_categories(db, CATEGORY_TILES), async {
        if dev {
            query::package_category_pairs(db, &PackageFilter::default()).await
        } else {
            Ok(Vec::new())
        }
    })?;
    let packages = packages_per_app_category(&pairs);
    Ok(RailData {
        categories: top
            .into_iter()
            .map(|(category, apps)| {
                let app_category = query::app_category_label(&category);
                RailCategory {
                    packages: saturate(packages.get(app_category.as_str()).copied().unwrap_or_default()),
                    app_category,
                    apps: saturate(apps),
                }
            })
            .collect(),
        ..RailData::default()
    })
}

/// Public apps, and for developer viewers public packages, on the whole hub.
pub async fn type_counts<C: ConnectionTrait>(db: &C, dev: bool) -> Result<TypeCounts, ApiError> {
    let apps = AppFilter::new(DEFAULT_LANGUAGE);
    let (apps, packages) = futures::try_join!(
        query::count_apps(db, &apps),
        async {
            if dev {
                query::count_packages(db, &PackageFilter::default()).await
            } else {
                Ok(0)
            }
        },
    )?;
    Ok(TypeCounts { apps, packages })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::sea_orm_active_enums::WasmPackageCategory as DbPackageCategory;

    #[test]
    fn a_package_counts_once_per_mapped_app_category() {
        let counts = packages_per_app_category(&[
            (
                Some(DbPackageCategory::FinanceBilling),
                Some(DbPackageCategory::Insurance),
                3,
            ),
            (Some(DbPackageCategory::Insurance), None, 2),
            (
                Some(DbPackageCategory::MediaContent),
                Some(DbPackageCategory::Education),
                1,
            ),
            (Some(DbPackageCategory::Legal), None, 7),
            (None, None, 4),
        ]);
        assert_eq!(counts.get("Finance"), Some(&5));
        assert_eq!(counts.get("Education"), Some(&1));
        assert_eq!(counts.get("Music"), Some(&1));
        assert_eq!(counts.get("News"), Some(&1));
        assert_eq!(counts.len(), 6);
    }
}
