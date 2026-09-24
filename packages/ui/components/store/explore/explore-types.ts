import type { IApp, IAppCategory, IAppType } from "../../../lib/schema/app/app";
import type { IMetadata } from "../../../lib/schema/bit/bit-pack";
import type {
	PackageSummary,
	WasmPackageCategory,
} from "../../../lib/schema/wasm";

export type ExplorePlatform = "desktop" | "web";
export type ExplorePlacementKind =
	| "announcement"
	| "spotlight"
	| "feature"
	| "collection"
	| "rail"
	| "sponsored";
export type ExploreRailKey =
	| "trending"
	| "new"
	| "new_count"
	| "top_paid"
	| "for_builders"
	| "by_category"
	| "suites";
export type ExploreGridSlotKey =
	| "hero"
	| "notice"
	| "feature"
	| "collection"
	| "stat"
	| "categories";
export type ExploreItemKind = "app" | "package" | "collection";
export type ExploreTone = "info" | "launch" | "maintenance" | "warning";
export type ExploreStatus = "draft" | "scheduled" | "live" | "ended";
export type ExploreAccent = "auto" | `category:${IAppCategory}`;
export type ExploreAudienceTag =
	| "everyone"
	| "dev"
	| "signed_in"
	| "signed_out"
	| "desktop"
	| "web"
	| `locale:${string}`;
export type ExploreSlotKey =
	| "hero"
	| "notice"
	| "feature"
	| "collection"
	| "stat"
	| "categories"
	| "unplaced"
	| `row:${string}`;
export type ExploreTypeFilter = "all" | "apps" | "packages";

export type ExplorePlacementContent =
	| {
			kind: "announcement";
			tone: ExploreTone;
			title: string;
			body: string;
			ctaLabel?: string | null;
			ctaHref?: string | null;
			imageUrl?: string | null;
			dismissible: boolean;
	  }
	| { kind: "spotlight"; rotationSeconds: number; autoFill?: boolean }
	| { kind: "feature"; eyebrow?: string | null }
	| {
			kind: "collection";
			title: string;
			blurb?: string | null;
			source: "hand" | "rule";
			rule?: ExploreCollectionRule | null;
	  }
	| { kind: "rail"; rail: ExploreRailKey; title?: string | null }
	| { kind: "sponsored"; advertiser: string };

export interface ExploreCollectionRule {
	itemKind: "app" | "package";
	appCategory?: IAppCategory | null;
	appType?: IAppType | null;
	packageCategory?: WasmPackageCategory | null;
	verifiedOnly: boolean;
	minRating?: number | null;
	price?: "free" | "paid" | null;
	sort: "installs" | "rating" | "newest";
	limit: number;
}

export interface ExplorePlacementItem {
	kind: ExploreItemKind;
	id: string;
	headline?: string | null;
	subline?: string | null;
	artworkUrl?: string | null;
	accent?: ExploreAccent | null;
}

export interface ExplorePlacement {
	id: string;
	kind: ExplorePlacementKind;
	name: string;
	enabled: boolean;
	startsAt?: string | null;
	endsAt?: string | null;
	audience: ExploreAudienceTag[];
	content: ExplorePlacementContent;
	items: ExplorePlacementItem[];
	status?: ExploreStatus;
	updatedAt?: string;
}

export interface ExploreSlot {
	key: ExploreSlotKey;
	area: "grid" | "row" | "unplaced";
	position: number;
	placements: ExplorePlacement[];
}

export interface ExploreLayoutDoc {
	slots: ExploreSlot[];
}

/** No `kind`: the server derives it from `content.kind` and rejects a change on PUT. Items must carry only the
 * fields of ExplorePlacementItem (the server denies unknown fields). */
export type ExplorePlacementInput = Omit<
	ExplorePlacement,
	"id" | "kind" | "status" | "updatedAt"
>;

