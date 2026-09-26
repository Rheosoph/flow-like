import { categoryColor } from "../../../lib/category-meta";
import { asArray, isRecord } from "../../../lib/response-shape";
import type { IApp, IAppCategory } from "../../../lib/schema/app/app";
import type { IMetadata } from "../../../lib/schema/bit/bit-pack";
import type {
	MetaSummary,
	PackageSummary,
	WasmPackageCategory,
} from "../../../lib/schema/wasm";
import { stabilizeMetadata } from "../../../lib/stable-asset-url";
import { browseParamEntries, isExploreCategoryFacet } from "./explore-href";
import {
	APP_CATEGORIES,
	APP_TO_PACKAGE_CATEGORIES,
	BENTO_ROW_HEIGHTS,
	BENTO_SLOTS,
	EXPLORE_LIMITS,
	EXPLORE_SEARCH_LIMITS,
	type ExploreAccent,
	type ExploreAnnouncement,
	type ExploreCollection,
	type ExploreCollectionSummary,
	type ExploreFeature,
	type ExploreGrid,
	type ExploreGridSlotKey,
	type ExplorePlacement,
	type ExplorePlatform,
	type ExploreQuery,
	type ExploreRail,
	type ExploreRailCategory,
	type ExploreRailKey,
	type ExploreRailStat,
	type ExploreResolvedItem,
	type ExploreRow,
	type ExploreSearchFacets,
	type ExploreSearchPackageHit,
	type ExploreSearchQuery,
	type ExploreSearchResponse,
	type ExploreSlide,
	type ExploreSpotlight,
	type ExploreStatus,
	type ExploreTone,
	type ExploreTypeFilter,
	ExploreUnsupportedError,
	type ExploreView,
	type ExploreViewer,
	GRID_SLOT_KEYS,
	type ResolvedExplore,
} from "./explore-types";

type AppItem = Extract<ExploreResolvedItem, { kind: "app" }>;
type PackageItem = Extract<ExploreResolvedItem, { kind: "package" }>;

export function isExploreUnsupportedError(
	error: unknown,
): error is ExploreUnsupportedError {
	return (
		error instanceof ExploreUnsupportedError ||
		(isRecord(error) && error.name === "ExploreUnsupportedError")
	);
}

/** A 404 without an API `code` is a route miss (hub without Explore); a coded 404 is a real "not found". */
export function toExploreError(error: unknown): unknown {
	if (!isRecord(error) || error.status !== 404) return error;
	const code = error.code;
	if (typeof code === "string" && code.trim()) return error;
	return new ExploreUnsupportedError();
}

export function placementStatus(
	placement: Pick<ExplorePlacement, "enabled" | "startsAt" | "endsAt">,
	now: Date | number | string = Date.now(),
): ExploreStatus {
	if (!placement.enabled) return "draft";
	const at = new Date(now).getTime();
	if (placement.startsAt && at < Date.parse(placement.startsAt)) {
		return "scheduled";
	}
	if (placement.endsAt && Date.parse(placement.endsAt) <= at) return "ended";
	return "live";
}

/** Tags AND across dimensions and OR within one; unknown tags never match. */
export function audienceMatches(
	audience: readonly string[],
	viewer: Pick<ExploreViewer, "dev" | "signedIn" | "platform" | "language">,
): boolean {
	const tags = audience.filter((tag) => tag !== "everyone");
	const platforms: string[] = [];
	const locales: string[] = [];
	let signedIn = false;
	let signedOut = false;
	for (const tag of tags) {
		if (tag === "dev") {
			if (!viewer.dev) return false;
		} else if (tag === "signed_in") signedIn = true;
		else if (tag === "signed_out") signedOut = true;
		else if (tag === "desktop" || tag === "web") platforms.push(tag);
		else if (tag.startsWith("locale:") && tag.length > "locale:".length) {
			locales.push(tag.slice("locale:".length).toLowerCase());
		} else return false;
	}
	if (signedIn && signedOut) return false;
	if (signedIn && !viewer.signedIn) return false;
	if (signedOut && viewer.signedIn) return false;
	if (platforms.length && !platforms.includes(viewer.platform)) return false;
	if (locales.length) {
		const language = viewer.language.toLowerCase();
		return locales.some(
			(code) => language === code || language.startsWith(`${code}-`),
		);
	}
	return true;
}

