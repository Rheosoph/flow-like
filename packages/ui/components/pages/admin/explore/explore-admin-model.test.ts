import { describe, expect, test } from "bun:test";
import {
	EXPLORE_LIMITS,
	type ExploreLayoutDoc,
	type ExplorePlacement,
	type ExplorePlacementContent,
	type ExplorePlacementInput,
	type ExploreRailKey,
	type ExploreSlot,
	type ExploreSlotKey,
	type ResolvedExplore,
} from "../../../store/explore/explore-types";
import {
	RAIL_KEYS,
	addChoicesFor,
	addTargets,
	addedPlacementId,
	adminPreviewPath,
	duplicateInput,
	fromLocalInput,
	moveTargets,
	newPlacementTemplate,
	newRowKey,
	orderAfterMove,
	orderWithNewRow,
	orderWithPriority,
	orderWithRows,
	orderWithoutRow,
	parseEditorState,
	parsePreview,
	patchItem,
	placementInput,
	previewCollection,
	previewSlideCount,
	referencingSpotlights,
	replaceKindItems,
	sameInput,
	slotAccepts,
	statusCounts,
	toLocalInput,
	toggleAudience,
	traceMarkers,
	validatePlacement,
} from "./explore-admin-model";

const rail = (key: ExploreRailKey): ExplorePlacementContent => ({
	kind: "rail",
	rail: key,
	title: null,
});
const CONTENT = {
	announcement: {
		kind: "announcement",
		tone: "info",
		title: "Hello",
		body: "",
		dismissible: true,
	},
	spotlight: { kind: "spotlight", rotationSeconds: 8, autoFill: true },
	feature: { kind: "feature", eyebrow: null },
	collection: {
		kind: "collection",
		title: "Picks",
		source: "hand",
		rule: null,
	},
	sponsored: { kind: "sponsored", advertiser: "Acme" },
} satisfies Record<string, ExplorePlacementContent>;

function placement(
	id: string,
	content: ExplorePlacementContent,
	extra: Partial<ExplorePlacement> = {},
): ExplorePlacement {
	return {
		id,
		kind: content.kind,
		name: id,
		enabled: true,
		audience: ["everyone"],
		content,
		items: [],
		...extra,
	};
}

function slot(
	key: ExploreSlotKey,
	placements: ExplorePlacement[],
	position = 0,
): ExploreSlot {
	const area = key.startsWith("row:")
		? "row"
		: key === "unplaced"
			? "unplaced"
			: "grid";
	return { key, area, position, placements };
}

function sampleLayout(): ExploreLayoutDoc {
	return {
		slots: [
			slot("hero", [
				placement("spot", CONTENT.spotlight, {
					items: [{ kind: "collection", id: "invoices" }],
				}),
			]),
			slot("notice", [placement("ann", CONTENT.announcement)], 1),
			slot(
				"feature",
				[
					placement("typst", CONTENT.feature, {
						items: [{ kind: "package", id: "typst" }],
					}),
					placement("research", CONTENT.feature, {
						items: [{ kind: "app", id: "research" }],
					}),
				],
				2,
			),
			slot(
				"collection",
				[
					placement("invoices", CONTENT.collection),
					placement("agents", CONTENT.collection),
				],
				3,
			),
			slot("stat", [placement("fresh", rail("new_count"))], 4),
			slot("categories", [placement("cats", rail("by_category"))], 5),
			slot("row:trending", [placement("trending", rail("trending"))], 0),
			slot("row:top-paid", [placement("toppaid", rail("top_paid"))], 1),
			slot(
				"row:builders",
				[
					placement("builders", rail("for_builders")),
					placement("new", rail("new")),
				],
				2,
			),
			slot("row:empty", [], 3),
			slot("unplaced", [
				placement("sponsor", CONTENT.sponsored, { enabled: false }),
			]),
		],
	};
}

function input(
	content: ExplorePlacementContent,
	extra: Partial<ExplorePlacementInput> = {},
): ExplorePlacementInput {
	return {
		name: "Name",
		enabled: false,
		startsAt: null,
		endsAt: null,
		audience: ["everyone"],
		content,
		items: [],
		...extra,
	};
}

const text = (length: number) => "x".repeat(length);
const codes = (value: ExplorePlacementInput) =>
	validatePlacement(value).map((issue) => `${issue.field}:${issue.code}`);

