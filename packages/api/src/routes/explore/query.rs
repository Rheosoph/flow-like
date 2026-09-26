use std::collections::BTreeSet;

use chrono::{DateTime, NaiveDate, Utc};
use flow_like::app::{AppCategory, AppType};
use flow_like_wasm_schema::manifest::WasmPackageCategory;
use sea_orm::sea_query::{
    Alias, Expr, ExprTrait, Func, LikeExpr, NullOrdering, Order, Query, SelectStatement,
    extension::postgres::PgExpr,
};
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, EntityName, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Select, Value,
};

use super::audience::DEFAULT_LANGUAGE;
use super::model::{
    CollectionRule, ItemKind, PriceFilter, RuleSort, app_categories_for, app_category_name,
    package_category_from_db, package_category_to_db, parse_app_category,
};
use crate::entity::sea_orm_active_enums::{
    AppType as DbAppType, Category, PurchaseStatus, Status, Visibility,
    WasmPackageCategory as DbPackageCategory, WasmPackageStatus, WasmPackageVisibility,
};
use crate::entity::{app, app_sales_daily, meta, wasm_package, wasm_package_purchase};
use crate::error::ApiError;

/// Most packages a permission-filtered search looks at; permissions exist only in Rust.
pub const PACKAGE_WINDOW: u64 = 200;

/// Result order of Explore queries. Nullable keys sort last, and every order ends with the id.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExploreSort {
    #[default]
    Best,
    Newest,
    Rating,
    Installs,
    Name,
    Updated,
}

impl ExploreSort {
    pub fn parse(raw: Option<&str>) -> Result<Self, ApiError> {
        match raw.map(str::trim).filter(|value| !value.is_empty()) {
            None | Some("best") => Ok(Self::Best),
            Some("newest") => Ok(Self::Newest),
            Some("rating") => Ok(Self::Rating),
            Some("installs") => Ok(Self::Installs),
            Some("name") => Ok(Self::Name),
            Some("updated") => Ok(Self::Updated),
            Some(other) => Err(ApiError::bad_request(format!(
                "sort value '{other}' is not supported; expected best, newest, rating, installs, name or updated"
            ))),
        }
    }
}

impl From<RuleSort> for ExploreSort {
    fn from(sort: RuleSort) -> Self {
        match sort {
            RuleSort::Installs => Self::Installs,
            RuleSort::Rating => Self::Rating,
            RuleSort::Newest => Self::Newest,
        }
    }
}

