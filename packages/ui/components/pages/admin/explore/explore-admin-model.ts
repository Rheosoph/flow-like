import { LANGUAGES } from "@flow-like/locales";
import type { TFunction } from "i18next";
import { apiErrorMessage } from "../../../../lib/api-error";
import { asArray, isRecord } from "../../../../lib/response-shape";
import {
	describeResponseShape,
	isExploreCtaHref,
	isExploreHttpsUrl,
	parseResolvedExplore,
	placementStatus,
	recordsOf,
} from "../../../store/explore/explore-model";
import {
	BENTO_SLOTS,
	EXPLORE_LIMITS,
	type ExploreAudienceTag,
	type ExploreChange,
	type ExploreCollection,
	type ExploreCollectionRule,
	type ExploreEditorState,
	type ExploreGridSlotKey,
	type ExploreItemKind,
	type ExploreItemRef,
	type ExploreLayoutDoc,
	type ExplorePlacement,
	type ExplorePlacementContent,
	type ExplorePlacementInput,
	type ExplorePlacementItem,
	type ExplorePlacementKind,
	type ExplorePreview,
	type ExploreRailKey,
	type ExploreSkipReason,
	type ExploreSlot,
	type ExploreSlotKey,
	type ExploreSlotTrace,
	type ExploreStatus,
	type ExploreTone,
	type ExploreViewer,
	GRID_SLOT_KEYS,
	ROW_RAILS,
	type ResolvedExplore,
} from "../../../store/explore/explore-types";

export type AdminT = TFunction<"admin">;

export const UNPLACED_SLOT = "unplaced" as const;
export const RAIL_KEYS: readonly ExploreRailKey[] = [
	"trending",
	"new",
	"new_count",
	"top_paid",
	"for_builders",
	"by_category",
	"suites",
];
export const TONES: readonly ExploreTone[] = [
	"info",
	"launch",
	"maintenance",
	"warning",
];
export const STATUSES: readonly ExploreStatus[] = [
	"live",
	"scheduled",
	"draft",
	"ended",
];
export const BASE_AUDIENCE_TAGS: readonly ExploreAudienceTag[] = [
	"everyone",
	"dev",
	"signed_in",
	"signed_out",
	"desktop",
	"web",
];
export const AUDIENCE_TAGS: readonly ExploreAudienceTag[] = [
	...BASE_AUDIENCE_TAGS,
	...LANGUAGES.map((code): ExploreAudienceTag => `locale:${code}`),
];

const ROW_KEY = /^row:[a-z0-9-]{1,40}$/;

export function isRowKey(key: string): key is `row:${string}` {
	return ROW_KEY.test(key);
}

export function slotArea(key: string): ExploreSlot["area"] | null {
	if (GRID_SLOT_KEYS.includes(key as ExploreGridSlotKey)) return "grid";
	if (key === UNPLACED_SLOT) return "unplaced";
	return isRowKey(key) ? "row" : null;
}

export function contentRail(
	content: ExplorePlacementContent,
): ExploreRailKey | undefined {
	return content.kind === "rail" ? content.rail : undefined;
}

/** §2.3: a slot accepts a placement by its content kind and, for rails, by the rail key. */
export function slotAccepts(
	slot: string,
	content: ExplorePlacementContent,
): boolean {
	if (slot === UNPLACED_SLOT) return true;
	const grid = BENTO_SLOTS.find((geometry) => geometry.key === slot);
	if (grid) {
		return (
			grid.kind === content.kind &&
			(grid.rail === undefined || grid.rail === contentRail(content))
		);
	}
	if (!isRowKey(slot)) return false;
	if (content.kind === "collection") return true;
	const rail = contentRail(content);
	return rail !== undefined && ROW_RAILS.includes(rail);
}

export function layoutRows(layout: ExploreLayoutDoc): ExploreSlot[] {
	return layout.slots
		.filter((slot) => slot.area === "row")
		.sort((a, b) => a.position - b.position);
}

export function findSlot(
	layout: ExploreLayoutDoc,
	key: string,
): ExploreSlot | undefined {
	return layout.slots.find((slot) => slot.key === key);
}

export interface PlacementLocation {
	slot: ExploreSlot;
	placement: ExplorePlacement;
	index: number;
}

export function findPlacement(
	layout: ExploreLayoutDoc,
	id: string,
): PlacementLocation | undefined {
	for (const slot of layout.slots) {
		const index = slot.placements.findIndex((placement) => placement.id === id);
		if (index >= 0) return { slot, placement: slot.placements[index], index };
	}
	return undefined;
}

export function allPlacements(
	layout: ExploreLayoutDoc,
): { slot: ExploreSlot; placement: ExplorePlacement; index: number }[] {
	return layout.slots.flatMap((slot) =>
		slot.placements.map((placement, index) => ({ slot, placement, index })),
	);
}

export function placementCount(layout: ExploreLayoutDoc): number {
	return layout.slots.reduce((sum, slot) => sum + slot.placements.length, 0);
}

/** Editor slot order: the fixed grid, rows by position, then unplaced. */
export function orderedSlotKeys(layout: ExploreLayoutDoc): ExploreSlotKey[] {
	return [
		...GRID_SLOT_KEYS,
		...layoutRows(layout).map((slot) => slot.key),
		UNPLACED_SLOT,
	];
}

export function newRowKey(layout: ExploreLayoutDoc): `row:${string}` {
	const taken = new Set(layout.slots.map((slot) => slot.key));
	for (let index = 1; ; index++) {
		const key = `row:row-${index}` as const;
		if (!taken.has(key)) return key;
	}
}

export function defaultRule(
	itemKind: ExploreCollectionRule["itemKind"] = "app",
): ExploreCollectionRule {
	return {
		itemKind,
		appCategory: null,
		appType: null,
		packageCategory: null,
		verifiedOnly: false,
		minRating: null,
		price: null,
		sort: "installs",
		limit: 8,
	};
}

function templateRail(slot: string): ExploreRailKey {
	return (
		BENTO_SLOTS.find((geometry) => geometry.key === slot)?.rail ?? "trending"
	);
}

function clip(value: string, max: number): string {
	return Array.from(value).slice(0, max).join("");
}

function templateContent(
	kind: ExplorePlacementKind,
	slot: string,
	name: string,
	rail?: ExploreRailKey,
): ExplorePlacementContent {
	switch (kind) {
		case "announcement":
			return {
				kind,
				tone: "info",
				title: clip(name, EXPLORE_LIMITS.announcementTitle),
				body: "",
				ctaLabel: null,
				ctaHref: null,
				imageUrl: null,
				dismissible: true,
			};
		case "spotlight":
			return { kind, rotationSeconds: 8, autoFill: true };
		case "feature":
			return { kind, eyebrow: null };
		case "collection":
			return {
				kind,
				title: clip(name, EXPLORE_LIMITS.collectionTitle),
				blurb: null,
				source: "rule",
				rule: defaultRule(),
			};
		case "rail":
			return { kind, rail: rail ?? templateRail(slot), title: null };
		case "sponsored":
			return { kind, advertiser: "" };
	}
}