describe("slotAccepts (§2.3)", () => {
	const table: [string, ExplorePlacementContent, boolean][] = [
		["hero", CONTENT.spotlight, true],
		["hero", CONTENT.feature, false],
		["notice", CONTENT.announcement, true],
		["notice", CONTENT.spotlight, false],
		["feature", CONTENT.feature, true],
		["feature", CONTENT.collection, false],
		["collection", CONTENT.collection, true],
		["collection", rail("trending"), false],
		["stat", rail("new_count"), true],
		["stat", rail("trending"), false],
		["stat", rail("by_category"), false],
		["categories", rail("by_category"), true],
		["categories", rail("new_count"), false],
		["row:x", CONTENT.collection, true],
		["row:x", rail("trending"), true],
		["row:x", rail("new"), true],
		["row:x", rail("top_paid"), true],
		["row:x", rail("for_builders"), true],
		["row:x", rail("suites"), true],
		["row:x", rail("by_category"), false],
		["row:x", rail("new_count"), false],
		["row:x", CONTENT.spotlight, false],
		["row:x", CONTENT.sponsored, false],
		["unplaced", rail("new_count"), true],
		["unplaced", CONTENT.sponsored, true],
		["unplaced", CONTENT.feature, true],
		["row:Bad", CONTENT.collection, false],
		["row:", CONTENT.collection, false],
		["footer", CONTENT.collection, false],
	];
	for (const [key, content, expected] of table) {
		const label =
			content.kind === "rail" ? `rail/${content.rail}` : content.kind;
		test(`${key} ${expected ? "accepts" : "rejects"} ${label}`, () => {
			expect(slotAccepts(key, content)).toBe(expected);
		});
	}
});

describe("moveTargets", () => {
	const layout = sampleLayout();
	test("a feature goes to the feature slot and Unplaced only", () => {
		expect(moveTargets(layout, "typst")).toEqual(["unplaced"]);
		const parked = orderAfterMove(layout, "typst", "unplaced");
		expect(parked.slots.map((entry) => entry.key)).toEqual([
			"feature",
			"unplaced",
		]);
	});
	test("a collection goes to any row and Unplaced", () => {
		expect(moveTargets(layout, "invoices")).toEqual([
			"row:trending",
			"row:top-paid",
			"row:builders",
			"row:empty",
			"unplaced",
		]);
	});
	test("a trending rail goes to other rows and Unplaced, never stat", () => {
		const targets = moveTargets(layout, "trending");
		expect(targets).toEqual([
			"row:top-paid",
			"row:builders",
			"row:empty",
			"unplaced",
		]);
		expect(targets).not.toContain("stat");
	});
	test("by_category never goes to a row", () => {
		expect(moveTargets(layout, "cats")).toEqual(["unplaced"]);
	});
	test("an unknown id has no targets", () => {
		expect(moveTargets(layout, "nope")).toEqual([]);
	});
});