/// `text` with the LIKE wildcards `%`, `_` and the escape character `\` escaped.
pub fn escape_like(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

/// PostgreSQL's default LIKE escape is `\`. An explicit `ESCAPE` would make SeaQuery render
/// `ILIKE (pattern ESCAPE '\')`, which PostgreSQL rejects as a syntax error.
fn containing(text: &str) -> LikeExpr {
    LikeExpr::new(format!("%{}%", escape_like(text)))
}

fn never() -> Expr {
    Expr::cust("FALSE")
}

fn count_all() -> Expr {
    Expr::cust("COUNT(*)")
}

fn count_where(condition: Expr) -> Expr {
    Expr::cust_with_expr("COUNT(*) FILTER (WHERE $1)", condition)
}

fn scalar(select: SelectStatement) -> Expr {
    Expr::SubQuery(None, Box::new(select.into()))
}

pub fn public_app_condition() -> Condition {
    Condition::all()
        .add(app::Column::Visibility.is_in([Visibility::Public, Visibility::PublicRequestAccess]))
        .add(app::Column::Status.eq(Status::Active))
}

/// Public, active packages. Applied to every direct `WasmPackage` query before ORDER, LIMIT and COUNT, so ranked
/// lists are never short and counts never reveal hidden packages.
pub fn public_package_condition() -> Condition {
    Condition::all()
        .add(wasm_package::Column::Visibility.is_in([
            WasmPackageVisibility::Public,
            WasmPackageVisibility::PublicRequestAccess,
        ]))
        .add(wasm_package::Column::Status.eq(WasmPackageStatus::Active))
}

/// Primary or secondary category in `values`; an empty list matches nothing.
fn either_category<C, V>(primary: C, secondary: C, values: &[V]) -> Condition
where
    C: ColumnTrait,
    V: Into<Value> + Clone,
{
    if values.is_empty() {
        return Condition::all().add(never());
    }
    Condition::any()
        .add(primary.is_in(values.to_vec()))
        .add(secondary.is_in(values.to_vec()))
}

fn price_condition<C: ColumnTrait>(column: C, price: PriceFilter) -> Expr {
    match price {
        PriceFilter::Free => column.eq(0i64),
        PriceFilter::Paid => column.gt(0i64),
    }
}

/// `EXISTS` a `Meta` row of the app in the viewer language or English whose name or description contains `text`.
/// A correlated predicate, so the outer query still pages over `App` rows only.
fn meta_mentions(text: &str, language: &str) -> Expr {
    let m = Alias::new("m");
    let pattern = containing(text);
    Expr::exists(
        Query::select()
            .expr(Expr::val(1))
            .from(meta::Entity.table_ref().alias(m.clone()))
            .and_where(
                Expr::col((m.clone(), meta::Column::AppId)).equals((app::Entity, app::Column::Id)),
            )
            .and_where(
                Expr::col((m.clone(), meta::Column::Lang))
                    .is_in([language.to_owned(), DEFAULT_LANGUAGE.to_owned()]),
            )
            .and_where(
                Expr::col((m.clone(), meta::Column::Name))
                    .ilike(pattern.clone())
                    .or(Expr::col((m, meta::Column::Description)).ilike(pattern)),
            )
            .to_owned(),
    )
}

/// One display name per app for `sort=name`: the viewer language first, then English.
fn localized_name(language: &str) -> Expr {
    let m = Alias::new("m");
    scalar(
        Query::select()
            .column((m.clone(), meta::Column::Name))
            .from(meta::Entity.table_ref().alias(m.clone()))
            .and_where(
                Expr::col((m.clone(), meta::Column::AppId)).equals((app::Entity, app::Column::Id)),
            )
            .and_where(
                Expr::col((m.clone(), meta::Column::Lang))
                    .is_in([language.to_owned(), DEFAULT_LANGUAGE.to_owned()]),
            )
            .order_by_expr(
                Expr::col((m, meta::Column::Lang)).eq(language.to_owned()),
                Order::Desc,
            )
            .limit(1)
            .to_owned(),
    )
}

/// Predicates over public, active apps shared by rails, rule collections and search.
#[derive(Clone, Debug)]
pub struct AppFilter {
    pub language: String,
    pub text: Option<String>,
    /// Primary or secondary category in the list; `Some(empty)` matches nothing.
    pub categories: Option<Vec<Category>>,
    pub price: Option<PriceFilter>,
    pub app_type: Option<DbAppType>,
    pub min_rating: Option<f32>,
    pub created_since: Option<DateTime<Utc>>,
    pub exclude: Vec<String>,
}

impl AppFilter {
    pub fn new(language: &str) -> Self {
        Self {
            language: language.to_owned(),
            text: None,
            categories: None,
            price: None,
            app_type: None,
            min_rating: None,
            created_since: None,
            exclude: Vec::new(),
        }
    }

    pub fn condition(&self) -> Condition {
        let mut condition = public_app_condition();
        if let Some(text) = &self.text {
            condition = condition.add(meta_mentions(text, &self.language));
        }
        if let Some(categories) = &self.categories {
            condition = condition.add(either_category(
                app::Column::PrimaryCategory,
                app::Column::SecondaryCategory,
                categories,
            ));
        }
        if let Some(price) = self.price {
            condition = condition.add(price_condition(app::Column::Price, price));
        }
        if let Some(app_type) = &self.app_type {
            condition = condition.add(app::Column::AppType.eq(app_type.clone()));
        }
        if let Some(rating) = self.min_rating {
            condition = condition.add(app::Column::AvgRating.gte(f64::from(rating)));
        }
        if let Some(since) = self.created_since {
            condition = condition.add(app::Column::CreatedAt.gte(since.fixed_offset()));
        }
        if !self.exclude.is_empty() {
            condition = condition.add(app::Column::Id.is_not_in(self.exclude.clone()));
        }
        condition
    }
}

/// Predicates over public, active packages shared by rails, rule collections and search.
#[derive(Clone, Debug, Default)]
pub struct PackageFilter {
    pub text: Option<String>,
    /// Packages of the collections a search matched; they count as text matches.
    pub collection_ids: Vec<String>,
    /// Primary or secondary category in the list; `Some(empty)` matches nothing.
    pub categories: Option<Vec<DbPackageCategory>>,
    pub price: Option<PriceFilter>,
    pub verified_only: bool,
    pub min_rating: Option<f32>,
    pub published_since: Option<DateTime<Utc>>,
}

impl PackageFilter {
    pub fn condition(&self) -> Condition {
        let mut condition = public_package_condition();
        if let Some(text) = &self.text {
            let pattern = containing(text);
            let mut matched = Condition::any()
                .add(
                    wasm_package::Column::Name
                        .into_expr()
                        .ilike(pattern.clone()),
                )
                .add(
                    wasm_package::Column::Description
                        .into_expr()
                        .ilike(pattern.clone()),
                )
                .add(wasm_package::Column::Id.into_expr().ilike(pattern));
            if !self.collection_ids.is_empty() {
                matched = matched.add(wasm_package::Column::Id.is_in(self.collection_ids.clone()));
            }
            condition = condition.add(matched);
        }
        if let Some(categories) = &self.categories {
            condition = condition.add(either_category(
                wasm_package::Column::PrimaryCategory,
                wasm_package::Column::SecondaryCategory,
                categories,
            ));
        }
        if let Some(price) = self.price {
            condition = condition.add(price_condition(wasm_package::Column::Price, price));
        }
        if self.verified_only {
            condition = condition.add(wasm_package::Column::Verified.eq(true));
        }
        if let Some(rating) = self.min_rating {
            condition = condition.add(wasm_package::Column::AvgRating.gte(f64::from(rating)));
        }
        if let Some(since) = self.published_since {
            condition = condition.add(wasm_package::Column::PublishedAt.gte(since.fixed_offset()));
        }
        condition
    }
}

fn order_apps(
    select: Select<app::Entity>,
    sort: ExploreSort,
    language: &str,
) -> Select<app::Entity> {
    use app::Column as C;
    let select = match sort {
        ExploreSort::Best | ExploreSort::Installs => select
            .order_by_desc(C::DownloadCount)
            .order_by_desc(C::RatingSum),
        ExploreSort::Newest => select.order_by_desc(C::CreatedAt),
        ExploreSort::Rating => select
            .order_by_with_nulls(C::AvgRating, Order::Desc, NullOrdering::Last)
            .order_by_desc(C::RatingCount),
        ExploreSort::Updated => select.order_by_desc(C::UpdatedAt),
        ExploreSort::Name => {
            select.order_by_with_nulls(localized_name(language), Order::Asc, NullOrdering::Last)
        }
    };
    select.order_by_asc(C::Id)
}

fn order_packages(
    select: Select<wasm_package::Entity>,
    sort: ExploreSort,
) -> Select<wasm_package::Entity> {
    use wasm_package::Column as C;
    let select = match sort {
        ExploreSort::Best | ExploreSort::Installs => select.order_by_desc(C::DownloadCount),
        ExploreSort::Newest => {
            select.order_by_with_nulls(C::PublishedAt, Order::Desc, NullOrdering::Last)
        }
        ExploreSort::Rating => select
            .order_by_with_nulls(C::AvgRating, Order::Desc, NullOrdering::Last)
            .order_by_desc(C::RatingCount),
        ExploreSort::Name => select.order_by_asc(C::Name),
        ExploreSort::Updated => select.order_by_desc(C::UpdatedAt),
    };
    select.order_by_asc(C::Id)
}

/// Ranked app ids over `App` rows only: metadata never joins in, so each app takes one slot per page.
pub fn app_ids(filter: &AppFilter, sort: ExploreSort) -> Select<app::Entity> {
    order_apps(
        app::Entity::find()
            .select_only()
            .column(app::Column::Id)
            .filter(filter.condition()),
        sort,
        &filter.language,
    )
}

pub fn package_ids(filter: &PackageFilter, sort: ExploreSort) -> Select<wasm_package::Entity> {
    order_packages(
        wasm_package::Entity::find()
            .select_only()
            .column(wasm_package::Column::Id)
            .filter(filter.condition()),
        sort,
    )
}

/// Paid public apps by purchases in the last 30 days (none sorts last), then rating sum and id.
pub fn top_paid_apps(since: NaiveDate) -> Select<app::Entity> {
    let sales = Alias::new("sales");
    let purchases = Query::select()
        .expr(Func::coalesce([
            Expr::col((sales.clone(), app_sales_daily::Column::PurchaseCount)).sum(),
            Expr::val(0i64),
        ]))
        .from(app_sales_daily::Entity.table_ref().alias(sales.clone()))
        .and_where(
            Expr::col((sales.clone(), app_sales_daily::Column::AppId))
                .equals((app::Entity, app::Column::Id)),
        )
        .and_where(Expr::col((sales, app_sales_daily::Column::Date)).gte(since))
        .to_owned();
    app::Entity::find()
        .select_only()
        .column(app::Column::Id)
        .filter(public_app_condition())
        .filter(app::Column::Price.gt(0i64))
        .order_by(scalar(purchases), Order::Desc)
        .order_by_desc(app::Column::RatingSum)
        .order_by_asc(app::Column::Id)
}

/// Paid public packages by completed purchases since `since` (none sorts last), then downloads and id.
pub fn top_paid_packages(since: DateTime<Utc>) -> Select<wasm_package::Entity> {
    let bought = Alias::new("bought");
    let purchases = Query::select()
        .expr(count_all())
        .from(
            wasm_package_purchase::Entity
                .table_ref()
                .alias(bought.clone()),
        )
        .and_where(
            Expr::col((bought.clone(), wasm_package_purchase::Column::PackageId))
                .equals((wasm_package::Entity, wasm_package::Column::Id)),
        )
        .and_where(
            Expr::col((bought.clone(), wasm_package_purchase::Column::Status))
                .eq(Expr::val(PurchaseStatus::Completed)),
        )
        .and_where(
            Expr::col((bought, wasm_package_purchase::Column::CompletedAt))
                .gte(since.fixed_offset()),
        )
        .to_owned();
    wasm_package::Entity::find()
        .select_only()
        .column(wasm_package::Column::Id)
        .filter(public_package_condition())
        .filter(wasm_package::Column::Price.gt(0i64))
        .order_by(scalar(purchases), Order::Desc)
        .order_by_desc(wasm_package::Column::DownloadCount)
        .order_by_asc(wasm_package::Column::Id)
}

/// Public packages for builders: verified first, then downloads and id.
pub fn builder_packages() -> Select<wasm_package::Entity> {
    wasm_package::Entity::find()
        .select_only()
        .column(wasm_package::Column::Id)
        .filter(public_package_condition())
        .order_by_desc(wasm_package::Column::Verified)
        .order_by_desc(wasm_package::Column::DownloadCount)
        .order_by_asc(wasm_package::Column::Id)
}

pub async fn ids<E, C>(
    db: &C,
    select: Select<E>,
    offset: u64,
    limit: u64,
) -> Result<Vec<String>, ApiError>
where
    E: EntityTrait,
    C: ConnectionTrait,
{
    if limit == 0 {
        return Ok(Vec::new());
    }
    Ok(select
        .offset(offset)
        .limit(limit)
        .into_tuple::<String>()
        .all(db)
        .await?)
}

fn unsigned(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default()
}

/// `COUNT(*)` of the apps under `filter`: apps, never metadata rows.
pub async fn count_apps<C: ConnectionTrait>(db: &C, filter: &AppFilter) -> Result<u64, ApiError> {
    let total = app::Entity::find()
        .select_only()
        .expr(count_all())
        .filter(filter.condition())
        .into_tuple::<i64>()
        .one(db)
        .await?;
    Ok(total.map(unsigned).unwrap_or_default())
}

pub async fn count_packages<C: ConnectionTrait>(
    db: &C,
    filter: &PackageFilter,
) -> Result<u64, ApiError> {
    let total = wasm_package::Entity::find()
        .select_only()
        .expr(count_all())
        .filter(filter.condition())
        .into_tuple::<i64>()
        .one(db)
        .await?;
    Ok(total.map(unsigned).unwrap_or_default())
}

/// `(free, paid)` apps under `filter`.
pub async fn app_price_counts<C: ConnectionTrait>(
    db: &C,
    filter: &AppFilter,
) -> Result<(u64, u64), ApiError> {
    let row = app::Entity::find()
        .select_only()
        .expr(count_where(price_condition(
            app::Column::Price,
            PriceFilter::Free,
        )))
        .expr(count_where(price_condition(
            app::Column::Price,
            PriceFilter::Paid,
        )))
        .filter(filter.condition())
        .into_tuple::<(i64, i64)>()
        .one(db)
        .await?
        .unwrap_or_default();
    Ok((unsigned(row.0), unsigned(row.1)))
}

/// `(free, paid)` packages under `filter`.
pub async fn package_price_counts<C: ConnectionTrait>(
    db: &C,
    filter: &PackageFilter,
) -> Result<(u64, u64), ApiError> {
    let row = wasm_package::Entity::find()
        .select_only()
        .expr(count_where(price_condition(
            wasm_package::Column::Price,
            PriceFilter::Free,
        )))
        .expr(count_where(price_condition(
            wasm_package::Column::Price,
            PriceFilter::Paid,
        )))
        .filter(filter.condition())
        .into_tuple::<(i64, i64)>()
        .one(db)
        .await?
        .unwrap_or_default();
    Ok((unsigned(row.0), unsigned(row.1)))
}

/// `(total, verified)` packages under `filter`.
pub async fn package_verified_counts<C: ConnectionTrait>(
    db: &C,
    filter: &PackageFilter,
) -> Result<(u64, u64), ApiError> {
    let row = wasm_package::Entity::find()
        .select_only()
        .expr(count_all())
        .expr(count_where(wasm_package::Column::Verified.eq(true)))
        .filter(filter.condition())
        .into_tuple::<(i64, i64)>()
        .one(db)
        .await?
        .unwrap_or_default();
    Ok((unsigned(row.0), unsigned(row.1)))
}

/// `(total, free, paid)` apps under `filter`.
pub async fn app_price_split<C: ConnectionTrait>(
    db: &C,
    filter: &AppFilter,
) -> Result<(u64, u64, u64), ApiError> {
    let row = app::Entity::find()
        .select_only()
        .expr(count_all())
        .expr(count_where(price_condition(
            app::Column::Price,
            PriceFilter::Free,
        )))
        .expr(count_where(price_condition(
            app::Column::Price,
            PriceFilter::Paid,
        )))
        .filter(filter.condition())
        .into_tuple::<(i64, i64, i64)>()
        .one(db)
        .await?
        .unwrap_or_default();
    Ok((unsigned(row.0), unsigned(row.1), unsigned(row.2)))
}

pub type AppCategoryPair = (Option<Category>, Option<Category>, u64);
pub type PackageCategoryPair = (Option<DbPackageCategory>, Option<DbPackageCategory>, u64);

/// `(primaryCategory, secondaryCategory, COUNT(*))` of the apps under `filter`, one row per pair.
pub async fn app_category_pairs<C: ConnectionTrait>(
    db: &C,
    filter: &AppFilter,
) -> Result<Vec<AppCategoryPair>, ApiError> {
    Ok(app::Entity::find()
        .select_only()
        .column(app::Column::PrimaryCategory)
        .column(app::Column::SecondaryCategory)
        .expr(count_all())
        .filter(filter.condition())
        .group_by(app::Column::PrimaryCategory)
        .group_by(app::Column::SecondaryCategory)
        .into_tuple::<(Option<Category>, Option<Category>, i64)>()
        .all(db)
        .await?
        .into_iter()
        .map(|(primary, secondary, count)| (primary, secondary, unsigned(count)))
        .collect())
}

/// `(primaryCategory, secondaryCategory, COUNT(*))` of the packages under `filter`, one row per pair.
pub async fn package_category_pairs<C: ConnectionTrait>(
    db: &C,
    filter: &PackageFilter,
) -> Result<Vec<PackageCategoryPair>, ApiError> {
    Ok(wasm_package::Entity::find()
        .select_only()
        .column(wasm_package::Column::PrimaryCategory)
        .column(wasm_package::Column::SecondaryCategory)
        .expr(count_all())
        .filter(filter.condition())
        .group_by(wasm_package::Column::PrimaryCategory)
        .group_by(wasm_package::Column::SecondaryCategory)
        .into_tuple::<(Option<DbPackageCategory>, Option<DbPackageCategory>, i64)>()
        .all(db)
        .await?
        .into_iter()
        .map(|(primary, secondary, count)| (primary, secondary, unsigned(count)))
        .collect())
}

/// `(primaryCategory, apps)` of the public apps, the four categories with the most downloads first.
pub async fn top_app_categories<C: ConnectionTrait>(
    db: &C,
    limit: u64,
) -> Result<Vec<(Category, u64)>, ApiError> {
    Ok(app::Entity::find()
        .select_only()
        .column(app::Column::PrimaryCategory)
        .expr(count_all())
        .filter(public_app_condition())
        .filter(app::Column::PrimaryCategory.is_not_null())
        .group_by(app::Column::PrimaryCategory)
        .order_by(app::Column::DownloadCount.into_expr().sum(), Order::Desc)
        .order_by_asc(app::Column::PrimaryCategory)
        .limit(limit)
        .into_tuple::<(Category, i64)>()
        .all(db)
        .await?
        .into_iter()
        .map(|(category, count)| (category, unsigned(count)))
        .collect())
}

/// Core `AppCategory` serde name of an entity category ("Finance").
pub fn app_category_label(category: &Category) -> String {
    app_category_name(&AppCategory::from(category.clone()))
}

/// The app categories an app's category pair names, each once.
pub fn app_pair_names(
    primary: Option<&Category>,
    secondary: Option<&Category>,
) -> BTreeSet<String> {
    primary
        .into_iter()
        .chain(secondary)
        .map(app_category_label)
        .collect()
}

/// The package categories (SCREAMING_SNAKE) a package's category pair names, each once.
pub fn package_pair_names(
    primary: Option<&DbPackageCategory>,
    secondary: Option<&DbPackageCategory>,
) -> BTreeSet<String> {
    primary
        .into_iter()
        .chain(secondary)
        .map(|category| package_category_from_db(category).to_string())
        .collect()
}

/// The app categories a package's category pair maps to through `app_categories_for`, each once.
pub fn package_pair_app_names(
    primary: Option<&DbPackageCategory>,
    secondary: Option<&DbPackageCategory>,
) -> BTreeSet<&'static str> {
    primary
        .into_iter()
        .chain(secondary)
        .flat_map(|category| {
            app_categories_for(&package_category_from_db(category))
                .iter()
                .copied()
        })
        .collect()
}