/** A new placement starts disabled (Draft) and, where the kind allows it, already valid for POST. */
export function newPlacementTemplate(
	kind: ExplorePlacementKind,
	slot: ExploreSlotKey,
	name: string,
	rail?: ExploreRailKey,
): ExplorePlacementInput {
	return {
		name: clip(name, EXPLORE_LIMITS.name),
		enabled: false,
		startsAt: null,
		endsAt: null,
		audience: ["everyone"],
		content: templateContent(kind, slot, name, rail),
		items: [],
	};
}

export type LibraryKind =
	| "announcement"
	| "spotlight"
	| "collection"
	| "rail"
	| "sponsored";

export const LIBRARY_KINDS: readonly LibraryKind[] = [
	"announcement",
	"spotlight",
	"collection",
	"rail",
	"sponsored",
];

export interface AddChoice {
	kind: ExplorePlacementKind;
	rail?: ExploreRailKey;
}

export interface AddTarget {
	slot: ExploreSlotKey;
	newRow: boolean;
	choices: AddChoice[];
}

function libraryChoices(entry: LibraryKind): AddChoice[] {
	switch (entry) {
		case "spotlight":
			return [{ kind: "spotlight" }, { kind: "feature" }];
		case "rail":
			return RAIL_KEYS.map((rail) => ({ kind: "rail", rail }));
		default:
			return [{ kind: entry }];
	}
}

/** Where a library entry can be added: every slot whose `slotAccepts` holds, a new row while there is room, and
 * Unplaced. Empty when the draft already holds the maximum number of placements. */
export function addTargets(
	layout: ExploreLayoutDoc,
	entry: LibraryKind,
): AddTarget[] {
	if (placementCount(layout) >= EXPLORE_LIMITS.placements) return [];
	const rows = layoutRows(layout).map((slot) => slot.key);
	const candidates: { slot: ExploreSlotKey; newRow: boolean }[] = [
		...GRID_SLOT_KEYS.map((slot) => ({ slot, newRow: false })),
		...rows.map((slot) => ({ slot, newRow: false })),
		...(rows.length < EXPLORE_LIMITS.rows
			? [{ slot: newRowKey(layout), newRow: true }]
			: []),
		{ slot: UNPLACED_SLOT, newRow: false },
	];
	return candidates
		.map(({ slot, newRow }) => ({
			slot,
			newRow,
			choices: libraryChoices(entry).filter((choice) =>
				slotAccepts(slot, templateContent(choice.kind, slot, "", choice.rail)),
			),
		}))
		.filter((target) => target.choices.length > 0);
}

/** Everything the Add menu of one slot offers, across the whole library. */
export function addChoicesFor(
	layout: ExploreLayoutDoc,
	slot: ExploreSlotKey,
): AddChoice[] {
	return LIBRARY_KINDS.flatMap((entry) =>
		addTargets(layout, entry)
			.filter((target) => target.slot === slot)
			.flatMap((target) => target.choices),
	);
}

/** Slots a placement can move to: every other slot that accepts its content, Unplaced included. */
export function moveTargets(
	layout: ExploreLayoutDoc,
	placementId: string,
): ExploreSlotKey[] {
	const found = findPlacement(layout, placementId);
	if (!found) return [];
	return orderedSlotKeys(layout).filter(
		(key) =>
			key !== found.slot.key && slotAccepts(key, found.placement.content),
	);
}

export interface ExploreSlotOrder {
	key: string;
	placementIds: string[];
}

/** Body of PUT /admin/explore/order without the revision: the complete row list plus the full priority list of
 * every touched slot. */
export interface ExploreOrderInput {
	rows: string[];
	slots: ExploreSlotOrder[];
}

function rowKeys(layout: ExploreLayoutDoc): string[] {
	return layoutRows(layout).map((slot) => slot.key);
}

function slotIds(layout: ExploreLayoutDoc, key: string): string[] {
	return (findSlot(layout, key)?.placements ?? []).map(
		(placement) => placement.id,
	);
}

/** Moves a placement into `target` (appended, or at `index`). Both the source and the target slot are listed in
 * full, because the server rejects a partial or duplicated permutation. A new `row:<slug>` target is created. */
export function orderAfterMove(
	layout: ExploreLayoutDoc,
	placementId: string,
	target: ExploreSlotKey,
	index?: number,
): ExploreOrderInput {
	const found = findPlacement(layout, placementId);
	if (!found) {
		throw new Error(`Placement ${placementId} is not in the Explore draft`);
	}
	if (found.slot.key === target) {
		throw new Error(`Placement ${placementId} is already in ${target}`);
	}
	if (slotArea(target) === null) {
		throw new Error(`Slot ${target} is not part of the Explore layout`);
	}
	if (!slotAccepts(target, found.placement.content)) {
		throw new Error(
			`${target} does not accept ${placementLabel(found.placement.content)}`,
		);
	}
	const rows = rowKeys(layout);
	if (isRowKey(target) && !rows.includes(target)) rows.push(target);
	const source = slotIds(layout, found.slot.key).filter(
		(id) => id !== placementId,
	);
	const destination = slotIds(layout, target);
	const at =
		index === undefined
			? destination.length
			: Math.max(0, Math.min(index, destination.length));
	destination.splice(at, 0, placementId);
	return {
		rows,
		slots: [
			{ key: found.slot.key, placementIds: source },
			{ key: target, placementIds: destination },
		],
	};
}

/** A new priority order inside one slot; `ids` must be a permutation of the slot's placements. */
export function orderWithPriority(
	layout: ExploreLayoutDoc,
	slotKey: ExploreSlotKey,
	ids: readonly string[],
): ExploreOrderInput {
	const current = slotIds(layout, slotKey);
	const same =
		ids.length === current.length &&
		new Set(ids).size === ids.length &&
		ids.every((id) => current.includes(id));
	if (!same) {
		throw new Error(
			`The new order for ${slotKey} must list each of its ${current.length} placements once`,
		);
	}
	return {
		rows: rowKeys(layout),
		slots: [{ key: slotKey, placementIds: [...ids] }],
	};
}

export function orderWithRows(
	layout: ExploreLayoutDoc,
	rows: readonly string[],
): ExploreOrderInput {
	const current = rowKeys(layout);
	const same =
		rows.length === current.length &&
		new Set(rows).size === rows.length &&
		rows.every((key) => current.includes(key));
	if (!same) {
		throw new Error("The new row order must list every row once");
	}
	return { rows: [...rows], slots: [] };
}

export function orderWithNewRow(
	layout: ExploreLayoutDoc,
	key: `row:${string}` = newRowKey(layout),
): ExploreOrderInput {
	const rows = rowKeys(layout);
	if (rows.length >= EXPLORE_LIMITS.rows) {
		throw new Error(
			`An Explore layout holds at most ${EXPLORE_LIMITS.rows} rows`,
		);
	}
	if (rows.includes(key)) throw new Error(`Row ${key} already exists`);
	return { rows: [...rows, key], slots: [] };
}