describe("order bodies", () => {
	const layout = sampleLayout();
	const rows = ["row:trending", "row:top-paid", "row:builders", "row:empty"];

	test("orderAfterMove lists both the source and the target slot in full", () => {
		expect(orderAfterMove(layout, "new", "row:trending")).toEqual({
			rows,
			slots: [
				{ key: "row:builders", placementIds: ["builders"] },
				{ key: "row:trending", placementIds: ["trending", "new"] },
			],
		});
	});
	test("orderAfterMove inserts at an index and creates a new row", () => {
		expect(orderAfterMove(layout, "trending", "row:top-paid", 0).slots).toEqual(
			[
				{ key: "row:trending", placementIds: [] },
				{ key: "row:top-paid", placementIds: ["trending", "toppaid"] },
			],
		);
		const created = orderAfterMove(layout, "agents", "row:new-one");
		expect(created.rows).toEqual([...rows, "row:new-one"]);
		expect(created.slots).toEqual([
			{ key: "collection", placementIds: ["invoices"] },
			{ key: "row:new-one", placementIds: ["agents"] },
		]);
	});
	test("orderAfterMove refuses incompatible, same-slot and unknown moves", () => {
		expect(() => orderAfterMove(layout, "trending", "stat")).toThrow(
			"stat does not accept rail/trending",
		);
		expect(() => orderAfterMove(layout, "cats", "row:empty")).toThrow();
		expect(() => orderAfterMove(layout, "ann", "notice")).toThrow();
		expect(() => orderAfterMove(layout, "ghost", "unplaced")).toThrow();
		expect(orderAfterMove(layout, "fresh", "unplaced").slots).toEqual([
			{ key: "stat", placementIds: [] },
			{ key: "unplaced", placementIds: ["sponsor", "fresh"] },
		]);
	});
	test("orderWithPriority needs a permutation of the slot", () => {
		expect(orderWithPriority(layout, "feature", ["research", "typst"])).toEqual(
			{
				rows,
				slots: [{ key: "feature", placementIds: ["research", "typst"] }],
			},
		);
		expect(() => orderWithPriority(layout, "feature", ["research"])).toThrow();
		expect(() =>
			orderWithPriority(layout, "feature", ["research", "research"]),
		).toThrow();
	});
	test("row order, new rows and removing an empty row", () => {
		expect(orderWithRows(layout, [...rows].reverse())).toEqual({
			rows: [...rows].reverse(),
			slots: [],
		});
		expect(() => orderWithRows(layout, rows.slice(1))).toThrow();
		expect(orderWithNewRow(layout)).toEqual({
			rows: [...rows, "row:row-1"],
			slots: [],
		});
		expect(orderWithoutRow(layout, "row:empty")).toEqual({
			rows: rows.slice(0, 3),
			slots: [],
		});
		expect(() => orderWithoutRow(layout, "row:builders")).toThrow(
			"still has placements",
		);
		expect(() => orderWithoutRow(layout, "hero")).toThrow();
	});
	test("newRowKey skips taken keys", () => {
		const taken: ExploreLayoutDoc = {
			slots: [slot("row:row-1", []), slot("row:row-2", [])],
		};
		expect(newRowKey(taken)).toBe("row:row-3");
	});
});

describe("templates and add targets", () => {
	test("announcement, spotlight, collection and rail templates are valid for POST", () => {
		const cases: [ExplorePlacementInput, ExploreSlotKey][] = [
			[newPlacementTemplate("announcement", "notice", "News"), "notice"],
			[newPlacementTemplate("spotlight", "hero", "Spot"), "hero"],
			[newPlacementTemplate("collection", "collection", "Picks"), "collection"],
			[newPlacementTemplate("rail", "stat", "Count"), "stat"],
			[newPlacementTemplate("rail", "categories", "Cats"), "categories"],
			[newPlacementTemplate("rail", "row:x", "Trending"), "row:x"],
		];
		for (const [template, key] of cases) {
			expect(validatePlacement(template)).toEqual([]);
			expect(template.enabled).toBe(false);
			expect("kind" in template).toBe(false);
			expect(slotAccepts(key, template.content)).toBe(true);
		}
	});
	test("feature and sponsored templates need an item first", () => {
		expect(codes(newPlacementTemplate("feature", "feature", "F"))).toEqual([
			"items:item_count",
		]);
		expect(codes(newPlacementTemplate("sponsored", "unplaced", "S"))).toEqual([
			"advertiser:required",
			"items:item_count",
		]);
	});
	test("the Add menu offers only compatible slots", () => {
		const layout = sampleLayout();
		const summary = (entry: Parameters<typeof addTargets>[1]) =>
			addTargets(layout, entry).map(
				(target) =>
					`${target.slot}${target.newRow ? "+" : ""}=${target.choices
						.map((choice) => choice.rail ?? choice.kind)
						.join(",")}`,
			);
		expect(summary("announcement")).toEqual([
			"notice=announcement",
			"unplaced=announcement",
		]);
		expect(summary("spotlight")).toEqual([
			"hero=spotlight",
			"feature=feature",
			"unplaced=spotlight,feature",
		]);
		expect(summary("sponsored")).toEqual(["unplaced=sponsored"]);
		const rows = "trending,new,top_paid,for_builders,suites";
		expect(summary("rail")).toEqual([
			"stat=new_count",
			"categories=by_category",
			`row:trending=${rows}`,
			`row:top-paid=${rows}`,
			`row:builders=${rows}`,
			`row:empty=${rows}`,
			`row:row-1+=${rows}`,
			`unplaced=${RAIL_KEYS.join(",")}`,
		]);
		expect(summary("collection")).toEqual([
			"collection=collection",
			"row:trending=collection",
			"row:top-paid=collection",
			"row:builders=collection",
			"row:empty=collection",
			"row:row-1+=collection",
			"unplaced=collection",
		]);
	});
	test("an empty slot offers only what it accepts", () => {
		const layout = sampleLayout();
		const offer = (key: ExploreSlotKey) =>
			addChoicesFor(layout, key).map((choice) => choice.rail ?? choice.kind);
		expect(offer("hero")).toEqual(["spotlight"]);
		expect(offer("feature")).toEqual(["feature"]);
		expect(offer("stat")).toEqual(["new_count"]);
		expect(offer("categories")).toEqual(["by_category"]);
		expect(offer("row:empty")).toEqual([
			"collection",
			"trending",
			"new",
			"top_paid",
			"for_builders",
			"suites",
		]);
	});
	test("the Add menu is empty at the placement cap and offers no new row at the row cap", () => {
		const full: ExploreLayoutDoc = {
			slots: [
				slot(
					"unplaced",
					Array.from({ length: EXPLORE_LIMITS.placements }, (_, index) =>
						placement(`p${index}`, CONTENT.announcement),
					),
				),
			],
		};
		expect(addTargets(full, "announcement")).toEqual([]);
		const rows: ExploreLayoutDoc = {
			slots: Array.from({ length: EXPLORE_LIMITS.rows }, (_, index) =>
				slot(`row:r${index}`, [], index),
			),
		};
		expect(addTargets(rows, "collection").some((target) => target.newRow)).toBe(
			false,
		);
	});
});