/// The filter a rule collection of apps runs, or `None` when a stored category or type no longer parses (the
/// rule then selects nothing rather than everything).
pub fn app_rule_filter(rule: &CollectionRule, language: &str) -> Option<AppFilter> {
    let categories = match rule.app_category.as_deref() {
        Some(name) => Some(vec![Category::from(parse_app_category(name)?)]),
        None => None,
    };
    let app_type = match rule.app_type.as_deref() {
        Some(name) => Some(DbAppType::from(
            serde_json::from_value::<AppType>(serde_json::Value::String(name.to_owned())).ok()?,
        )),
        None => None,
    };
    Some(AppFilter {
        categories,
        app_type,
        price: rule.price,
        min_rating: rule.min_rating,
        ..AppFilter::new(language)
    })
}

/// The filter a rule collection of packages runs, or `None` when a stored category no longer parses.
pub fn package_rule_filter(rule: &CollectionRule) -> Option<PackageFilter> {
    let categories = match rule.package_category.as_deref() {
        Some(name) => Some(vec![package_category_to_db(
            WasmPackageCategory::from_str_opt(name)?,
        )]),
        None => None,
    };
    Some(PackageFilter {
        categories,
        price: rule.price,
        verified_only: rule.verified_only,
        min_rating: rule.min_rating,
        ..PackageFilter::default()
    })
}