export type ExploreResolvedItem =
	| { kind: "app"; app: IApp; metadata: IMetadata | null }
	| { kind: "package"; package: PackageSummary }
	| { kind: "collection"; collection: ExploreCollectionSummary };

export interface ExploreCollectionSummary {
	id: string;
	title: string;
	blurb?: string | null;
	apps: number;
	packages: number;
	preview: ExploreResolvedItem[];
}

export interface ExploreSlide {
	item: ExploreResolvedItem;
	headline?: string | null;
	subline?: string | null;
	artworkUrl?: string | null;
	accent?: ExploreAccent | null;
}

export interface ExploreSpotlight {
	placementId: string;
	rotationSeconds: number;
	slides: ExploreSlide[];
}

export interface ExploreAnnouncement {
	placementId: string;
	dismissKey: string;
	tone: ExploreTone;
	title: string;
	body: string;
	ctaLabel?: string | null;
	ctaHref?: string | null;
	imageUrl?: string | null;
	dismissible: boolean;
}

export interface ExploreFeature {
	placementId: string;
	eyebrow?: string | null;
	slide: ExploreSlide;
}

export interface ExploreCollection {
	placementId: string;
	title: string;
	blurb?: string | null;
	source: "hand" | "rule";
	items: ExploreResolvedItem[];
	apps: number;
	packages: number;
}

export interface ExploreRailStat {
	total: number;
	apps: number;
	packages: number;
	freeApps: number;
	paidApps: number;
	verifiedPackages: number;
}

export interface ExploreRailCategory {
	appCategory: IAppCategory;
	apps: number;
	packages: number;
}

export interface ExploreRail {
	placementId: string;
	rail: ExploreRailKey;
	title?: string | null;
	items: ExploreResolvedItem[];
	stat?: ExploreRailStat | null;
	categories?: ExploreRailCategory[] | null;
}

export type ExploreRow =
	| { kind: "rail"; rail: ExploreRail }
	| { kind: "collection"; collection: ExploreCollection };

export interface ExploreViewer {
	dev: boolean;
	signedIn: boolean;
	platform: ExplorePlatform;
	language: string;
}

export interface ExploreGrid {
	hero?: ExploreSpotlight | null;
	notice?: ExploreAnnouncement | null;
	feature?: ExploreFeature | null;
	collection?: ExploreCollection | null;
	stat?: ExploreRail | null;
	categories?: ExploreRail | null;
}

export interface ExploreView {
	grid: ExploreGrid;
	rows: ExploreRow[];
}

export interface ResolvedExplore {
	revision: string;
	generatedAt: string;
	viewer: ExploreViewer;
	typeCounts: { apps: number; packages: number };
	views: {
		all: ExploreView;
		apps?: ExploreView | null;
		packages?: ExploreView | null;
	};
}

/** Platform is added by the state implementation. */
export interface ExploreQuery {
	language: string;
	dev: boolean;
}

export type ExploreSearchSort =
	| "best"
	| "newest"
	| "rating"
	| "installs"
	| "name"
	| "updated";
export type ExploreSearchType = ExploreTypeFilter | "collections";
export type ExplorePermissionFacet = "none" | "network" | "models" | "storage";
export type ExploreCategoryFacet =
	| `app:${IAppCategory}`
	| `package:${WasmPackageCategory}`;

/** `categories` and `permissions` go on the wire as ONE comma-joined param each
 * (`categories=app:Finance,package:EDUCATION`, `permissions=network,models`); the other fields as snake_case params. */
export interface ExploreSearchQuery extends ExploreQuery {
	q?: string;
	type?: ExploreSearchType;
	categories?: string[];
	price?: "free" | "paid";
	verified?: boolean;
	permissions?: ExplorePermissionFacet[];
	sort?: ExploreSearchSort;
	collection?: string;
	appsOffset?: number;
	appsLimit?: number;
	packagesOffset?: number;
	packagesLimit?: number;
}

/** Thrown by getExplore/searchExplore when the hub has no Explore endpoints (a 404 without an API `code`, i.e. a
 * route miss). Pages then render the legacy lists. */