describe("duplicate, input and references", () => {
	test("duplicateInput drops kind, disables and suffixes the name", () => {
		const source = placement("typst", CONTENT.feature, {
			name: "Typst Documents",
			status: "live",
			updatedAt: "2026-09-24T00:00:00Z",
			items: [{ kind: "package", id: "typst", headline: "Hi", accent: null }],
		});
		const copy = duplicateInput(source);
		expect(copy).toEqual({
			name: "Typst Documents (copy)",
			enabled: false,
			startsAt: null,
			endsAt: null,
			audience: ["everyone"],
			content: CONTENT.feature,
			items: [{ kind: "package", id: "typst", headline: "Hi" }],
		});
		expect("kind" in copy).toBe(false);
		expect("id" in copy).toBe(false);
		expect("status" in copy).toBe(false);
		const long = duplicateInput(
			placement("long", CONTENT.feature, { name: text(80) }),
		);
		expect(Array.from(long.name)).toHaveLength(EXPLORE_LIMITS.name);
		expect(long.name.endsWith(" (copy)")).toBe(true);
		expect(duplicateInput(source, " (Kopie)").name).toBe(
			"Typst Documents (Kopie)",
		);
	});
	test("placementInput carries only the input fields", () => {
		const body = placementInput(
			placement("x", CONTENT.announcement, { status: "draft" }),
		);
		expect(Object.keys(body).sort()).toEqual([
			"audience",
			"content",
			"enabled",
			"endsAt",
			"items",
			"name",
			"startsAt",
		]);
	});
	test("sameInput ignores the server's normalization but not real edits", () => {
		const local = input(
			{
				...CONTENT.announcement,
				title: "Hello ",
				ctaLabel: "",
				ctaHref: undefined,
			} as ExplorePlacementContent,
			{ startsAt: "2026-10-01T10:00:00.000Z" },
		);
		const echoed = input(
			{
				...CONTENT.announcement,
				title: "Hello",
				ctaLabel: null,
				ctaHref: null,
			} as ExplorePlacementContent,
			{ startsAt: "2026-10-01T10:00:00Z" },
		);
		expect(sameInput(local, echoed)).toBe(true);
		expect(sameInput(local, { ...echoed, name: "Other" })).toBe(false);
		const hand = input({
			...CONTENT.collection,
			rule: {
				itemKind: "app",
				verifiedOnly: false,
				sort: "installs",
				limit: 8,
			},
		});
		expect(sameInput(hand, input(CONTENT.collection))).toBe(true);
	});
	test("referencingSpotlights finds slides that point at a collection", () => {
		const layout = sampleLayout();
		expect(
			referencingSpotlights(layout, "invoices").map((entry) => entry.id),
		).toEqual(["spot"]);
		expect(referencingSpotlights(layout, "agents")).toEqual([]);
	});
	test("addedPlacementId finds the new placement in the target slot", () => {
		const before = sampleLayout();
		const after = sampleLayout();
		after.slots[1].placements.push(
			placement("fresh-ann", CONTENT.announcement),
		);
		expect(addedPlacementId(before, after, "notice")).toBe("fresh-ann");
		expect(addedPlacementId(before, before, "notice")).toBeUndefined();
	});
	test("replaceKindItems keeps overrides and other kinds", () => {
		const items = [
			{ kind: "app" as const, id: "a", headline: "A!" },
			{ kind: "package" as const, id: "p" },
			{ kind: "app" as const, id: "b" },
		];
		expect(replaceKindItems(items, "app", ["a", "c"])).toEqual([
			{ kind: "app", id: "a", headline: "A!" },
			{ kind: "package", id: "p" },
			{ kind: "app", id: "c" },
		]);
		expect(replaceKindItems(items, "package", [])).toEqual([
			{ kind: "app", id: "a", headline: "A!" },
			{ kind: "app", id: "b" },
		]);
	});
});