/// Ids a rule collection selects, in rank order.
pub async fn rule_ids<C: ConnectionTrait>(
    db: &C,
    rule: &CollectionRule,
    language: &str,
    limit: u64,
) -> Result<Vec<(ItemKind, String)>, ApiError> {
    let sort = ExploreSort::from(rule.sort);
    let (kind, found) = match rule.item_kind {
        ItemKind::App => match app_rule_filter(rule, language) {
            Some(filter) => (
                ItemKind::App,
                ids(db, app_ids(&filter, sort), 0, limit).await?,
            ),
            None => return Ok(Vec::new()),
        },
        ItemKind::Package => match package_rule_filter(rule) {
            Some(filter) => (
                ItemKind::Package,
                ids(db, package_ids(&filter, sort), 0, limit).await?,
            ),
            None => return Ok(Vec::new()),
        },
        ItemKind::Collection => return Ok(Vec::new()),
    };
    Ok(found.into_iter().map(|id| (kind, id)).collect())
}

/// Run with FLOW_LIKE_EXPLORE_TEST_DATABASE_URL pointing to a disposable PostgreSQL server. The entities pin
/// the `public` schema, so each fixture creates its own database: the Explore migration, the coordination lock
/// table and the `App`/`Meta` columns the Explore queries read.
#[cfg(test)]
pub(crate) mod test_database {
    use sea_orm::{ConnectionTrait, Database, DatabaseBackend, DatabaseConnection, Statement};