/** How an unexpected response body reads in an error message. */
export function describeResponseShape(value: unknown): string {
	if (value === null) return "null";
	if (Array.isArray(value)) return "an array";
	if (typeof value === "string") {
		const snippet = value.trim().slice(0, 40);
		return snippet ? `text "${snippet}"` : "an empty body";
	}
	return typeof value;
}

function text(value: unknown, fallback = ""): string {
	return typeof value === "string" ? value : fallback;
}

function optionalText(value: unknown): string | null {
	return typeof value === "string" ? value : null;
}

function count(value: unknown): number {
	return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

export function recordsOf(value: unknown): Record<string, unknown>[] {
	return asArray(value as unknown[] | undefined).filter(isRecord);
}

const URL_MAX = 2048;
const ROTATION_DEFAULT = 8;

function fitsUrlLimit(value: string): boolean {
	return Array.from(value).length <= URL_MAX;
}

function hasControlCharacter(value: string): boolean {
	for (const char of value) {
		const code = char.codePointAt(0) ?? 0;
		if (code < 0x20 || (code >= 0x7f && code <= 0x9f)) return true;
	}
	return false;
}

/** The hub's write-side rule (§3.2): `https://` with a host and no credentials, at most 2048 code points. */
export function isExploreHttpsUrl(value: string): boolean {
	if (!fitsUrlLimit(value) || hasControlCharacter(value)) return false;
	try {
		const url = new URL(value);
		return (
			url.protocol === "https:" &&
			url.hostname !== "" &&
			url.username === "" &&
			url.password === ""
		);
	} catch {
		return false;
	}
}

/** An app path (`/…`, never `//` or `/\`, no whitespace) or an https URL, as the hub validates on write (§3.2). */
export function isExploreCtaHref(value: string): boolean {
	const appPath =
		value.startsWith("/") &&
		!value.startsWith("//") &&
		!value.startsWith("/\\") &&
		!/\s/.test(value) &&
		!hasControlCharacter(value);
	return appPath ? fitsUrlLimit(value) : isExploreHttpsUrl(value);
}

function httpsUrl(value: unknown): string | null {
	return typeof value === "string" && isExploreHttpsUrl(value) ? value : null;
}

function ctaHref(value: unknown): string | null {
	return typeof value === "string" && isExploreCtaHref(value) ? value : null;
}

function rotationSeconds(value: unknown): number {
	const seconds = count(value);
	if (seconds <= 0) return ROTATION_DEFAULT;
	return Math.min(
		EXPLORE_LIMITS.rotationMax,
		Math.max(EXPLORE_LIMITS.rotationMin, Math.round(seconds)),
	);
}

function present<T>(value: T | null): value is T {
	return value !== null;
}

const RAIL_KEYS: ReadonlySet<string> = new Set<ExploreRailKey>([
	"trending",
	"new",
	"new_count",
	"top_paid",
	"for_builders",
	"by_category",
	"suites",
]);
const TONES: ReadonlySet<string> = new Set<ExploreTone>([
	"info",
	"launch",
	"maintenance",
	"warning",
]);
const APP_CATEGORY_SET: ReadonlySet<string> = new Set(APP_CATEGORIES);

function parseAccent(value: unknown): ExploreAccent | null {
	if (value === "auto") return "auto";
	if (typeof value !== "string" || !value.startsWith("category:")) return null;
	return APP_CATEGORY_SET.has(value.slice("category:".length))
		? (value as ExploreAccent)
		: null;
}

function parsePackage(raw: Record<string, unknown>): PackageSummary {
	const summary = raw as unknown as PackageSummary;
	return {
		...summary,
		keywords: asArray(summary.keywords),
		capabilities: Array.isArray(raw.capabilities)
			? raw.capabilities.filter(
					(entry): entry is string => typeof entry === "string",
				)
			: undefined,
		metadata: isRecord(raw.metadata)
			? stabilizeMetadata(raw.metadata as unknown as MetaSummary)
			: undefined,
	};
}

function parseCollectionSummary(
	raw: Record<string, unknown>,
): ExploreCollectionSummary {
	return {
		id: text(raw.id),
		title: text(raw.title),
		blurb: optionalText(raw.blurb),
		apps: count(raw.apps),
		packages: count(raw.packages),
		preview: parseItems(raw.preview),
	};
}

function parseItem(raw: Record<string, unknown>): ExploreResolvedItem | null {
	if (raw.kind === "app") {
		const app = raw.app;
		if (!isRecord(app) || typeof app.id !== "string") return null;
		return {
			kind: "app",
			app: app as unknown as IApp,
			metadata: isRecord(raw.metadata)
				? stabilizeMetadata(raw.metadata as unknown as IMetadata)
				: null,
		};
	}
	if (raw.kind === "package") {
		const pkg = raw.package;
		if (!isRecord(pkg) || typeof pkg.id !== "string") return null;
		return { kind: "package", package: parsePackage(pkg) };
	}
	if (raw.kind === "collection") {
		const collection = raw.collection;
		if (!isRecord(collection) || typeof collection.id !== "string") return null;
		return {
			kind: "collection",
			collection: parseCollectionSummary(collection),
		};
	}
	return null;
}

function parseItems(value: unknown): ExploreResolvedItem[] {
	return recordsOf(value).map(parseItem).filter(present);
}

function parseSlide(value: unknown): ExploreSlide | null {
	if (!isRecord(value) || !isRecord(value.item)) return null;
	const item = parseItem(value.item);
	if (!item) return null;
	return {
		item,
		headline: optionalText(value.headline),
		subline: optionalText(value.subline),
		artworkUrl: httpsUrl(value.artworkUrl),
		accent: parseAccent(value.accent),
	};
}

function parseSpotlight(value: unknown): ExploreSpotlight | null {
	if (!isRecord(value)) return null;
	const slides = asArray(value.slides as unknown[] | undefined)
		.map(parseSlide)
		.filter(present);
	if (!slides.length) return null;
	return {
		placementId: text(value.placementId),
		rotationSeconds: rotationSeconds(value.rotationSeconds),
		slides,
	};
}

function parseAnnouncement(value: unknown): ExploreAnnouncement | null {
	if (!isRecord(value)) return null;
	const href = ctaHref(value.ctaHref);
	return {
		placementId: text(value.placementId),
		dismissKey: text(value.dismissKey, text(value.placementId)),
		tone: TONES.has(text(value.tone)) ? (value.tone as ExploreTone) : "info",
		title: text(value.title),
		body: text(value.body),
		ctaLabel: href ? optionalText(value.ctaLabel) : null,
		ctaHref: href,
		imageUrl: httpsUrl(value.imageUrl),
		dismissible: value.dismissible === true,
	};
}

function parseFeature(value: unknown): ExploreFeature | null {
	if (!isRecord(value)) return null;
	const slide = parseSlide(value.slide);
	if (!slide) return null;
	return {
		placementId: text(value.placementId),
		eyebrow: optionalText(value.eyebrow),
		slide,
	};
}

function parseCollection(value: unknown): ExploreCollection | null {
	if (!isRecord(value)) return null;
	return {
		placementId: text(value.placementId),
		title: text(value.title),
		blurb: optionalText(value.blurb),
		source: value.source === "rule" ? "rule" : "hand",
		items: parseItems(value.items),
		apps: count(value.apps),
		packages: count(value.packages),
	};
}

function parseRailStat(value: unknown): ExploreRailStat | null {
	if (!isRecord(value)) return null;
	return {
		total: count(value.total),
		apps: count(value.apps),
		packages: count(value.packages),
		freeApps: count(value.freeApps),
		paidApps: count(value.paidApps),
		verifiedPackages: count(value.verifiedPackages),
	};
}

function parseRailCategories(value: unknown): ExploreRailCategory[] | null {
	if (!Array.isArray(value)) return null;
	return recordsOf(value)
		.filter((entry) => APP_CATEGORY_SET.has(text(entry.appCategory)))
		.map((entry) => ({
			appCategory: entry.appCategory as IAppCategory,
			apps: count(entry.apps),
			packages: count(entry.packages),
		}));
}

function parseRail(value: unknown): ExploreRail | null {
	if (!isRecord(value) || !RAIL_KEYS.has(text(value.rail))) return null;
	return {
		placementId: text(value.placementId),
		rail: value.rail as ExploreRailKey,
		title: optionalText(value.title),
		items: parseItems(value.items),
		stat: parseRailStat(value.stat),
		categories: parseRailCategories(value.categories),
	};
}

function parseRow(raw: Record<string, unknown>): ExploreRow | null {
	if (raw.kind === "rail") {
		const rail = parseRail(raw.rail);
		return rail ? { kind: "rail", rail } : null;
	}
	if (raw.kind === "collection") {
		const collection = parseCollection(raw.collection);
		return collection ? { kind: "collection", collection } : null;
	}
	return null;
}

function parseGrid(value: unknown): ExploreGrid {
	const grid = isRecord(value) ? value : {};
	return {
		hero: parseSpotlight(grid.hero),
		notice: parseAnnouncement(grid.notice),
		feature: parseFeature(grid.feature),
		collection: parseCollection(grid.collection),
		stat: parseRail(grid.stat),
		categories: parseRail(grid.categories),
	};
}

function parseView(value: unknown): ExploreView | null {
	if (!isRecord(value)) return null;
	return {
		grid: parseGrid(value.grid),
		rows: recordsOf(value.rows).map(parseRow).filter(present),
	};
}

function parseViewer(value: unknown): ExploreViewer {
	const viewer = isRecord(value) ? value : {};
	return {
		dev: viewer.dev === true,
		signedIn: viewer.signedIn === true,
		platform: viewer.platform === "desktop" ? "desktop" : "web",
		language: text(viewer.language, "en"),
	};
}

export function parseResolvedExplore(value: unknown): ResolvedExplore {
	if (!isRecord(value)) {
		throw new Error(
			`Unexpected Explore response: expected an object, got ${describeResponseShape(value)}`,
		);
	}
	const views = isRecord(value.views) ? value.views : {};
	const all = parseView(views.all);
	if (!all) {
		throw new Error(
			`Unexpected Explore response: views.all is ${describeResponseShape(views.all)}`,
		);
	}
	const typeCounts = isRecord(value.typeCounts) ? value.typeCounts : {};
	return {
		revision: text(value.revision),
		generatedAt: text(value.generatedAt),
		viewer: parseViewer(value.viewer),
		typeCounts: {
			apps: count(typeCounts.apps),
			packages: count(typeCounts.packages),
		},
		views: {
			all,
			apps: parseView(views.apps),
			packages: parseView(views.packages),
		},
	};
}

function parseGroup<T>(
	value: unknown,
	parse: (raw: Record<string, unknown>) => T | null,
) {
	const group = isRecord(value) ? value : {};
	return {
		items: recordsOf(group.items).map(parse).filter(present),
		total: count(group.total),
		hasMore: group.hasMore === true,
	};
}

function parsePackageHit(
	raw: Record<string, unknown>,
): ExploreSearchPackageHit | null {
	if (!isRecord(raw.package) || typeof raw.package.id !== "string") {
		return null;
	}
	return {
		package: parsePackage(raw.package),
		matchedVia: raw.matchedVia === "collection" ? "collection" : "name",
		collectionTitle: optionalText(raw.collectionTitle),
	};
}

function parseFacets(value: unknown): ExploreSearchFacets {
	const facets = isRecord(value) ? value : {};
	const types = isRecord(facets.types) ? facets.types : {};
	const price = isRecord(facets.price) ? facets.price : {};
	const permissions = isRecord(facets.permissions) ? facets.permissions : {};
	return {
		types: {
			apps: count(types.apps),
			packages: count(types.packages),
			collections: count(types.collections),
		},
		categories: recordsOf(facets.categories).flatMap((entry) => {
			const facet = text(entry.value);
			if (!isExploreCategoryFacet(facet)) return [];
			return [
				{
					value: facet,
					kind: facet.startsWith("app:") ? "app" : "package",
					count: count(entry.count),
				} as const,
			];
		}),
		price: { free: count(price.free), paid: count(price.paid) },
		verified: count(facets.verified),
		permissions: {
			none: count(permissions.none),
			network: count(permissions.network),
			models: count(permissions.models),
			storage: count(permissions.storage),
		},
		packagesCapped: facets.packagesCapped === true,
	};
}

export function parseExploreSearch(value: unknown): ExploreSearchResponse {
	if (!isRecord(value)) {
		throw new Error(
			`Unexpected Explore search response: expected an object, got ${describeResponseShape(value)}`,
		);
	}
	return {
		query: text(value.query),
		viewer: parseViewer(value.viewer),
		collections: recordsOf(value.collections)
			.map(parseCollection)
			.filter(present),
		apps: parseGroup(value.apps, (raw) => {
			const item = parseItem(raw);
			return item?.kind === "app" ? item : null;
		}),
		packages: parseGroup(value.packages, parsePackageHit),
		related: parseItems(value.related),
		facets: parseFacets(value.facets),
	};
}

/** Wire path for GET /store/explore; the state implementation passes its own platform. */
export function exploreRequestPath(
	query: ExploreQuery,
	platform: ExplorePlatform,
): string {
	const params = new URLSearchParams({
		platform,
		language: query.language,
		dev: String(query.dev),
	});
	return `store/explore?${params}`;
}

function pageSize(limit: number | undefined): number | undefined {
	return limit === undefined
		? undefined
		: Math.min(limit, EXPLORE_SEARCH_LIMITS.pageSize);
}

/** Wire path for GET /store/explore/search: lists are ONE comma-joined param, the rest snake_case, all capped to
 * the server's limits so an overlong query or facet selection never turns into a 400. */
export function exploreSearchRequestPath(
	query: ExploreSearchQuery,
	platform: ExplorePlatform,
): string {
	const params = new URLSearchParams({
		platform,
		language: query.language,
		dev: String(query.dev),
	});
	for (const [key, value] of browseParamEntries(query)) params.set(key, value);
	const paging: [string, number | undefined][] = [
		["apps_offset", query.appsOffset],
		["apps_limit", pageSize(query.appsLimit)],
		["packages_offset", query.packagesOffset],
		["packages_limit", pageSize(query.packagesLimit)],
	];
	for (const [key, value] of paging) {
		if (value !== undefined) params.set(key, String(value));
	}
	return `store/explore/search?${params}`;
}

/** The server resolved each view's fallbacks; the client only picks one. */
export function pickExploreView(
	page: Pick<ResolvedExplore, "views">,
	type: ExploreTypeFilter,
): ExploreView {
	if (type === "apps") return page.views.apps ?? page.views.all;
	if (type === "packages") return page.views.packages ?? page.views.all;
	return page.views.all;
}

export type BentoTileSize = "short" | "tall";

export interface BentoArea {
	col: number;
	span: number;
	row: number;
	rowSpan: number;
}

export interface BentoTilePlacement {
	/** 12 columns. */
	xl: BentoArea;
	/** 6 columns. */
	md: BentoArea;
	/** One grid row ("short") or two ("tall") at xl. */
	size: BentoTileSize;
	mdSize: BentoTileSize;
}

export type BentoPlacement = Partial<
	Record<ExploreGridSlotKey, BentoTilePlacement>
>;

export interface BentoRows {
	/** Row heights in px for `gridTemplateRows`, top band then lower band. */
	xl: number[];
	md: number[];
}

type BandLayout = {
	rows: readonly number[];
	tiles: Partial<Record<ExploreGridSlotKey, BentoArea>>;
};

function area(
	col: number,
	span: number,
	row: number,
	rowSpan: number,
): BentoArea {
	return { col, span, row, rowSpan };
}

const TOP_BAND_ROWS = 2;
const TOP_ROWS: readonly number[] = BENTO_ROW_HEIGHTS.slice(0, TOP_BAND_ROWS);
const LOWER_ROWS: readonly number[] = BENTO_ROW_HEIGHTS.slice(TOP_BAND_ROWS);
const TOP_SLOTS: readonly ExploreGridSlotKey[] = BENTO_SLOTS.filter(
	(slot) => slot.row <= TOP_BAND_ROWS,
).map((slot) => slot.key);
const LOWER_SLOTS: readonly ExploreGridSlotKey[] = GRID_SLOT_KEYS.filter(
	(key) => !TOP_SLOTS.includes(key),
);
const MD_HEIGHTS = {
	hero: 360,
	notice: TOP_ROWS[0],
	feature: TOP_ROWS[1],
	lowerTall: 200,
	lowerShort: LOWER_ROWS[0],
};

/** A band with every slot present: the full §2.3 layout from BENTO_SLOTS, rows counted from the band's top. */
function fullBand(
	slots: readonly ExploreGridSlotKey[],
	rows: readonly number[],
	rowOffset: number,
): BandLayout {
	const tiles: BandLayout["tiles"] = {};
	for (const { key, col, span, row, rowSpan } of BENTO_SLOTS) {
		if (slots.includes(key)) {
			tiles[key] = area(col, span, row - rowOffset, rowSpan);
		}
	}
	return { rows, tiles };
}

const TOP_BAND: Readonly<Record<string, BandLayout>> = {
	[TOP_SLOTS.join(",")]: fullBand(TOP_SLOTS, TOP_ROWS, 0),
	"hero,notice": {
		rows: TOP_ROWS,
		tiles: { hero: area(1, 7, 1, 2), notice: area(8, 5, 1, 2) },
	},
	"hero,feature": {
		rows: TOP_ROWS,
		tiles: { hero: area(1, 7, 1, 2), feature: area(8, 5, 1, 2) },
	},
	hero: { rows: TOP_ROWS, tiles: { hero: area(1, 12, 1, 2) } },
	"notice,feature": {
		rows: TOP_ROWS,
		tiles: { notice: area(1, 6, 1, 2), feature: area(7, 6, 1, 2) },
	},
	notice: { rows: [TOP_ROWS[0]], tiles: { notice: area(1, 12, 1, 1) } },
	feature: { rows: [TOP_ROWS[1]], tiles: { feature: area(1, 12, 1, 1) } },
};

const LOWER_BAND: Readonly<Record<string, BandLayout>> = {
	[LOWER_SLOTS.join(",")]: fullBand(LOWER_SLOTS, LOWER_ROWS, TOP_BAND_ROWS),
	"collection,categories": {
		rows: LOWER_ROWS,
		tiles: { collection: area(1, 6, 1, 2), categories: area(7, 6, 1, 2) },
	},
	"collection,stat": {
		rows: LOWER_ROWS,
		tiles: { collection: area(1, 8, 1, 2), stat: area(9, 4, 1, 2) },
	},
	"stat,categories": {
		rows: LOWER_ROWS,
		tiles: { stat: area(1, 4, 1, 2), categories: area(5, 8, 1, 2) },
	},
	collection: {
		rows: LOWER_ROWS,
		tiles: { collection: area(1, 12, 1, 2) },
	},
	categories: {
		rows: [LOWER_ROWS[0]],
		tiles: { categories: area(1, 12, 1, 1) },
	},
	stat: { rows: [LOWER_ROWS[0]], tiles: { stat: area(1, 12, 1, 1) } },
};

function bandOf(
	table: Readonly<Record<string, BandLayout>>,
	order: readonly ExploreGridSlotKey[],
	present: ReadonlySet<ExploreGridSlotKey>,
): BandLayout {
	const key = order.filter((slot) => present.has(slot)).join(",");
	return Object.hasOwn(table, key) ? table[key] : { rows: [], tiles: {} };
}

type MdRow = { height: number; tiles: [ExploreGridSlotKey, number, number][] };

function mdRowsOf(present: ReadonlySet<ExploreGridSlotKey>): MdRow[] {
	const rows: MdRow[] = [];
	if (present.has("hero")) {
		rows.push({ height: MD_HEIGHTS.hero, tiles: [["hero", 1, 6]] });
	}
	const notice = present.has("notice");
	const feature = present.has("feature");
	if (notice && feature) {
		rows.push({
			height: MD_HEIGHTS.feature,
			tiles: [
				["notice", 1, 3],
				["feature", 4, 3],
			],
		});
	} else if (notice) {
		rows.push({ height: MD_HEIGHTS.notice, tiles: [["notice", 1, 6]] });
	} else if (feature) {
		rows.push({ height: MD_HEIGHTS.feature, tiles: [["feature", 1, 6]] });
	}
	if (present.has("collection")) {
		rows.push({
			height: MD_HEIGHTS.lowerTall,
			tiles: [["collection", 1, 6]],
		});
	}
	const stat = present.has("stat");
	const categories = present.has("categories");
	if (stat && categories) {
		rows.push({
			height: MD_HEIGHTS.lowerTall,
			tiles: [
				["stat", 1, 2],
				["categories", 3, 4],
			],
		});
	} else if (stat || categories) {
		rows.push({
			height: MD_HEIGHTS.lowerShort,
			tiles: [[stat ? "stat" : "categories", 1, 6]],
		});
	}
	return rows;
}

function mdSizeOf(
	slot: ExploreGridSlotKey,
	present: ReadonlySet<ExploreGridSlotKey>,
): BentoTileSize {
	if (slot === "hero" || slot === "collection") return "tall";
	if (slot === "stat" || slot === "categories") {
		return present.has("stat") && present.has("categories") ? "tall" : "short";
	}
	return "short";
}

/** Grid areas for the slots that resolved in the current view (§2.3 collapse tables); bands without a slot vanish. */
export function bentoPlacement(
	present: readonly ExploreGridSlotKey[],
): BentoPlacement {
	const slots = new Set(present);
	const top = bandOf(TOP_BAND, TOP_SLOTS, slots);
	const lower = bandOf(LOWER_BAND, LOWER_SLOTS, slots);
	const lowerOffset = top.rows.length;
	const md = new Map<ExploreGridSlotKey, BentoArea>();
	mdRowsOf(slots).forEach((row, index) => {
		for (const [slot, col, span] of row.tiles) {
			md.set(slot, area(col, span, index + 1, 1));
		}
	});
	const placement: BentoPlacement = {};
	const place = (slot: ExploreGridSlotKey, xl: BentoArea) => {
		const mdArea = md.get(slot);
		if (!mdArea) return;
		placement[slot] = {
			xl,
			md: mdArea,
			size: xl.rowSpan > 1 ? "tall" : "short",
			mdSize: mdSizeOf(slot, slots),
		};
	};
	for (const slot of TOP_SLOTS) {
		const xl = top.tiles[slot];
		if (xl) place(slot, xl);
	}
	for (const slot of LOWER_SLOTS) {
		const xl = lower.tiles[slot];
		if (xl) place(slot, { ...xl, row: xl.row + lowerOffset });
	}
	return placement;
}

export function bentoRows(present: readonly ExploreGridSlotKey[]): BentoRows {
	const slots = new Set(present);
	return {
		xl: [
			...bandOf(TOP_BAND, TOP_SLOTS, slots).rows,
			...bandOf(LOWER_BAND, LOWER_SLOTS, slots).rows,
		],
		md: mdRowsOf(slots).map((row) => row.height),
	};
}

function isAppItem(item: ExploreResolvedItem): item is AppItem {
	return item.kind === "app";
}

function isPackageItem(item: ExploreResolvedItem): item is PackageItem {
	return item.kind === "package";
}

export interface ExploreRowCell extends BentoArea {
	item: ExploreResolvedItem;
}

/** Trending row on 12 columns × 2 rows: app | package pair | app (dev), four apps, or two package pairs. Package
 * cells widen into the space a missing app or partner would leave empty. */
export function layoutTrending(
	items: readonly ExploreResolvedItem[],
	dev: boolean,
): ExploreRowCell[] {
	const apps = items.filter(isAppItem);
	const packages = dev ? items.filter(isPackageItem) : [];
	if (apps.length && packages.length >= 2) {
		const trailing = apps[1];
		const pairSpan = trailing ? 6 : 9;
		const cells: ExploreRowCell[] = [
			{ item: apps[0], ...area(1, 3, 1, 2) },
			{ item: packages[0], ...area(4, pairSpan, 1, 1) },
			{ item: packages[1], ...area(4, pairSpan, 2, 1) },
		];
		if (trailing) cells.push({ item: trailing, ...area(10, 3, 1, 2) });
		return cells;
	}
	if (apps.length) {
		return apps
			.slice(0, 4)
			.map((item, index) => ({ item, ...area(1 + index * 3, 3, 1, 2) }));
	}
	const shown = packages.slice(0, 4);
	return shown.map((item, index) => {
		const partnerless = index % 2 === 0 && index === shown.length - 1;
		return {
			item,
			...area(
				index % 2 ? 7 : 1,
				partnerless ? 12 : 6,
				Math.floor(index / 2) + 1,
				1,
			),
		};
	});
}

/** Top paid: two apps and two packages (topped up from the other kind), or four apps without packages. */
export function layoutTopPaid(
	items: readonly ExploreResolvedItem[],
	dev: boolean,
): ExploreResolvedItem[] {
	const apps = items.filter(isAppItem);
	const packages = dev ? items.filter(isPackageItem) : [];
	const appCount = Math.min(apps.length, Math.max(2, 4 - packages.length));
	const packageCount = Math.min(packages.length, 4 - appCount);
	return [...apps.slice(0, appCount), ...packages.slice(0, packageCount)];
}

const DISMISSED_STORAGE_KEY = "flow-like.explore.dismissed-announcements";
const MAX_DISMISSED = 50;

export interface DismissStorage {
	read(): string[];
	isDismissed(key: string): boolean;
	dismiss(key: string): void;
	restore(key: string): void;
}

/** Dismissed announcement keys in localStorage; every access tolerates a missing or throwing storage. */
export function createDismissStorage(
	storage: () => Storage | null | undefined = () => globalThis.localStorage,
): DismissStorage {
	const read = (): string[] => {
		try {
			const raw = storage()?.getItem(DISMISSED_STORAGE_KEY);
			if (!raw) return [];
			const parsed: unknown = JSON.parse(raw);
			return asArray(parsed as unknown[]).filter(
				(entry): entry is string => typeof entry === "string",
			);
		} catch {
			return [];
		}
	};
	const write = (keys: string[]) => {
		try {
			storage()?.setItem(
				DISMISSED_STORAGE_KEY,
				JSON.stringify(keys.slice(-MAX_DISMISSED)),
			);
		} catch {}
	};
	return {
		read,
		isDismissed: (key) => read().includes(key),
		dismiss: (key) => write([...read().filter((entry) => entry !== key), key]),
		restore: (key) => write(read().filter((entry) => entry !== key)),
	};
}

export const dismissStorage = createDismissStorage();

export function appCategoryForPackage(
	category: WasmPackageCategory | null | undefined,
): IAppCategory | undefined {
	if (!category) return undefined;
	return APP_CATEGORIES.find((app) =>
		APP_TO_PACKAGE_CATEGORIES[app].includes(category),
	);
}

function itemAppCategory(item: ExploreResolvedItem): string | undefined {
	if (item.kind === "app") {
		return (
			item.app.primary_category ?? item.app.secondary_category ?? undefined
		);
	}
	if (item.kind === "package") {
		return appCategoryForPackage(
			item.package.primaryCategory ?? item.package.secondaryCategory,
		);
	}
	const first = item.collection.preview[0];
	return first ? itemAppCategory(first) : undefined;
}

/** An admin accent picks a category color; `auto`/none uses the item's own category color. */
export function accentColor(
	accent: ExploreAccent | null | undefined,
	item: ExploreResolvedItem,
): string {
	if (accent?.startsWith("category:")) {
		return categoryColor(accent.slice("category:".length));
	}
	return categoryColor(itemAppCategory(item));
}