describe("audience chips", () => {
	test("everyone is exclusive", () => {
		expect(toggleAudience(["dev", "web"], "everyone")).toEqual(["everyone"]);
		expect(toggleAudience(["everyone"], "dev")).toEqual(["dev"]);
		expect(toggleAudience(["dev"], "web")).toEqual(["dev", "web"]);
	});
	test("removing the last tag falls back to everyone", () => {
		expect(toggleAudience(["dev"], "dev")).toEqual(["everyone"]);
	});
	test("signed in and signed out replace each other", () => {
		expect(toggleAudience(["signed_in", "web"], "signed_out")).toEqual([
			"web",
			"signed_out",
		]);
		expect(toggleAudience(["signed_out"], "signed_in")).toEqual(["signed_in"]);
	});
});

describe("traceMarkers", () => {
	test("marks chosen fallbacks and preview-only skips from the server trace", () => {
		const markers = traceMarkers(
			[
				{
					slotKey: "feature",
					chosen: "research",
					skipped: [{ placementId: "typst", reason: "dev_only" }],
				},
				{
					slotKey: "notice",
					chosen: null,
					skipped: [{ placementId: "ann", reason: "audience" }],
				},
				{
					slotKey: "row:builders",
					chosen: "builders",
					skipped: [{ placementId: "new", reason: "draft" }],
				},
				{
					slotKey: "collection",
					chosen: "agents",
					skipped: [{ placementId: "invoices", reason: "too_few_items" }],
				},
			],
			sampleLayout(),
		);
		expect([...markers.fallback].sort()).toEqual(["agents", "research"]);
		expect(Object.fromEntries(markers.hidden)).toEqual({
			typst: "dev_only",
			ann: "audience",
			invoices: "too_few_items",
		});
		expect(markers.chosen.get("notice")).toBeNull();
		expect(markers.chosen.get("row:builders")).toBe("builders");
	});
});

describe("preview lookups", () => {
	const collection = (placementId: string, apps: number, packages: number) => ({
		placementId,
		title: placementId,
		source: "rule" as const,
		items: [],
		apps,
		packages,
	});
	const page = {
		revision: "r1",
		generatedAt: "2026-09-24T10:00:00Z",
		viewer: { dev: true, signedIn: true, platform: "web", language: "en" },
		typeCounts: { apps: 3, packages: 2 },
		views: {
			all: {
				grid: {
					hero: {
						placementId: "hero",
						rotationSeconds: 8,
						slides: [
							{
								item: { kind: "collection", collection: collection("c", 1, 1) },
							},
						],
					},
					collection: collection("grid-picks", 4, 0),
				},
				rows: [
					{ kind: "collection", collection: collection("row-picks", 2, 3) },
				],
			},
		},
	} as unknown as ResolvedExplore;

	test("previewSlideCount only answers for the spotlight the preview shows", () => {
		expect(previewSlideCount(page, "hero")).toBe(1);
		expect(previewSlideCount(page, "other")).toBeUndefined();
		expect(previewSlideCount(undefined, "hero")).toBeUndefined();
	});

	test("previewCollection finds grid and row collections", () => {
		expect(previewCollection(page, "grid-picks")?.apps).toBe(4);
		expect(previewCollection(page, "row-picks")?.packages).toBe(3);
		expect(previewCollection(page, "missing")).toBeUndefined();
		expect(previewCollection(undefined, "row-picks")).toBeUndefined();
	});
});