    pub async fn execute(db: &DatabaseConnection, sql: &str) {
        db.execute_raw(Statement::from_string(DatabaseBackend::Postgres, sql))
            .await
            .unwrap();
    }

    const TABLES: &[&str] = &[
        r#"CREATE TABLE "MutationLock" (id BIGINT PRIMARY KEY, "updatedAt" TIMESTAMPTZ NOT NULL DEFAULT now())"#,
        r#"CREATE TABLE "App" (id TEXT PRIMARY KEY, visibility TEXT NOT NULL, status TEXT NOT NULL, "primaryCategory" TEXT, "secondaryCategory" TEXT, "appType" TEXT, price BIGINT NOT NULL DEFAULT 0, "downloadCount" BIGINT NOT NULL DEFAULT 0, "ratingSum" BIGINT NOT NULL DEFAULT 0, "ratingCount" BIGINT NOT NULL DEFAULT 0, "avgRating" DOUBLE PRECISION, "createdAt" TIMESTAMPTZ NOT NULL DEFAULT now(), "updatedAt" TIMESTAMPTZ NOT NULL DEFAULT now())"#,
        r#"CREATE TABLE "Meta" (id TEXT PRIMARY KEY, lang TEXT NOT NULL, name TEXT NOT NULL, description TEXT, "appId" TEXT)"#,
    ];