/** Removes an empty row; a row that still holds placements must be emptied first. */
export function orderWithoutRow(
	layout: ExploreLayoutDoc,
	rowKey: string,
): ExploreOrderInput {
	const row = findSlot(layout, rowKey);
	if (!row || row.area !== "row") {
		throw new Error(`Row ${rowKey} is not part of the Explore layout`);
	}
	if (row.placements.length) {
		throw new Error(`Row ${rowKey} still has placements; move them first`);
	}
	return {
		rows: rowKeys(layout).filter((key) => key !== rowKey),
		slots: [],
	};
}

function cleanItem(item: ExplorePlacementItem): ExplorePlacementItem {
	const clean: ExplorePlacementItem = { kind: item.kind, id: item.id };
	if (item.headline != null) clean.headline = item.headline;
	if (item.subline != null) clean.subline = item.subline;
	if (item.artworkUrl != null) clean.artworkUrl = item.artworkUrl;
	if (item.accent != null) clean.accent = item.accent;
	return clean;
}

/** The PUT/POST body for a placement: never `kind`, and items carry only the fields the server accepts. */
export function placementInput(
	placement: ExplorePlacement | ExplorePlacementInput,
): ExplorePlacementInput {
	return {
		name: placement.name,
		enabled: placement.enabled,
		startsAt: placement.startsAt ?? null,
		endsAt: placement.endsAt ?? null,
		audience: [...placement.audience],
		content: { ...placement.content },
		items: placement.items.map(cleanItem),
	};
}

function normalizeValue(value: unknown): unknown {
	if (typeof value === "string") return value.trim() || null;
	if (value === undefined) return null;
	if (Array.isArray(value)) return value.map(normalizeValue);
	if (isRecord(value)) {
		return Object.fromEntries(
			Object.keys(value)
				.sort()
				.map((key) => [key, normalizeValue(value[key])]),
		);
	}
	return value;
}

function normalizeContent(
	content: ExplorePlacementContent,
): ExplorePlacementContent {
	if (content.kind !== "collection") return content;
	if (content.source === "hand") return { ...content, rule: null };
	if (!content.rule) return content;
	const rule =
		content.rule.itemKind === "package"
			? { ...content.rule, appCategory: null, appType: null }
			: { ...content.rule, packageCategory: null, verifiedOnly: false };
	return { ...content, rule };
}

/** Compares two inputs the way the server stores them: trimmed text, empty optionals as null, hand-picked
 * collections without a rule. Used to tell our own echo from someone else's edit. */
export function sameInput(
	a: ExplorePlacementInput,
	b: ExplorePlacementInput,
): boolean {
	const shape = (input: ExplorePlacementInput) =>
		JSON.stringify(
			normalizeValue({
				...placementInput(input),
				content: normalizeContent(input.content),
				startsAt: input.startsAt ? Date.parse(input.startsAt) : null,
				endsAt: input.endsAt ? Date.parse(input.endsAt) : null,
			}),
		);
	return shape(a) === shape(b);
}

export function duplicateInput(
	placement: ExplorePlacement,
	suffix = " (copy)",
): ExplorePlacementInput {
	const room = Math.max(0, EXPLORE_LIMITS.name - Array.from(suffix).length);
	return {
		...placementInput(placement),
		name: `${clip(placement.name, room)}${suffix}`,
		enabled: false,
	};
}

/** Placements whose items point at this collection (spotlight slides); the server refuses to delete it. */
export function referencingSpotlights(
	layout: ExploreLayoutDoc,
	collectionId: string,
): ExplorePlacement[] {
	return allPlacements(layout)
		.map(({ placement }) => placement)
		.filter((placement) =>
			placement.items.some(
				(item) => item.kind === "collection" && item.id === collectionId,
			),
		);
}

/** Applies an override patch to one item of the latest list. The item is found by identity, so an upload that
 * finishes after the list was reordered or edited still lands on the right item, and on none once it is gone. */
export function patchItem(
	items: readonly ExplorePlacementItem[],
	target: Pick<ExplorePlacementItem, "kind" | "id">,
	index: number,
	patch: Partial<Omit<ExplorePlacementItem, "kind" | "id">>,
): ExplorePlacementItem[] {
	const matches = (item: ExplorePlacementItem | undefined) =>
		item?.kind === target.kind && item.id === target.id;
	const at = matches(items[index]) ? index : items.findIndex(matches);
	return items.map((item, position) =>
		position === at ? { ...item, ...patch } : item,
	);
}

/** The id POST added to `slotKey`: present in `after` but nowhere in `before`. */
export function addedPlacementId(
	before: ExploreLayoutDoc | undefined,
	after: ExploreLayoutDoc,
	slotKey: string,
): string | undefined {
	const known = new Set(
		before ? allPlacements(before).map(({ placement }) => placement.id) : [],
	);
	return findSlot(after, slotKey)?.placements.find(
		(placement) => !known.has(placement.id),
	)?.id;
}

export function effectiveStatus(
	placement: Pick<
		ExplorePlacement,
		"enabled" | "startsAt" | "endsAt" | "status"
	>,
	now: string | number | Date = Date.now(),
): ExploreStatus {
	return placement.status ?? placementStatus(placement, now);
}

export function statusCounts(
	layout: ExploreLayoutDoc,
	now?: string,
): Record<ExploreStatus, number> {
	const counts: Record<ExploreStatus, number> = {
		live: 0,
		scheduled: 0,
		draft: 0,
		ended: 0,
	};
	for (const { placement } of allPlacements(layout)) {
		counts[effectiveStatus(placement, now)]++;
	}
	return counts;
}

/** Everyone is exclusive, signed in/out exclude each other, and an empty list means everyone again. */
export function toggleAudience(
	audience: readonly ExploreAudienceTag[],
	tag: ExploreAudienceTag,
): ExploreAudienceTag[] {
	if (tag === "everyone") return ["everyone"];
	const current = audience.filter((entry) => entry !== "everyone");
	if (current.includes(tag)) {
		const next = current.filter((entry) => entry !== tag);
		return next.length ? next : ["everyone"];
	}
	const opposite =
		tag === "signed_in"
			? "signed_out"
			: tag === "signed_out"
				? "signed_in"
				: undefined;
	return [...current.filter((entry) => entry !== opposite), tag];
}

export function audienceIncludes(
	audience: readonly string[],
	tag: ExploreAudienceTag,
): boolean {
	if (tag === "everyone") {
		return audience.length === 0 || audience.includes("everyone");
	}
	return audience.includes(tag);
}

const PREVIEW_SKIPS: readonly ExploreSkipReason[] = [
	"audience",
	"dev_only",
	"too_few_items",
	"empty",
];

export interface TraceMarkers {
	/** Slot key → the placement the preview shows there, or null when the slot stays empty. */
	chosen: ReadonlyMap<string, string | null>;
	/** Placements shown although a higher-priority one exists in their slot. */
	fallback: ReadonlySet<string>;
	/** Live placements the previewed viewer does not get, with the server's reason. */
	hidden: ReadonlyMap<string, ExploreSkipReason>;
}