describe("patchItem", () => {
	const items = [
		{ kind: "app" as const, id: "a", headline: "A" },
		{ kind: "package" as const, id: "p" },
	];

	test("patches the item at its index", () => {
		expect(
			patchItem(items, items[1], 1, { artworkUrl: "https://x/p.webp" }),
		).toEqual([
			items[0],
			{ kind: "package", id: "p", artworkUrl: "https://x/p.webp" },
		]);
	});

	test("follows the item after the list was reordered and keeps newer edits", () => {
		const reordered = [{ ...items[1], headline: "New" }, items[0]];
		expect(patchItem(reordered, items[1], 1, { artworkUrl: "u" })).toEqual([
			{ kind: "package", id: "p", headline: "New", artworkUrl: "u" },
			items[0],
		]);
	});

	test("does nothing once the item was removed", () => {
		expect(patchItem([items[0]], items[1], 1, { artworkUrl: "u" })).toEqual([
			items[0],
		]);
	});
});

describe("validatePlacement mirrors EXPLORE_LIMITS", () => {
	const announcement = (
		patch: Partial<Extract<ExplorePlacementContent, { kind: "announcement" }>>,
	) => input({ ...CONTENT.announcement, ...patch } as ExplorePlacementContent);

	test("text limits at the boundary and one past", () => {
		const cases: [(length: number) => ExplorePlacementInput, number, string][] =
			[
				(() => {
					const build = (length: number) =>
						input(CONTENT.announcement, { name: text(length) });
					return [build, EXPLORE_LIMITS.name, "name"];
				})(),
				[
					(length) => announcement({ title: text(length) }),
					EXPLORE_LIMITS.announcementTitle,
					"title",
				],
				[
					(length) => announcement({ body: text(length) }),
					EXPLORE_LIMITS.announcementBody,
					"body",
				],
				[
					(length) => announcement({ ctaLabel: text(length), ctaHref: "/x" }),
					EXPLORE_LIMITS.ctaLabel,
					"ctaLabel",
				],
				[
					(length) =>
						input(
							{ ...CONTENT.collection, title: text(length) },
							{
								items: [
									{ kind: "app", id: "a" },
									{ kind: "app", id: "b" },
								],
							},
						),
					EXPLORE_LIMITS.collectionTitle,
					"title",
				],
				[
					(length) =>
						input(
							{ ...CONTENT.collection, blurb: text(length) },
							{
								items: [
									{ kind: "app", id: "a" },
									{ kind: "app", id: "b" },
								],
							},
						),
					EXPLORE_LIMITS.blurb,
					"blurb",
				],
				[
					(length) =>
						input({
							...rail("trending"),
							title: text(length),
						} as ExplorePlacementContent),
					EXPLORE_LIMITS.railTitle,
					"title",
				],
				[
					(length) =>
						input(
							{ kind: "sponsored", advertiser: text(length) },
							{
								items: [{ kind: "app", id: "a" }],
							},
						),
					EXPLORE_LIMITS.advertiser,
					"advertiser",
				],
				[
					(length) =>
						input(
							{ kind: "feature", eyebrow: text(length) },
							{
								items: [{ kind: "app", id: "a" }],
							},
						),
					EXPLORE_LIMITS.eyebrow,
					"eyebrow",
				],
				[
					(length) =>
						input(CONTENT.feature, {
							items: [{ kind: "app", id: "a", headline: text(length) }],
						}),
					EXPLORE_LIMITS.headline,
					"items.0.headline",
				],
				[
					(length) =>
						input(CONTENT.feature, {
							items: [{ kind: "app", id: "a", subline: text(length) }],
						}),
					EXPLORE_LIMITS.subline,
					"items.0.subline",
				],
			];
		for (const [build, max, field] of cases) {
			expect(codes(build(max))).toEqual([]);
			expect(codes(build(max + 1))).toEqual([`${field}:too_long`]);
		}
	});
	test("item counts per kind", () => {
		const apps = (count: number) =>
			Array.from({ length: count }, (_, index) => ({
				kind: "app" as const,
				id: `a${index}`,
			}));
		const spot = (count: number, autoFill: boolean) =>
			input(
				{ kind: "spotlight", rotationSeconds: 8, autoFill },
				{
					items: apps(count),
				},
			);
		expect(codes(spot(0, true))).toEqual([]);
		expect(codes(spot(0, false))).toEqual(["items:item_count"]);
		expect(codes(spot(EXPLORE_LIMITS.spotlightItems, false))).toEqual([]);
		expect(codes(spot(EXPLORE_LIMITS.spotlightItems + 1, true))).toEqual([
			"items:item_count",
		]);
		const hand = (count: number) =>
			input(CONTENT.collection, { items: apps(count) });
		expect(codes(hand(1))).toEqual(["items:item_count"]);
		expect(codes(hand(2))).toEqual([]);
		expect(codes(hand(EXPLORE_LIMITS.collectionItems))).toEqual([]);
		expect(codes(hand(EXPLORE_LIMITS.collectionItems + 1))).toEqual([
			"items:item_count",
		]);
		const rule = (count: number) =>
			input(
				{
					...CONTENT.collection,
					source: "rule",
					rule: {
						itemKind: "app",
						verifiedOnly: false,
						sort: "installs",
						limit: 8,
					},
				},
				{ items: apps(count) },
			);
		expect(codes(rule(0))).toEqual([]);
		expect(codes(rule(EXPLORE_LIMITS.rulePinned))).toEqual([]);
		expect(codes(rule(EXPLORE_LIMITS.rulePinned + 1))).toEqual([
			"items:item_count",
		]);
		expect(codes(input(CONTENT.feature, { items: apps(2) }))).toEqual([
			"items:item_count",
		]);
		expect(codes(input(rail("trending"), { items: apps(1) }))).toEqual([
			"items:item_count",
		]);
	});
	test("item kinds, duplicates and collection references", () => {
		expect(
			codes(
				input(CONTENT.feature, { items: [{ kind: "collection", id: "c" }] }),
			),
		).toEqual(["items.0:item_kind"]);
		expect(
			codes(
				input(CONTENT.collection, {
					items: [
						{ kind: "app", id: "a" },
						{ kind: "app", id: "a" },
					],
				}),
			),
		).toEqual(["items.1:duplicate_item"]);
		const slides = input(
			{ kind: "spotlight", rotationSeconds: 8 },
			{ items: [{ kind: "collection", id: "gone" }] },
		);
		expect(
			validatePlacement(slides, new Set(["invoices"])).map(
				(issue) => issue.code,
			),
		).toEqual(["missing_collection"]);
	});
	test("urls, cta pairs and rotation", () => {
		expect(codes(announcement({ ctaLabel: "Go", ctaHref: "/store" }))).toEqual(
			[],
		);
		expect(
			codes(announcement({ ctaLabel: "Go", ctaHref: "https://example.com" })),
		).toEqual([]);
		for (const href of [
			"//evil.com",
			"javascript:alert(1)",
			"http://x.com",
			"/a b",
		]) {
			expect(codes(announcement({ ctaLabel: "Go", ctaHref: href }))).toEqual([
				"ctaHref:cta_href",
			]);
		}
		expect(codes(announcement({ ctaLabel: "Go" }))).toEqual([
			"ctaHref:cta_pair",
		]);
		expect(codes(announcement({ ctaHref: "/x" }))).toEqual([
			"ctaLabel:cta_pair",
		]);
		expect(codes(announcement({ imageUrl: "http://x.com/a.png" }))).toEqual([
			"imageUrl:https_url",
		]);
		expect(
			codes(announcement({ imageUrl: "https://user:pw@x.com/a.png" })),
		).toEqual(["imageUrl:https_url"]);
		const rotation = (rotationSeconds: number) =>
			codes(input({ kind: "spotlight", rotationSeconds, autoFill: true }));
		expect(rotation(EXPLORE_LIMITS.rotationMin)).toEqual([]);
		expect(rotation(EXPLORE_LIMITS.rotationMax)).toEqual([]);
		expect(rotation(EXPLORE_LIMITS.rotationMin - 1)).toEqual([
			"rotationSeconds:rotation",
		]);
		expect(rotation(EXPLORE_LIMITS.rotationMax + 1)).toEqual([
			"rotationSeconds:rotation",
		]);
	});
	test("schedule, audience and sponsored", () => {
		expect(
			codes(
				input(CONTENT.announcement, {
					startsAt: "2026-10-02T00:00:00Z",
					endsAt: "2026-10-01T00:00:00Z",
				}),
			),
		).toEqual(["schedule:window"]);
		expect(
			codes(input(CONTENT.announcement, { audience: ["everyone", "dev"] })),
		).toEqual(["audience:audience_everyone"]);
		expect(
			codes(
				input(CONTENT.announcement, { audience: ["signed_in", "signed_out"] }),
			),
		).toEqual(["audience:audience_sign_in"]);
		expect(
			codes(input(CONTENT.announcement, { audience: ["locale:xx"] })),
		).toEqual(["audience:audience_unknown"]);
		expect(
			codes(input(CONTENT.announcement, { audience: ["locale:pt-br", "web"] })),
		).toEqual([]);
		expect(
			codes(
				input(CONTENT.sponsored, {
					enabled: true,
					items: [{ kind: "app", id: "a" }],
				}),
			),
		).toEqual(["enabled:sponsored_enabled"]);
	});
});