    pub struct Fixture {
        pub db: DatabaseConnection,
        server: String,
        name: String,
    }

    impl Fixture {
        pub async fn new() -> Self {
            let server = std::env::var("FLOW_LIKE_EXPLORE_TEST_DATABASE_URL")
                .expect("Use a disposable Explore test database");
            let setup = Database::connect(&server).await.unwrap();
            let name = format!("explore_{}", flow_like_types::create_id().to_lowercase());
            execute(&setup, &format!("CREATE DATABASE \"{name}\"")).await;
            setup.close().await.unwrap();
            let mut target = reqwest::Url::parse(&server).unwrap();
            target.set_path(&name);
            let db = Database::connect(target.as_str()).await.unwrap();
            let migration = include_str!(
                "../../../prisma/migrations/20260924200000_explore_layout/migration.sql"
            );
            for statement in migration
                .split(';')
                .filter(|statement| !statement.trim().is_empty())
                .chain(TABLES.iter().copied())
            {
                execute(&db, statement).await;
            }
            Self { db, server, name }
        }

        pub async fn drop_database(self) {
            self.db.close().await.unwrap();
            let setup = Database::connect(&self.server).await.unwrap();
            execute(
                &setup,
                &format!("DROP DATABASE \"{}\" WITH (FORCE)", self.name),
            )
            .await;
            setup.close().await.unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{DbBackend, QueryTrait};

    fn sql<E: EntityTrait>(select: Select<E>) -> String {
        select.build(DbBackend::Postgres).to_string()
    }

    fn position(sql: &str, needle: &str) -> usize {
        sql.find(needle)
            .unwrap_or_else(|| panic!("'{needle}' is missing from {sql}"))
    }

    #[test]
    fn like_wildcards_are_escaped() {
        assert_eq!(escape_like(r"50%_off\now"), r"50\%\_off\\now");
        assert_eq!(escape_like("plain"), "plain");
    }

    #[test]
    fn sorts_parse_and_reject_unknown_values() {
        assert_eq!(ExploreSort::parse(None).unwrap(), ExploreSort::Best);
        assert_eq!(ExploreSort::parse(Some("name")).unwrap(), ExploreSort::Name);
        let error = ExploreSort::parse(Some("relevance")).unwrap_err();
        assert_eq!(error.status(), axum::http::StatusCode::BAD_REQUEST);
        assert!(error.public_message().unwrap().contains("'relevance'"));
    }

    #[test]
    fn the_public_package_condition_precedes_order_and_limit() {
        let filter = PackageFilter {
            text: Some("pdf".into()),
            ..PackageFilter::default()
        };
        for select in [
            package_ids(&filter, ExploreSort::Rating),
            top_paid_packages(Utc::now()),
            builder_packages(),
        ] {
            let sql = sql(select.offset(0).limit(4));
            let visibility = position(
                &sql,
                r#""WasmPackage"."visibility" IN ('PUBLIC', 'PUBLIC_REQUEST_ACCESS')"#,
            );
            let status = position(&sql, r#""WasmPackage"."status" = 'ACTIVE'"#);
            let order = position(&sql, "ORDER BY");
            let limit = position(&sql, "LIMIT");
            assert!(
                visibility < order && status < order && order < limit,
                "{sql}"
            );
        }
        let sql = sql(package_ids(&filter, ExploreSort::Rating));
        assert!(
            str::contains(&sql, r#""WasmPackage"."avgRating" DESC NULLS LAST"#),
            "{sql}"
        );
        assert!(sql.ends_with(r#""WasmPackage"."id" ASC"#), "{sql}");
        assert!(!str::contains(&sql, "relevanceScore"));
    }

    #[test]
    fn the_app_page_pages_over_app_rows_without_joining_meta() {
        let filter = AppFilter {
            text: Some("in_voice%".into()),
            categories: Some(vec![Category::Finance]),
            price: Some(PriceFilter::Paid),
            ..AppFilter::new("de")
        };
        let sql = sql(app_ids(&filter, ExploreSort::Name).offset(24).limit(24));
        assert!(!str::contains(&sql, "JOIN"), "{sql}");
        assert!(
            sql.starts_with(r#"SELECT "App"."id" FROM "public"."App" WHERE"#),
            "{sql}"
        );
        assert!(
            str::contains(
                &sql,
                r#"EXISTS(SELECT 1 FROM "public"."Meta" AS "m" WHERE "m"."appId" = "App"."id""#
            ),
            "{sql}"
        );
        assert!(
            str::contains(&sql, r#""m"."lang" IN ('de', 'en')"#),
            "{sql}"
        );
        assert!(
            str::contains(&sql, r#""m"."name" ILIKE E'%in\\_voice\\%%'"#),
            "{sql}"
        );
        assert!(!str::contains(&sql, "ESCAPE"), "{sql}");
        let order = position(&sql, "ORDER BY");
        assert!(
            sql[order..].starts_with(r#"ORDER BY (SELECT "m"."name" FROM "public"."Meta" AS "m""#),
            "{sql}"
        );
        assert!(
            str::contains(
                &sql,
                r#"ORDER BY "m"."lang" = 'de' DESC LIMIT 1) ASC NULLS LAST, "App"."id" ASC LIMIT 24 OFFSET 24"#
            ),
            "{sql}"
        );
        assert!(!str::contains(&sql, "relevanceScore"));
    }

    #[test]
    fn app_sorts_break_ties_by_id_and_put_missing_ratings_last() {
        let filter = AppFilter::new("en");
        let rating = sql(app_ids(&filter, ExploreSort::Rating));
        assert!(
            str::contains(
                &rating,
                r#"ORDER BY "App"."avgRating" DESC NULLS LAST, "App"."ratingCount" DESC, "App"."id" ASC"#
            ),
            "{rating}"
        );
        let best = sql(app_ids(&filter, ExploreSort::Best));
        assert!(
            str::contains(
                &best,
                r#"ORDER BY "App"."downloadCount" DESC, "App"."ratingSum" DESC, "App"."id" ASC"#
            ),
            "{best}"
        );
        let paid = sql(top_paid_apps(Utc::now().date_naive()));
        assert!(
            str::contains(&paid, r#"COALESCE(SUM("sales"."purchaseCount"), 0)"#),
            "{paid}"
        );
        assert!(
            paid.ends_with(r#"DESC, "App"."ratingSum" DESC, "App"."id" ASC"#),
            "{paid}"
        );
    }

    #[test]
    fn an_empty_category_list_matches_nothing() {
        let filter = AppFilter {
            categories: Some(Vec::new()),
            ..AppFilter::new("en")
        };
        assert!(str::contains(
            &sql(app_ids(&filter, ExploreSort::Best)),
            "FALSE"
        ));
    }

    #[test]
    fn count_filters_render_as_aggregate_filters() {
        let sql = sql(wasm_package::Entity::find()
            .select_only()
            .expr(count_where(wasm_package::Column::Verified.eq(true)))
            .filter(PackageFilter::default().condition()));
        assert!(
            sql.starts_with(
                r#"SELECT COUNT(*) FILTER (WHERE "WasmPackage"."verified" = TRUE) FROM"#
            ),
            "{sql}"
        );
    }

    #[test]
    fn category_pairs_name_each_category_once() {
        assert_eq!(
            app_pair_names(Some(&Category::Finance), Some(&Category::Finance)),
            BTreeSet::from(["Finance".to_owned()])
        );
        assert_eq!(
            package_pair_app_names(
                Some(&DbPackageCategory::FinanceBilling),
                Some(&DbPackageCategory::Insurance)
            ),
            BTreeSet::from(["Finance"])
        );
        assert_eq!(
            package_pair_names(Some(&DbPackageCategory::AnalyticsReporting), None),
            BTreeSet::from(["ANALYTICS_REPORTING".to_owned()])
        );
    }

    #[test]
    fn rules_that_no_longer_parse_select_nothing() {
        let rule = CollectionRule {
            item_kind: ItemKind::App,
            app_category: Some("Nope".into()),
            app_type: None,
            package_category: None,
            verified_only: false,
            min_rating: None,
            price: None,
            sort: RuleSort::Installs,
            limit: 4,
        };
        assert!(app_rule_filter(&rule, "en").is_none());
        let rule = CollectionRule {
            app_category: Some("Finance".into()),
            ..rule
        };
        assert_eq!(
            app_rule_filter(&rule, "en").unwrap().categories,
            Some(vec![Category::Finance])
        );
        let rule = CollectionRule {
            item_kind: ItemKind::Package,
            package_category: Some("Finance".into()),
            ..rule
        };
        assert!(package_rule_filter(&rule).is_none());
    }
    mod database {
        use super::super::test_database::{Fixture, execute};
        use super::*;
        use flow_like_types::tokio;

        #[tokio::test]
        #[ignore = "requires a disposable PostgreSQL database"]
        async fn an_app_with_several_meta_rows_takes_one_slot() {
            let fixture = Fixture::new().await;
            let db = &fixture.db;
            execute(
                db,
                r#"INSERT INTO "App" (id, visibility, status, "primaryCategory", "secondaryCategory", price, "downloadCount") VALUES
                    ('invoicer', 'PUBLIC', 'ACTIVE', 'FINANCE', NULL, 0, 10),
                    ('ledger', 'PUBLIC_REQUEST_ACCESS', 'ACTIVE', 'BUSINESS', 'FINANCE', 500, 5),
                    ('hidden', 'PRIVATE', 'ACTIVE', 'FINANCE', NULL, 0, 99),
                    ('retired', 'PUBLIC', 'ARCHIVED', 'FINANCE', NULL, 0, 98)"#,
            )
            .await;
            execute(
                db,
                r#"INSERT INTO "Meta" (id, lang, name, description, "appId") VALUES
                    ('m1', 'en', 'Invoice Pro', 'Send invoices', 'invoicer'),
                    ('m2', 'de', 'Rechnung Pro', 'Invoice auf Deutsch', 'invoicer'),
                    ('m3', 'en', 'Ledger', 'Books with invoice export', 'ledger'),
                    ('m4', 'fr', 'Facture', 'Livre de comptes', 'ledger'),
                    ('m5', 'en', 'Invoice Pro Secret', NULL, 'hidden'),
                    ('m6', 'en', 'Invoice Pro Retired', NULL, 'retired')"#,
            )
            .await;

            let pro = AppFilter {
                text: Some("pro".into()),
                ..AppFilter::new("de")
            };
            assert_eq!(
                ids(db, app_ids(&pro, ExploreSort::Best), 0, 24)
                    .await
                    .unwrap(),
                ["invoicer"]
            );
            assert_eq!(count_apps(db, &pro).await.unwrap(), 1);

            let invoice = AppFilter {
                text: Some("invoice".into()),
                ..AppFilter::new("de")
            };
            assert_eq!(count_apps(db, &invoice).await.unwrap(), 2);
            for sort in [
                ExploreSort::Best,
                ExploreSort::Newest,
                ExploreSort::Rating,
                ExploreSort::Installs,
                ExploreSort::Name,
                ExploreSort::Updated,
            ] {
                let mut paged = ids(db, app_ids(&invoice, sort), 0, 1).await.unwrap();
                paged.extend(ids(db, app_ids(&invoice, sort), 1, 1).await.unwrap());
                paged.sort();
                assert_eq!(paged, ["invoicer", "ledger"], "{sort:?}");
            }
            assert_eq!(
                ids(db, app_ids(&invoice, ExploreSort::Name), 0, 24)
                    .await
                    .unwrap(),
                ["ledger", "invoicer"]
            );
            let english = AppFilter {
                language: "en".into(),
                ..invoice.clone()
            };
            assert_eq!(
                ids(db, app_ids(&english, ExploreSort::Name), 0, 24)
                    .await
                    .unwrap(),
                ["invoicer", "ledger"]
            );
            let french = AppFilter {
                text: Some("facture".into()),
                ..AppFilter::new("de")
            };
            assert_eq!(count_apps(db, &french).await.unwrap(), 0);
            let wildcard = AppFilter {
                text: Some("%".into()),
                ..AppFilter::new("en")
            };
            assert_eq!(count_apps(db, &wildcard).await.unwrap(), 0);

            let everything = AppFilter::new("en");
            let mut pairs = app_category_pairs(db, &everything).await.unwrap();
            pairs.sort_by_key(|(_, _, count)| *count);
            assert_eq!(pairs.len(), 2);
            assert!(pairs.contains(&(Some(Category::Finance), None, 1)));
            assert!(pairs.contains(&(Some(Category::Business), Some(Category::Finance), 1)));
            assert_eq!(app_price_counts(db, &everything).await.unwrap(), (1, 1));
            assert_eq!(app_price_split(db, &everything).await.unwrap(), (2, 1, 1));
            let finance = AppFilter {
                categories: Some(vec![Category::Finance]),
                ..AppFilter::new("en")
            };
            assert_eq!(count_apps(db, &finance).await.unwrap(), 2);
            assert_eq!(
                top_app_categories(db, 4).await.unwrap(),
                [(Category::Finance, 1), (Category::Business, 1)]
            );

            fixture.drop_database().await;
        }
    }
}