export class ExploreUnsupportedError extends Error {
	constructor() {
		super("This hub does not support the Explore storefront yet");
		this.name = "ExploreUnsupportedError";
	}
}

export interface ExploreSearchGroup<T> {
	items: T[];
	total: number;
	hasMore: boolean;
}

export interface ExploreSearchPackageHit {
	package: PackageSummary;
	matchedVia: "name" | "collection";
	collectionTitle?: string | null;
}

export interface ExploreCategoryFacetCount {
	value: ExploreCategoryFacet;
	kind: "app" | "package";
	count: number;
}

export interface ExploreSearchFacets {
	types: { apps: number; packages: number; collections: number };
	categories: ExploreCategoryFacetCount[];
	price: { free: number; paid: number };
	verified: number;
	permissions: Record<ExplorePermissionFacet, number>;
	packagesCapped: boolean;
}

export interface ExploreSearchResponse {
	query: string;
	viewer: ExploreViewer;
	collections: ExploreCollection[];
	apps: ExploreSearchGroup<Extract<ExploreResolvedItem, { kind: "app" }>>;
	packages: ExploreSearchGroup<ExploreSearchPackageHit>;
	related: ExploreResolvedItem[];
	facets: ExploreSearchFacets;
}

export interface ExploreItemRef {
	kind: ExploreItemKind;
	id: string;
	name: string;
	iconUrl?: string | null;
	coverUrl?: string | null;
	public: boolean;
	exists: boolean;
}

export interface ExploreChange {
	placementId: string;
	name: string;
	change: "added" | "removed" | "modified" | "moved";
}

export interface ExploreEditorState {
	draftRevision: string;
	liveRevision: string | null;
	publishedAt: string | null;
	now: string;
	layout: ExploreLayoutDoc;
	refs: ExploreItemRef[];
	changes: ExploreChange[];
	warnings: string[];
}

export type ExploreSkipReason =
	| "draft"
	| "scheduled"
	| "ended"
	| "audience"
	| "dev_only"
	| "too_few_items"
	| "empty";

export interface ExploreSlotTrace {
	slotKey: ExploreSlotKey;
	chosen: string | null;
	skipped: { placementId: string; reason: ExploreSkipReason }[];
}

export interface ExplorePreview {
	page: ResolvedExplore;
	trace: ExploreSlotTrace[];
}

export const EXPLORE_LIMITS = {
	name: 80,
	announcementTitle: 60,
	announcementBody: 140,
	ctaLabel: 24,
	headline: 60,
	subline: 140,
	collectionTitle: 60,
	blurb: 140,
	railTitle: 40,
	advertiser: 60,
	eyebrow: 40,
	spotlightItems: 6,
	collectionItems: 12,
	rulePinned: 4,
	placements: 60,
	rows: 12,
	rotationMin: 5,
	rotationMax: 20,
} as const;

/** GET /store/explore/search rejects anything past these with a 400; clients cap before sending. */
export const EXPLORE_SEARCH_LIMITS = {
	q: 100,
	categories: 16,
	permissions: 4,
	pageSize: 48,
} as const;

const WASM_PACKAGE_CATEGORY_SET = {
	DOCUMENT_PROCESSING: true,
	DATA_TRANSFORMATION: true,
	WORKFLOW_AUTOMATION: true,
	COMMUNICATION: true,
	ANALYTICS_REPORTING: true,
	FINANCE_BILLING: true,
	COMPLIANCE_REGULATORY: true,
	HR_PEOPLE: true,
	AI_ML: true,
	INTEGRATION_CONNECTORS: true,
	SECURITY_IDENTITY: true,
	DEVOPS: true,
	IOT_INDUSTRIAL: true,
	ROBOTICS_PHYSICAL_AI: true,
	GAMING_SIMULATION: true,
	HEALTHCARE: true,
	VETERINARY: true,
	LEGAL: true,
	MANUFACTURING: true,
	AGRICULTURE: true,
	REAL_ESTATE: true,
	LOGISTICS: true,
	ENERGY: true,
	CONSTRUCTION_TRADES: true,
	EDUCATION: true,
	GOVERNMENT_DEFENSE: true,
	ECOMMERCE: true,
	INSURANCE: true,
	TELECOM: true,
	SCIENTIFIC_ENGINEERING: true,
	GEOSPATIAL: true,
	MEDIA_CONTENT: true,
	OTHER: true,
} as const satisfies Record<WasmPackageCategory, true>;