/** Reads the server's preview trace; the editor never re-implements selection. */
export function traceMarkers(
	trace: readonly ExploreSlotTrace[],
	layout: ExploreLayoutDoc,
): TraceMarkers {
	const chosen = new Map<string, string | null>();
	const fallback = new Set<string>();
	const hidden = new Map<string, ExploreSkipReason>();
	for (const entry of trace) {
		chosen.set(entry.slotKey, entry.chosen);
		const first = findSlot(layout, entry.slotKey)?.placements[0]?.id;
		if (entry.chosen && first && entry.chosen !== first) {
			fallback.add(entry.chosen);
		}
		for (const skipped of entry.skipped) {
			if (PREVIEW_SKIPS.includes(skipped.reason)) {
				hidden.set(skipped.placementId, skipped.reason);
			}
		}
	}
	return { chosen, fallback, hidden };
}

/** How many slides the previewed viewer gets from this spotlight; undefined when the preview shows another one. */
export function previewSlideCount(
	page: ResolvedExplore | undefined,
	placementId: string,
): number | undefined {
	const hero = page?.views.all.grid.hero;
	return hero?.placementId === placementId ? hero.slides.length : undefined;
}

/** The collection as the preview resolved it, in the grid or in a row. */
export function previewCollection(
	page: ResolvedExplore | undefined,
	placementId: string,
): ExploreCollection | undefined {
	if (!page) return undefined;
	const view = page.views.all;
	if (view.grid.collection?.placementId === placementId) {
		return view.grid.collection;
	}
	for (const row of view.rows) {
		if (
			row.kind === "collection" &&
			row.collection.placementId === placementId
		) {
			return row.collection;
		}
	}
	return undefined;
}

export type PlacementIssueCode =
	| "required"
	| "too_long"
	| "https_url"
	| "cta_href"
	| "cta_pair"
	| "item_count"
	| "item_kind"
	| "duplicate_item"
	| "missing_collection"
	| "window"
	| "audience_everyone"
	| "audience_sign_in"
	| "audience_unknown"
	| "rotation"
	| "sponsored_enabled"
	| "rule_rating"
	| "rule_limit";

export interface PlacementIssue {
	field: string;
	code: PlacementIssueCode;
	min?: number;
	max?: number;
}

const RULE_LIMIT_MIN = 2;
const RULE_LIMIT_MAX = 12;
const COLLECTION_ITEMS_MIN = 2;

function length(value: string): number {
	return Array.from(value.trim()).length;
}

function present(value: string | null | undefined): value is string {
	return typeof value === "string" && value.trim() !== "";
}

function isAudienceTag(tag: string): boolean {
	if ((BASE_AUDIENCE_TAGS as readonly string[]).includes(tag)) return true;
	if (!tag.startsWith("locale:")) return false;
	const code = tag.slice("locale:".length).toLowerCase();
	return LANGUAGES.some((language) => language.toLowerCase() === code);
}

function itemRange(
	content: ExplorePlacementContent,
): [number, number, readonly ExploreItemKind[]] {
	const picks: readonly ExploreItemKind[] = ["app", "package"];
	switch (content.kind) {
		case "spotlight":
			return [
				content.autoFill ? 0 : 1,
				EXPLORE_LIMITS.spotlightItems,
				["app", "package", "collection"],
			];
		case "feature":
		case "sponsored":
			return [1, 1, picks];
		case "collection":
			return content.source === "rule"
				? [0, EXPLORE_LIMITS.rulePinned, picks]
				: [COLLECTION_ITEMS_MIN, EXPLORE_LIMITS.collectionItems, picks];
		default:
			return [0, 0, picks];
	}
}

/** Mirrors the server's §3.2 checks so the inspector can flag a field before a PUT would fail. */
export function validatePlacement(
	input: ExplorePlacementInput,
	collectionIds?: ReadonlySet<string>,
): PlacementIssue[] {
	const issues: PlacementIssue[] = [];
	const text = (
		field: string,
		value: string | null | undefined,
		max: number,
		required = false,
	) => {
		if (!present(value)) {
			if (required) issues.push({ field, code: "required" });
			return;
		}
		if (length(value) > max) issues.push({ field, code: "too_long", max });
	};
	const https = (field: string, value: string | null | undefined) => {
		if (present(value) && !isExploreHttpsUrl(value.trim())) {
			issues.push({ field, code: "https_url" });
		}
	};

	text("name", input.name, EXPLORE_LIMITS.name, true);
	const content = input.content;
	switch (content.kind) {
		case "announcement":
			text("title", content.title, EXPLORE_LIMITS.announcementTitle, true);
			text("body", content.body, EXPLORE_LIMITS.announcementBody);
			text("ctaLabel", content.ctaLabel, EXPLORE_LIMITS.ctaLabel);
			if (present(content.ctaLabel) !== present(content.ctaHref)) {
				issues.push({
					field: present(content.ctaLabel) ? "ctaHref" : "ctaLabel",
					code: "cta_pair",
				});
			}
			if (
				present(content.ctaHref) &&
				!isExploreCtaHref(content.ctaHref.trim())
			) {
				issues.push({ field: "ctaHref", code: "cta_href" });
			}
			https("imageUrl", content.imageUrl);
			break;
		case "spotlight":
			if (
				!Number.isInteger(content.rotationSeconds) ||
				content.rotationSeconds < EXPLORE_LIMITS.rotationMin ||
				content.rotationSeconds > EXPLORE_LIMITS.rotationMax
			) {
				issues.push({
					field: "rotationSeconds",
					code: "rotation",
					min: EXPLORE_LIMITS.rotationMin,
					max: EXPLORE_LIMITS.rotationMax,
				});
			}
			break;
		case "feature":
			text("eyebrow", content.eyebrow, EXPLORE_LIMITS.eyebrow);
			break;
		case "collection":
			text("title", content.title, EXPLORE_LIMITS.collectionTitle, true);
			text("blurb", content.blurb, EXPLORE_LIMITS.blurb);
			if (content.source === "rule") {
				const rule = content.rule;
				if (!rule) {
					issues.push({ field: "rule", code: "required" });
				} else {
					if (
						rule.minRating != null &&
						!(rule.minRating >= 0 && rule.minRating <= 5)
					) {
						issues.push({
							field: "rule.minRating",
							code: "rule_rating",
							min: 0,
							max: 5,
						});
					}
					if (
						!Number.isInteger(rule.limit) ||
						rule.limit < RULE_LIMIT_MIN ||
						rule.limit > RULE_LIMIT_MAX
					) {
						issues.push({
							field: "rule.limit",
							code: "rule_limit",
							min: RULE_LIMIT_MIN,
							max: RULE_LIMIT_MAX,
						});
					}
				}
			}
			break;
		case "rail":
			text("title", content.title, EXPLORE_LIMITS.railTitle);
			break;
		case "sponsored":
			text("advertiser", content.advertiser, EXPLORE_LIMITS.advertiser, true);
			if (input.enabled) {
				issues.push({ field: "enabled", code: "sponsored_enabled" });
			}
			break;
	}

	const [min, max, kinds] = itemRange(content);
	if (input.items.length < min || input.items.length > max) {
		issues.push({ field: "items", code: "item_count", min, max });
	}
	const seen = new Set<string>();
	input.items.forEach((item, index) => {
		const key = `${item.kind}:${item.id}`;
		if (!kinds.includes(item.kind)) {
			issues.push({ field: `items.${index}`, code: "item_kind" });
		}
		if (seen.has(key)) {
			issues.push({ field: `items.${index}`, code: "duplicate_item" });
		}
		seen.add(key);
		if (
			item.kind === "collection" &&
			collectionIds &&
			!collectionIds.has(item.id)
		) {
			issues.push({ field: `items.${index}`, code: "missing_collection" });
		}
		text(`items.${index}.headline`, item.headline, EXPLORE_LIMITS.headline);
		text(`items.${index}.subline`, item.subline, EXPLORE_LIMITS.subline);
		https(`items.${index}.artworkUrl`, item.artworkUrl);
	});

	if (
		input.startsAt &&
		input.endsAt &&
		Date.parse(input.startsAt) >= Date.parse(input.endsAt)
	) {
		issues.push({ field: "schedule", code: "window" });
	}
	if (input.audience.some((tag) => !isAudienceTag(tag))) {
		issues.push({ field: "audience", code: "audience_unknown" });
	}
	if (input.audience.includes("everyone") && input.audience.length > 1) {
		issues.push({ field: "audience", code: "audience_everyone" });
	}
	if (
		input.audience.includes("signed_in") &&
		input.audience.includes("signed_out")
	) {
		issues.push({ field: "audience", code: "audience_sign_in" });
	}
	return issues;
}