describe("status, schedule and wire helpers", () => {
	test("statusCounts derives from enabled and the window", () => {
		const layout: ExploreLayoutDoc = {
			slots: [
				slot("unplaced", [
					placement("a", CONTENT.announcement),
					placement("b", CONTENT.announcement, { enabled: false }),
					placement("c", CONTENT.announcement, {
						startsAt: "2030-01-01T00:00:00Z",
					}),
					placement("d", CONTENT.announcement, {
						endsAt: "2020-01-01T00:00:00Z",
					}),
				]),
			],
		};
		expect(statusCounts(layout, "2026-09-24T00:00:00Z")).toEqual({
			live: 1,
			scheduled: 1,
			draft: 1,
			ended: 1,
		});
	});
	test("datetime-local values round-trip through ISO UTC", () => {
		expect(fromLocalInput("")).toBeNull();
		expect(fromLocalInput("not a date")).toBeNull();
		const iso = fromLocalInput("2026-10-11T18:00");
		expect(iso?.endsWith("Z")).toBe(true);
		expect(toLocalInput(iso)).toBe("2026-10-11T18:00");
		expect(toLocalInput(null)).toBe("");
	});
	test("parseEditorState rejects bodies that are not an editor state", () => {
		expect(() => parseEditorState("<html>")).toThrow('text "<html>"');
		expect(() => parseEditorState(null)).toThrow("null");
		expect(() => parseEditorState({ layout: [] })).toThrow();
	});
	test("parseEditorState keeps known slots and drops malformed entries", () => {
		const state = parseEditorState({
			draftRevision: "r1",
			liveRevision: null,
			now: "2026-09-24T00:00:00Z",
			layout: {
				slots: [
					{
						key: "hero",
						area: "grid",
						position: 0,
						placements: [
							{
								id: "spot",
								kind: "spotlight",
								name: "Spot",
								enabled: true,
								audience: ["everyone", 3],
								content: { kind: "spotlight", rotationSeconds: 8 },
								items: [{ kind: "app", id: "a", bogus: 1 }, { id: "x" }],
								status: "live",
							},
							{ id: "broken" },
						],
					},
					{ key: "footer", placements: [] },
					"junk",
				],
			},
			refs: [{ kind: "app", id: "a", name: "A", public: true, exists: true }],
			changes: [{ placementId: "spot", name: "Spot", change: "added" }],
			warnings: ["x", 1],
		});
		expect(state.layout.slots.map((entry) => entry.key)).toEqual(["hero"]);
		expect(state.layout.slots[0].placements).toHaveLength(1);
		expect(state.layout.slots[0].placements[0].items).toEqual([
			{ kind: "app", id: "a" },
		]);
		expect(state.layout.slots[0].placements[0].audience).toEqual(["everyone"]);
		expect(state.changes).toHaveLength(1);
		expect(state.warnings).toEqual(["x"]);
	});
	test("parsePreview rejects a non-object body", () => {
		expect(() => parsePreview("<html>")).toThrow();
	});
	test("adminPreviewPath simulates the whole viewer", () => {
		expect(
			adminPreviewPath({
				dev: true,
				signedIn: false,
				platform: "web",
				language: "de",
			}),
		).toBe(
			"admin/explore/preview?source=draft&dev=true&signed_in=false&platform=web&language=de",
		);
	});
});