export const WASM_PACKAGE_CATEGORIES = Object.keys(
	WASM_PACKAGE_CATEGORY_SET,
) as readonly WasmPackageCategory[];

/** Mirrors the Rust `package_categories_for` / `app_categories_for` pair. */
export const APP_TO_PACKAGE_CATEGORIES: Readonly<
	Record<IAppCategory, readonly WasmPackageCategory[]>
> = {
	Anime: [],
	Business: ["ANALYTICS_REPORTING", "INTEGRATION_CONNECTORS"],
	Communication: ["COMMUNICATION"],
	Education: ["EDUCATION"],
	Entertainment: ["MEDIA_CONTENT"],
	Finance: ["FINANCE_BILLING", "INSURANCE"],
	FoodAndDrink: [],
	Games: ["GAMING_SIMULATION"],
	Health: ["HEALTHCARE"],
	Lifestyle: [],
	Music: ["MEDIA_CONTENT"],
	News: ["MEDIA_CONTENT"],
	Other: [],
	Photography: ["MEDIA_CONTENT"],
	Productivity: ["WORKFLOW_AUTOMATION", "DOCUMENT_PROCESSING"],
	Shopping: ["ECOMMERCE"],
	Social: [],
	Sports: [],
	Travel: [],
	Utilities: ["DATA_TRANSFORMATION", "DEVOPS"],
	Weather: [],
};

export const APP_CATEGORIES = Object.keys(
	APP_TO_PACKAGE_CATEGORIES,
) as readonly IAppCategory[];

export interface BentoSlotGeometry {
	key: ExploreGridSlotKey;
	col: number;
	span: number;
	row: number;
	rowSpan: number;
	kind: ExplorePlacementKind;
	rail?: ExploreRailKey;
}

/** The full xl layout (§2.3): every grid slot present, 12 columns. */
export const BENTO_SLOTS: readonly BentoSlotGeometry[] = [
	{ key: "hero", col: 1, span: 7, row: 1, rowSpan: 2, kind: "spotlight" },
	{ key: "notice", col: 8, span: 5, row: 1, rowSpan: 1, kind: "announcement" },
	{ key: "feature", col: 8, span: 5, row: 2, rowSpan: 1, kind: "feature" },
	{
		key: "collection",
		col: 1,
		span: 6,
		row: 3,
		rowSpan: 2,
		kind: "collection",
	},
	{
		key: "stat",
		col: 7,
		span: 2,
		row: 3,
		rowSpan: 2,
		kind: "rail",
		rail: "new_count",
	},
	{
		key: "categories",
		col: 9,
		span: 4,
		row: 3,
		rowSpan: 2,
		kind: "rail",
		rail: "by_category",
	},
];

/** xl row heights in px for the full layout: hero/notice, hero/feature, lower band ×2. */
export const BENTO_ROW_HEIGHTS = [176, 204, 92, 92] as const;

export const GRID_SLOT_KEYS: readonly ExploreGridSlotKey[] = BENTO_SLOTS.map(
	(slot) => slot.key,
);

/** Placement kinds a `row:<id>` slot accepts. */
export const ROW_KINDS: readonly ExplorePlacementKind[] = [
	"rail",
	"collection",
];

/** Rails a `row:<id>` slot accepts. */
export const ROW_RAILS: readonly ExploreRailKey[] = [
	"trending",
	"new",
	"top_paid",
	"for_builders",
	"suites",
];