/** Keeps the other kinds and the overrides of items that stay, appends new ids, drops deselected ones. */
export function replaceKindItems(
	items: readonly ExplorePlacementItem[],
	kind: ExploreItemKind,
	ids: readonly string[],
): ExplorePlacementItem[] {
	const kept = items.filter(
		(item) => item.kind !== kind || ids.includes(item.id),
	);
	const known = new Set(
		kept.filter((item) => item.kind === kind).map((item) => item.id),
	);
	return [
		...kept,
		...ids.filter((id) => !known.has(id)).map((id) => ({ kind, id })),
	];
}

export function collectionPlacements(
	layout: ExploreLayoutDoc,
): ExplorePlacement[] {
	return allPlacements(layout)
		.map(({ placement }) => placement)
		.filter((placement) => placement.content.kind === "collection");
}

export function refKey(kind: ExploreItemKind, id: string): string {
	return `${kind}:${id}`;
}

export function refsByKey(
	refs: readonly ExploreItemRef[],
): Map<string, ExploreItemRef> {
	return new Map(refs.map((ref) => [refKey(ref.kind, ref.id), ref]));
}

/** `datetime-local` value in the admin's zone for an ISO instant. */
export function toLocalInput(iso: string | null | undefined): string {
	if (!iso) return "";
	const date = new Date(iso);
	if (Number.isNaN(date.getTime())) return "";
	const pad = (value: number) => String(value).padStart(2, "0");
	return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

/** ISO UTC for a `datetime-local` value, which the browser reads in the admin's zone. */
export function fromLocalInput(value: string): string | null {
	if (!value.trim()) return null;
	const date = new Date(value);
	return Number.isNaN(date.getTime()) ? null : date.toISOString();
}

export function slotSize(key: string): string {
	const grid = BENTO_SLOTS.find((geometry) => geometry.key === key);
	if (grid) return `${grid.span}×${grid.rowSpan}`;
	return isRowKey(key) ? "12×1" : "—";
}

export function errorStatus(error: unknown): number | undefined {
	return isRecord(error) && typeof error.status === "number"
		? error.status
		: undefined;
}

export function errorMessage(error: unknown, fallback: string): string {
	return apiErrorMessage(
		error,
		error instanceof Error && error.message.trim() ? error.message : fallback,
	);
}

function strings(value: unknown): string[] {
	return asArray(value as unknown[] | undefined).filter(
		(entry): entry is string => typeof entry === "string",
	);
}

function optionalString(value: unknown): string | null {
	return typeof value === "string" ? value : null;
}

const ITEM_KINDS: readonly ExploreItemKind[] = ["app", "package", "collection"];
const CHANGE_KINDS: readonly ExploreChange["change"][] = [
	"added",
	"removed",
	"modified",
	"moved",
];

function parseItem(value: Record<string, unknown>): ExplorePlacementItem[] {
	if (
		typeof value.id !== "string" ||
		!ITEM_KINDS.includes(value.kind as ExploreItemKind)
	) {
		return [];
	}
	return [
		cleanItem({
			kind: value.kind as ExploreItemKind,
			id: value.id,
			headline: optionalString(value.headline),
			subline: optionalString(value.subline),
			artworkUrl: optionalString(value.artworkUrl),
			accent: optionalString(value.accent) as ExplorePlacementItem["accent"],
		}),
	];
}

function parsePlacement(value: Record<string, unknown>): ExplorePlacement[] {
	const content = value.content;
	if (
		typeof value.id !== "string" ||
		!isRecord(content) ||
		typeof content.kind !== "string"
	) {
		return [];
	}
	return [
		{
			id: value.id,
			kind: content.kind as ExplorePlacementKind,
			name: typeof value.name === "string" ? value.name : value.id,
			enabled: value.enabled === true,
			startsAt: optionalString(value.startsAt),
			endsAt: optionalString(value.endsAt),
			audience: strings(value.audience) as ExploreAudienceTag[],
			content: content as unknown as ExplorePlacementContent,
			items: recordsOf(value.items).flatMap(parseItem),
			status: STATUSES.includes(value.status as ExploreStatus)
				? (value.status as ExploreStatus)
				: undefined,
			updatedAt: optionalString(value.updatedAt) ?? undefined,
		},
	];
}

function parseSlot(value: Record<string, unknown>): ExploreSlot[] {
	const key = value.key;
	if (typeof key !== "string") return [];
	const area = slotArea(key);
	if (!area) return [];
	return [
		{
			key: key as ExploreSlotKey,
			area,
			position: typeof value.position === "number" ? value.position : 0,
			placements: recordsOf(value.placements).flatMap(parsePlacement),
		},
	];
}

export function parseEditorState(value: unknown): ExploreEditorState {
	if (!isRecord(value) || !isRecord(value.layout)) {
		throw new Error(
			`The Explore editor answered with ${describeResponseShape(value)} instead of the draft layout`,
		);
	}
	return {
		draftRevision:
			typeof value.draftRevision === "string" ? value.draftRevision : "default",
		liveRevision: optionalString(value.liveRevision),
		publishedAt: optionalString(value.publishedAt),
		now: typeof value.now === "string" ? value.now : new Date().toISOString(),
		layout: { slots: recordsOf(value.layout.slots).flatMap(parseSlot) },
		refs: recordsOf(value.refs).flatMap((ref) =>
			typeof ref.id === "string" &&
			ITEM_KINDS.includes(ref.kind as ExploreItemKind)
				? [
						{
							kind: ref.kind as ExploreItemKind,
							id: ref.id,
							name: typeof ref.name === "string" ? ref.name : ref.id,
							iconUrl: optionalString(ref.iconUrl),
							coverUrl: optionalString(ref.coverUrl),
							public: ref.public !== false,
							exists: ref.exists !== false,
						},
					]
				: [],
		),
		changes: recordsOf(value.changes).flatMap((change) =>
			typeof change.placementId === "string" &&
			CHANGE_KINDS.includes(change.change as ExploreChange["change"])
				? [
						{
							placementId: change.placementId,
							name:
								typeof change.name === "string"
									? change.name
									: change.placementId,
							change: change.change as ExploreChange["change"],
						},
					]
				: [],
		),
		warnings: strings(value.warnings),
	};
}

const SKIP_REASONS: readonly ExploreSkipReason[] = [
	"draft",
	"scheduled",
	"ended",
	"audience",
	"dev_only",
	"too_few_items",
	"empty",
];

export function parsePreview(value: unknown): ExplorePreview {
	if (!isRecord(value)) {
		throw new Error(
			`The Explore preview answered with ${describeResponseShape(value)} instead of a page`,
		);
	}
	return {
		page: parseResolvedExplore(value.page),
		trace: recordsOf(value.trace).flatMap((entry) =>
			typeof entry.slotKey === "string"
				? [
						{
							slotKey: entry.slotKey as ExploreSlotKey,
							chosen: optionalString(entry.chosen),
							skipped: recordsOf(entry.skipped).flatMap((skipped) =>
								typeof skipped.placementId === "string" &&
								SKIP_REASONS.includes(skipped.reason as ExploreSkipReason)
									? [
											{
												placementId: skipped.placementId,
												reason: skipped.reason as ExploreSkipReason,
											},
										]
									: [],
							),
						},
					]
				: [],
		),
	};
}

export function adminPreviewPath(viewer: ExploreViewer): string {
	const params = new URLSearchParams({
		source: "draft",
		dev: String(viewer.dev),
		signed_in: String(viewer.signedIn),
		platform: viewer.platform,
		language: viewer.language,
	});
	return `admin/explore/preview?${params}`;
}

export type MediaFormat = "webp" | "png" | "jpeg";

export function mediaFormat(type: string): MediaFormat | null {
	switch (type) {
		case "image/webp":
			return "webp";
		case "image/png":
			return "png";
		case "image/jpeg":
			return "jpeg";
		default:
			return null;
	}
}

function placementLabel(content: ExplorePlacementContent): string {
	const rail = contentRail(content);
	return rail ? `rail/${rail}` : content.kind;
}

export function kindLabel(kind: ExplorePlacementKind, t: AdminT): string {
	switch (kind) {
		case "announcement":
			return t("exploreKindAnnouncement", "Announcement", { ns: "admin" });
		case "spotlight":
			return t("exploreKindSpotlight", "Spotlight", { ns: "admin" });
		case "feature":
			return t("exploreKindFeature", "Feature", { ns: "admin" });
		case "collection":
			return t("exploreKindCollection", "Collection", { ns: "admin" });
		case "rail":
			return t("exploreKindRail", "System rail", { ns: "admin" });
		case "sponsored":
			return t("exploreKindSponsored", "Sponsored", { ns: "admin" });
	}
}

/** Canvas badges use the short rail label, as the tiles are narrow. */
export function kindBadgeLabel(kind: ExplorePlacementKind, t: AdminT): string {
	return kind === "rail"
		? t("exploreKindRailShort", "Rail", { ns: "admin" })
		: kindLabel(kind, t);
}

export function libraryLabel(
	entry: LibraryKind,
	t: AdminT,
): { name: string; description: string } {
	switch (entry) {
		case "announcement":
			return {
				name: t("exploreKindAnnouncement", "Announcement", { ns: "admin" }),
				description: t(
					"exploreLibraryAnnouncement",
					"Tone, call to action, dismiss",
					{ ns: "admin" },
				),
			};
		case "spotlight":
			return {
				name: t("exploreKindSpotlight", "Spotlight", { ns: "admin" }),
				description: t(
					"exploreLibrarySpotlight",
					"App, package or collection",
					{ ns: "admin" },
				),
			};
		case "collection":
			return {
				name: t("exploreKindCollection", "Collection", { ns: "admin" }),
				description: t("exploreLibraryCollection", "Hand-picked or by rule", {
					ns: "admin",
				}),
			};
		case "rail":
			return {
				name: t("exploreKindRail", "System rail", { ns: "admin" }),
				description: t("exploreLibraryRail", "Trending, New, Top paid…", {
					ns: "admin",
				}),
			};
		case "sponsored":
			return {
				name: t("exploreKindSponsored", "Sponsored", { ns: "admin" }),
				description: t(
					"exploreLibrarySponsored",
					"Always labelled “Sponsored”",
					{ ns: "admin" },
				),
			};
	}
}

export function railLabel(rail: ExploreRailKey, t: AdminT): string {
	switch (rail) {
		case "trending":
			return t("exploreRailTrending", "Popular right now", { ns: "admin" });
		case "new":
			return t("exploreRailNew", "New this week", { ns: "admin" });
		case "new_count":
			return t("exploreRailNewCount", "New this week (count)", {
				ns: "admin",
			});
		case "top_paid":
			return t("exploreRailTopPaid", "Top paid", { ns: "admin" });
		case "for_builders":
			return t("exploreRailForBuilders", "For builders", { ns: "admin" });
		case "by_category":
			return t("exploreRailByCategory", "By category", { ns: "admin" });
		case "suites":
			return t("exploreRailSuites", "Suites & Platforms", { ns: "admin" });
	}
}

export function railSource(rail: ExploreRailKey, t: AdminT): string {
	switch (rail) {
		case "trending":
			return t(
				"exploreRailSourceTrending",
				"The most installed apps, and packages in developer mode, by total installs.",
				{ ns: "admin" },
			);
		case "new":
			return t(
				"exploreRailSourceNew",
				"Apps and packages published in the last 7 days, newest first.",
				{ ns: "admin" },
			);
		case "new_count":
			return t(
				"exploreRailSourceNewCount",
				"Counts what was published in the last 7 days. Viewers without developer mode see the app count only.",
				{ ns: "admin" },
			);
		case "top_paid":
			return t(
				"exploreRailSourceTopPaid",
				"Best-selling paid apps and packages over the last 30 days.",
				{ ns: "admin" },
			);
		case "for_builders":
			return t(
				"exploreRailSourceForBuilders",
				"Packages only, verified publishers first, sorted by installs. Empty without developer mode, so add a fallback.",
				{ ns: "admin" },
			);
		case "by_category":
			return t(
				"exploreRailSourceByCategory",
				"The 4 categories with the most installs, with app and package counts.",
				{ ns: "admin" },
			);
		case "suites":
			return t(
				"exploreRailSourceSuites",
				"The published suites and platforms, rendered by the app.",
				{ ns: "admin" },
			);
	}
}

export function toneLabel(tone: ExploreTone, t: AdminT): string {
	switch (tone) {
		case "info":
			return t("exploreToneInfo", "Info", { ns: "admin" });
		case "launch":
			return t("exploreToneLaunch", "Launch", { ns: "admin" });
		case "maintenance":
			return t("exploreToneMaintenance", "Maintenance", { ns: "admin" });
		case "warning":
			return t("exploreToneWarning", "Warning", { ns: "admin" });
	}
}

export function statusLabel(status: ExploreStatus, t: AdminT): string {
	switch (status) {
		case "live":
			return t("exploreStatusLive", "Live", { ns: "admin" });
		case "scheduled":
			return t("exploreStatusScheduled", "Scheduled", { ns: "admin" });
		case "draft":
			return t("exploreStatusDraft", "Draft", { ns: "admin" });
		case "ended":
			return t("exploreStatusEnded", "Ended", { ns: "admin" });
	}
}

export function statusCountLabel(
	status: ExploreStatus,
	count: number,
	t: AdminT,
): string {
	switch (status) {
		case "live":
			return t("exploreCountLive", "{{count}} live", { ns: "admin", count });
		case "scheduled":
			return t("exploreCountScheduled", "{{count}} scheduled", {
				ns: "admin",
				count,
			});
		case "draft":
			return t("exploreCountDraft", "{{count}} drafts", { ns: "admin", count });
		case "ended":
			return t("exploreCountEnded", "{{count}} ended", { ns: "admin", count });
	}
}

export function statusNote(
	placement: Pick<ExplorePlacement, "startsAt" | "endsAt">,
	status: ExploreStatus,
	t: AdminT,
	formatDate: (iso: string) => string,
): string {
	switch (status) {
		case "live":
			return placement.endsAt
				? t("exploreStatusNoteLiveUntil", "On · until {{date}}", {
						ns: "admin",
						date: formatDate(placement.endsAt),
					})
				: t("exploreStatusNoteLive", "On · no end date", { ns: "admin" });
		case "scheduled":
			return t("exploreStatusNoteScheduled", "On · starts {{date}}", {
				ns: "admin",
				date: placement.startsAt ? formatDate(placement.startsAt) : "",
			});
		case "draft":
			return t(
				"exploreStatusNoteDraft",
				"Off · only admins see it in the editor",
				{ ns: "admin" },
			);
		case "ended":
			return t(
				"exploreStatusNoteEnded",
				"Ended {{date}} · kept for reference",
				{
					ns: "admin",
					date: placement.endsAt ? formatDate(placement.endsAt) : "",
				},
			);
	}
}

export function scheduleSummary(
	placement: Pick<ExplorePlacement, "startsAt" | "endsAt">,
	t: AdminT,
	formatDate: (iso: string) => string,
): string {
	const { startsAt, endsAt } = placement;
	if (startsAt && endsAt) {
		return t("exploreScheduleRange", "{{start}} → {{end}}", {
			ns: "admin",
			start: formatDate(startsAt),
			end: formatDate(endsAt),
		});
	}
	if (startsAt) {
		return t("exploreScheduleFrom", "From {{date}}", {
			ns: "admin",
			date: formatDate(startsAt),
		});
	}
	if (endsAt) {
		return t("exploreScheduleUntil", "Until {{date}}", {
			ns: "admin",
			date: formatDate(endsAt),
		});
	}
	return t("exploreScheduleAlways", "Always on", { ns: "admin" });
}

export function audienceLabel(tag: string, t: AdminT): string {
	switch (tag) {
		case "everyone":
			return t("exploreAudienceEveryone", "Everyone", { ns: "admin" });
		case "dev":
			return t("exploreAudienceDev", "Developer mode", { ns: "admin" });
		case "signed_in":
			return t("exploreAudienceSignedIn", "Signed in", { ns: "admin" });
		case "signed_out":
			return t("exploreAudienceSignedOut", "Signed out", { ns: "admin" });
		case "desktop":
			return t("exploreAudienceDesktop", "Desktop", { ns: "admin" });
		case "web":
			return t("exploreAudienceWeb", "Web", { ns: "admin" });
		default:
			return tag.startsWith("locale:") ? tag.slice("locale:".length) : tag;
	}
}

export function slotLabel(
	layout: ExploreLayoutDoc,
	key: string,
	t: AdminT,
): string {
	switch (key) {
		case "hero":
			return t("exploreSlotHero", "Spotlight", { ns: "admin" });
		case "notice":
			return t("exploreSlotNotice", "Announcement", { ns: "admin" });
		case "feature":
			return t("exploreSlotFeature", "Feature", { ns: "admin" });
		case "collection":
			return t("exploreSlotCollection", "Collection", { ns: "admin" });
		case "stat":
			return t("exploreSlotStat", "New count", { ns: "admin" });
		case "categories":
			return t("exploreSlotCategories", "Categories", { ns: "admin" });
		case UNPLACED_SLOT:
			return t("exploreSlotUnplaced", "Unplaced", { ns: "admin" });
		default: {
			const index = layoutRows(layout).findIndex((slot) => slot.key === key);
			return index >= 0
				? t("exploreSlotRow", "Row {{number}}", {
						ns: "admin",
						number: index + 1,
					})
				: t("exploreSlotNewRow", "New row", { ns: "admin" });
		}
	}
}

export function choiceLabel(choice: AddChoice, t: AdminT): string {
	if (choice.rail) return railLabel(choice.rail, t);
	switch (choice.kind) {
		case "spotlight":
			return t("exploreChoiceSpotlight", "Spotlight rotation", { ns: "admin" });
		case "feature":
			return t("exploreChoiceFeature", "Featured item", { ns: "admin" });
		default:
			return kindLabel(choice.kind, t);
	}
}

export function defaultPlacementName(choice: AddChoice, t: AdminT): string {
	switch (choice.kind) {
		case "announcement":
			return t("exploreNewAnnouncement", "New announcement", { ns: "admin" });
		case "spotlight":
			return t("exploreNewSpotlight", "New spotlight", { ns: "admin" });
		case "feature":
			return t("exploreNewFeature", "New featured item", { ns: "admin" });
		case "collection":
			return t("exploreNewCollection", "New collection", { ns: "admin" });
		case "sponsored":
			return t("exploreNewSponsored", "Sponsored slot", { ns: "admin" });
		case "rail":
			return choice.rail
				? railLabel(choice.rail, t)
				: t("exploreKindRail", "System rail", { ns: "admin" });
	}
}

export function skipReasonLabel(reason: ExploreSkipReason, t: AdminT): string {
	switch (reason) {
		case "draft":
			return t("exploreSkipDraft", "Off", { ns: "admin" });
		case "scheduled":
			return t("exploreSkipScheduled", "Not started yet", { ns: "admin" });
		case "ended":
			return t("exploreSkipEnded", "Ended", { ns: "admin" });
		case "audience":
			return t(
				"exploreSkipAudience",
				"Not shown for the audience you are previewing",
				{ ns: "admin" },
			);
		case "dev_only":
			return t(
				"exploreSkipDevOnly",
				"Only has packages, which need developer mode",
				{ ns: "admin" },
			);
		case "too_few_items":
			return t(
				"exploreSkipTooFewItems",
				"Too few visible items for this preview",
				{ ns: "admin" },
			);
		case "empty":
			return t("exploreSkipEmpty", "Nothing to show for this preview", {
				ns: "admin",
			});
	}
}

export function changeLabel(
	changes: readonly ExploreChange[],
	t: AdminT,
): string {
	return changes.length
		? t("exploreUnpublishedChanges", "{{count}} unpublished changes", {
				ns: "admin",
				count: changes.length,
			})
		: t("exploreAllChangesLive", "All changes live", { ns: "admin" });
}

export function placementSummary(
	placement: ExplorePlacement | ExplorePlacementInput,
	t: AdminT,
	nameOf: (item: ExplorePlacementItem) => string,
): string {
	const content = placement.content;
	switch (content.kind) {
		case "announcement":
			return content.dismissible
				? t("exploreSummaryAnnouncementDismissible", "{{tone}} · dismissible", {
						ns: "admin",
						tone: toneLabel(content.tone, t),
					})
				: t("exploreSummaryAnnouncement", "{{tone}} · stays until it ends", {
						ns: "admin",
						tone: toneLabel(content.tone, t),
					});
		case "spotlight":
			if (!placement.items.length) {
				return content.autoFill
					? t("exploreSummaryAutoFill", "Fills from Popular right now", {
							ns: "admin",
						})
					: t("exploreSummaryNoItems", "No items yet", { ns: "admin" });
			}
			return t(
				"exploreSummarySpotlight",
				"{{count}} items · every {{seconds}} s",
				{
					ns: "admin",
					count: placement.items.length,
					seconds: content.rotationSeconds,
				},
			);
		case "feature":
		case "sponsored": {
			const item = placement.items[0];
			const itemName = item
				? nameOf(item)
				: t("exploreSummaryNoItem", "No item yet", { ns: "admin" });
			return content.kind === "sponsored" && content.advertiser.trim()
				? `${content.advertiser.trim()} · ${itemName}`
				: itemName;
		}
		case "collection": {
			if (content.source === "rule") {
				return content.rule?.itemKind === "package"
					? t("exploreSummaryRulePackages", "Rule · packages", { ns: "admin" })
					: t("exploreSummaryRuleApps", "Rule · apps", { ns: "admin" });
			}
			return t("exploreSummaryCollection", "{{count}} hand-picked items", {
				ns: "admin",
				count: placement.items.length,
			});
		}
		case "rail":
			return railSummary(content.rail, t);
	}
}

/** The rotation meta; says how many slides the previewed viewer really gets when some are hidden from them. */
export function spotlightMeta(
	input: ExplorePlacementInput,
	visible: number | undefined,
	t: AdminT,
	nameOf: (item: ExplorePlacementItem) => string,
): string {
	const content = input.content;
	if (
		content.kind === "spotlight" &&
		visible !== undefined &&
		visible < input.items.length
	) {
		return t(
			"exploreSummarySpotlightVisible",
			"{{visible}} of {{total}} visible · every {{seconds}} s",
			{
				ns: "admin",
				visible,
				total: input.items.length,
				seconds: content.rotationSeconds,
			},
		);
	}
	return placementSummary(input, t, nameOf);
}

export function railSummary(rail: ExploreRailKey, t: AdminT): string {
	switch (rail) {
		case "trending":
			return t(
				"exploreRailSummaryTrending",
				"Apps portrait · packages in landscape pairs",
				{ ns: "admin" },
			);
		case "new":
			return t("exploreRailSummaryNew", "Published in the last 7 days", {
				ns: "admin",
			});
		case "new_count":
			return t("exploreRailSummaryNewCount", "Count tile · last 7 days", {
				ns: "admin",
			});
		case "top_paid":
			return t("exploreRailSummaryTopPaid", "Best sellers · 4 mini tiles", {
				ns: "admin",
			});
		case "for_builders":
			return t(
				"exploreRailSummaryForBuilders",
				"Packages only · verified first",
				{ ns: "admin" },
			);
		case "by_category":
			return t("exploreRailSummaryByCategory", "4 category tiles", {
				ns: "admin",
			});
		case "suites":
			return t("exploreRailSummarySuites", "Suites and platforms", {
				ns: "admin",
			});
	}
}

export function issueMessage(issue: PlacementIssue, t: AdminT): string {
	switch (issue.code) {
		case "required":
			return t("exploreIssueRequired", "This field is required.", {
				ns: "admin",
			});
		case "too_long":
			return t("exploreIssueTooLong", "Keep it to {{max}} characters.", {
				ns: "admin",
				max: issue.max,
			});
		case "https_url":
			return t(
				"exploreIssueHttpsUrl",
				"Use an https:// address without a user name or password.",
				{ ns: "admin" },
			);
		case "cta_href":
			return t(
				"exploreIssueCtaHref",
				"Use an app path such as /store/explore or an https:// address.",
				{ ns: "admin" },
			);
		case "cta_pair":
			return t(
				"exploreIssueCtaPair",
				"Fill in both the button label and the link, or neither.",
				{ ns: "admin" },
			);
		case "item_count":
			if (issue.max === 0) {
				return t("exploreIssueNoItems", "This placement takes no items.", {
					ns: "admin",
				});
			}
			return issue.min === issue.max
				? t("exploreIssueItemExact", "Pick exactly one item.", { ns: "admin" })
				: t("exploreIssueItemRange", "Pick {{min}} to {{max}} items.", {
						ns: "admin",
						min: issue.min,
						max: issue.max,
					});
		case "item_kind":
			return t(
				"exploreIssueItemKind",
				"This placement cannot hold this kind of item.",
				{ ns: "admin" },
			);
		case "duplicate_item":
			return t("exploreIssueDuplicateItem", "This item is listed twice.", {
				ns: "admin",
			});
		case "missing_collection":
			return t(
				"exploreIssueMissingCollection",
				"This collection is no longer part of the layout.",
				{ ns: "admin" },
			);
		case "window":
			return t("exploreIssueWindow", "The end must be after the start.", {
				ns: "admin",
			});
		case "audience_everyone":
			return t(
				"exploreIssueAudienceEveryone",
				"Everyone cannot be combined with other audiences.",
				{ ns: "admin" },
			);
		case "audience_sign_in":
			return t(
				"exploreIssueAudienceSignIn",
				"Pick signed in or signed out, not both.",
				{ ns: "admin" },
			);
		case "audience_unknown":
			return t(
				"exploreIssueAudienceUnknown",
				"This audience contains a tag the hub does not know.",
				{ ns: "admin" },
			);
		case "rotation":
			return t(
				"exploreIssueRotation",
				"Rotate every {{min}} to {{max}} seconds.",
				{ ns: "admin", min: issue.min, max: issue.max },
			);
		case "sponsored_enabled":
			return t(
				"exploreIssueSponsoredEnabled",
				"Sponsored placements cannot be turned on yet.",
				{ ns: "admin" },
			);
		case "rule_rating":
			return t("exploreIssueRuleRating", "Pick a minimum rating from 0 to 5.", {
				ns: "admin",
			});
		case "rule_limit":
			return t("exploreIssueRuleLimit", "Show {{min}} to {{max}} items.", {
				ns: "admin",
				min: issue.min,
				max: issue.max,
			});
	}
}
