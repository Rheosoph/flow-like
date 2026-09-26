import { describe, expect, test } from "bun:test";
import { ApiResponseError } from "../../../lib/api-error";
import { categoryColor } from "../../../lib/category-meta";
import { IAppCategory } from "../../../lib/schema/app/app";
import {
	FIXTURE_APPS,
	FIXTURE_PACKAGES,
	exploreFixture,
	exploreSearchFixture,
} from "./explore-fixture";
import {
	type BentoArea,
	accentColor,
	appCategoryForPackage,
	audienceMatches,
	bentoPlacement,
	bentoRows,
	createDismissStorage,
	exploreRequestPath,
	exploreSearchRequestPath,
	isExploreCtaHref,
	isExploreHttpsUrl,
	isExploreUnsupportedError,
	layoutTopPaid,
	layoutTrending,
	parseExploreSearch,
	parseResolvedExplore,
	pickExploreView,
	placementStatus,
	toExploreError,
} from "./explore-model";
import {
	BENTO_ROW_HEIGHTS,
	BENTO_SLOTS,
	EXPLORE_SEARCH_LIMITS,
	type ExploreGridSlotKey,
	type ExploreResolvedItem,
	ExploreUnsupportedError,
	type ExploreView,
	type ExploreViewer,
	GRID_SLOT_KEYS,
} from "./explore-types";
import {
	exploreRetry,
	exploreSupportFromQuery,
	exploreViewerSignedIn,
} from "./use-explore";

const A = FIXTURE_APPS;
const P = FIXTURE_PACKAGES;

function itemId(item: ExploreResolvedItem): string {
	if (item.kind === "app") return `app:${item.app.id}`;
	if (item.kind === "package") return `package:${item.package.id}`;
	return `collection:${item.collection.id}`;
}

function viewIds(view: ExploreView) {
	const { grid } = view;
	return {
		hero: grid.hero?.slides.map((slide) => itemId(slide.item)) ?? null,
		notice: grid.notice?.dismissKey ?? null,
		feature: grid.feature ? itemId(grid.feature.slide.item) : null,
		collection: grid.collection?.items.map(itemId) ?? null,
		stat: grid.stat?.stat ?? null,
		categories: grid.categories?.categories ?? null,
		rows: view.rows.map((row) =>
			row.kind === "rail"
				? `${row.rail.rail}[${row.rail.items.map(itemId).join(",")}]`
				: `collection:${row.collection.placementId}`,
		),
	};
}

function roundTripJson<T>(value: T): unknown {
	return JSON.parse(JSON.stringify(value));
}

const area = (
	col: number,
	span: number,
	row: number,
	rowSpan: number,
): BentoArea => ({ col, span, row, rowSpan });

describe("placementStatus", () => {
	const window = {
		enabled: true,
		startsAt: "2026-09-24T10:00:00.000Z",
		endsAt: "2026-09-25T10:00:00.000Z",
	};

	test("disabled is a draft whatever the window", () => {
		expect(
			placementStatus({ ...window, enabled: false }, "2026-09-24T12:00:00Z"),
		).toBe("draft");
	});

	test("the window boundaries", () => {
		expect(placementStatus(window, "2026-09-24T09:59:59.999Z")).toBe(
			"scheduled",
		);
		expect(placementStatus(window, "2026-09-24T10:00:00.000Z")).toBe("live");
		expect(placementStatus(window, "2026-09-25T09:59:59.999Z")).toBe("live");
		expect(placementStatus(window, "2026-09-25T10:00:00.000Z")).toBe("ended");
	});

	test("no window is live", () => {
		expect(placementStatus({ enabled: true }, Date.now())).toBe("live");
		expect(
			placementStatus({ enabled: true, startsAt: null, endsAt: null }, 0),
		).toBe("live");
	});
});

describe("audienceMatches", () => {
	const viewer: ExploreViewer = {
		dev: false,
		signedIn: true,
		platform: "desktop",
		language: "de-AT",
	};

	test("empty and everyone match anyone", () => {
		expect(audienceMatches([], viewer)).toBe(true);
		expect(audienceMatches(["everyone"], viewer)).toBe(true);
	});

	test("dev requires developer mode", () => {
		expect(audienceMatches(["dev"], viewer)).toBe(false);
		expect(audienceMatches(["dev"], { ...viewer, dev: true })).toBe(true);
	});

	test("signed in and signed out", () => {
		expect(audienceMatches(["signed_in"], viewer)).toBe(true);
		expect(audienceMatches(["signed_out"], viewer)).toBe(false);
		expect(
			audienceMatches(["signed_out"], { ...viewer, signedIn: false }),
		).toBe(true);
		expect(audienceMatches(["signed_in", "signed_out"], viewer)).toBe(false);
		expect(
			audienceMatches(["signed_in", "signed_out"], {
				...viewer,
				signedIn: false,
			}),
		).toBe(false);
	});

	test("platforms OR within the dimension", () => {
		expect(audienceMatches(["desktop"], viewer)).toBe(true);
		expect(audienceMatches(["web"], viewer)).toBe(false);
		expect(audienceMatches(["web", "desktop"], viewer)).toBe(true);
	});

	test("locales match equal or region-tagged languages, case-insensitively", () => {
		expect(audienceMatches(["locale:de"], viewer)).toBe(true);
		expect(audienceMatches(["locale:de"], { ...viewer, language: "de" })).toBe(
			true,
		);
		expect(audienceMatches(["locale:DE"], { ...viewer, language: "de" })).toBe(
			true,
		);
		expect(audienceMatches(["locale:de"], { ...viewer, language: "en" })).toBe(
			false,
		);
		expect(audienceMatches(["locale:de"], { ...viewer, language: "deu" })).toBe(
			false,
		);
		expect(
			audienceMatches(["locale:pt-BR"], { ...viewer, language: "pt-br" }),
		).toBe(true);
		expect(audienceMatches(["locale:en", "locale:de"], viewer)).toBe(true);
	});

	test("dimensions AND together", () => {
		expect(audienceMatches(["desktop", "locale:de", "signed_in"], viewer)).toBe(
			true,
		);
		expect(audienceMatches(["desktop", "locale:de", "dev"], viewer)).toBe(
			false,
		);
		expect(audienceMatches(["web", "locale:de"], viewer)).toBe(false);
	});

	test("unknown tags fail closed", () => {
		expect(audienceMatches(["beta"], viewer)).toBe(false);
		expect(audienceMatches(["locale:"], viewer)).toBe(false);
		expect(audienceMatches(["desktop", "admins"], viewer)).toBe(false);
	});
});

describe("pickExploreView", () => {
	const page = exploreFixture({ dev: true });

	test("each type picks its server-resolved view", () => {
		expect(pickExploreView(page, "all")).toBe(page.views.all);
		expect(pickExploreView(page, "apps")).toBe(page.views.apps as ExploreView);
		expect(pickExploreView(page, "packages")).toBe(
			page.views.packages as ExploreView,
		);
	});

	test("a missing view falls back to all", () => {
		const nonDev = exploreFixture({ dev: false });
		expect(nonDev.views.apps).toBeNull();
		expect(pickExploreView(nonDev, "apps")).toBe(nonDev.views.all);
		expect(pickExploreView(nonDev, "packages")).toBe(nonDev.views.all);
	});

	test("the apps view carries the fallbacks the all view does not", () => {
		const all = viewIds(pickExploreView(page, "all"));
		const apps = viewIds(pickExploreView(page, "apps"));
		expect(all.feature).toBe("package:typst-documents");
		expect(apps.feature).toBe("app:app-invoice-autopilot");
		expect(apps.collection).toEqual([
			"app:app-bounded-agent-starter",
			"app:app-research-assistant",
			"app:app-customer-support-copilot",
		]);
		expect(apps.rows.at(-1)).toStartWith("new[");
		expect(all.rows.at(-1)).toStartWith("for_builders[");
	});
});

describe("bentoPlacement", () => {
	function xlOf(present: ExploreGridSlotKey[]) {
		return Object.fromEntries(
			Object.entries(bentoPlacement(present)).map(([slot, tile]) => [
				slot,
				tile.xl,
			]),
		);
	}

	test("top band collapse table", () => {
		expect(xlOf(["hero", "notice", "feature"])).toEqual({
			hero: area(1, 7, 1, 2),
			notice: area(8, 5, 1, 1),
			feature: area(8, 5, 2, 1),
		});
		expect(xlOf(["hero", "notice"])).toEqual({
			hero: area(1, 7, 1, 2),
			notice: area(8, 5, 1, 2),
		});
		expect(xlOf(["hero", "feature"])).toEqual({
			hero: area(1, 7, 1, 2),
			feature: area(8, 5, 1, 2),
		});
		expect(xlOf(["hero"])).toEqual({ hero: area(1, 12, 1, 2) });
		expect(xlOf(["notice", "feature"])).toEqual({
			notice: area(1, 6, 1, 2),
			feature: area(7, 6, 1, 2),
		});
		expect(xlOf(["notice"])).toEqual({ notice: area(1, 12, 1, 1) });
		expect(xlOf(["feature"])).toEqual({ feature: area(1, 12, 1, 1) });
	});

	test("feature size follows the band", () => {
		expect(bentoPlacement(["hero", "notice", "feature"]).feature?.size).toBe(
			"short",
		);
		expect(bentoPlacement(["hero", "feature"]).feature?.size).toBe("tall");
		expect(bentoPlacement(["notice", "feature"]).feature?.size).toBe("tall");
		expect(bentoPlacement(["feature"]).feature?.size).toBe("short");
	});

	test("lower band collapse table sits below a full top band", () => {
		const top: ExploreGridSlotKey[] = ["hero", "notice", "feature"];
		const lower = (slots: ExploreGridSlotKey[]) => {
			const xl = xlOf([...top, ...slots]);
			return Object.fromEntries(slots.map((slot) => [slot, xl[slot]]));
		};
		expect(lower(["collection", "stat", "categories"])).toEqual({
			collection: area(1, 6, 3, 2),
			stat: area(7, 2, 3, 2),
			categories: area(9, 4, 3, 2),
		});
		expect(lower(["collection", "categories"])).toEqual({
			collection: area(1, 6, 3, 2),
			categories: area(7, 6, 3, 2),
		});
		expect(lower(["collection", "stat"])).toEqual({
			collection: area(1, 8, 3, 2),
			stat: area(9, 4, 3, 2),
		});
		expect(lower(["stat", "categories"])).toEqual({
			stat: area(1, 4, 3, 2),
			categories: area(5, 8, 3, 2),
		});
		expect(lower(["collection"])).toEqual({ collection: area(1, 12, 3, 2) });
		expect(lower(["categories"])).toEqual({ categories: area(1, 12, 3, 1) });
		expect(lower(["stat"])).toEqual({ stat: area(1, 12, 3, 1) });
	});

	test("single-row stat and categories are short", () => {
		expect(bentoPlacement(["stat"]).stat?.size).toBe("short");
		expect(bentoPlacement(["categories"]).categories?.size).toBe("short");
		expect(bentoPlacement(["stat", "categories"]).stat?.size).toBe("tall");
	});

	test("an empty band is omitted and the other band moves up", () => {
		expect(xlOf(["collection", "stat", "categories"])).toEqual({
			collection: area(1, 6, 1, 2),
			stat: area(7, 2, 1, 2),
			categories: area(9, 4, 1, 2),
		});
		expect(xlOf(["notice", "stat"])).toEqual({
			notice: area(1, 12, 1, 1),
			stat: area(1, 12, 2, 1),
		});
		expect(bentoPlacement([])).toEqual({});
	});

	test("the full layout is exactly BENTO_SLOTS and BENTO_ROW_HEIGHTS", () => {
		expect(xlOf([...GRID_SLOT_KEYS])).toEqual(
			Object.fromEntries(
				BENTO_SLOTS.map(({ key, col, span, row, rowSpan }) => [
					key,
					area(col, span, row, rowSpan),
				]),
			),
		);
		expect(bentoRows(GRID_SLOT_KEYS).xl).toEqual([...BENTO_ROW_HEIGHTS]);
	});

	test("slot order in the input does not matter", () => {
		expect(bentoPlacement(["categories", "feature", "hero", "stat"])).toEqual(
			bentoPlacement(["hero", "feature", "stat", "categories"]),
		);
	});

	test("xl row heights per band", () => {
		expect(
			bentoRows(["hero", "notice", "feature", "collection", "stat"]).xl,
		).toEqual([176, 204, 92, 92]);
		expect(bentoRows(["notice", "categories"]).xl).toEqual([176, 92]);
		expect(bentoRows(["feature", "stat"]).xl).toEqual([204, 92]);
		expect(bentoRows(["hero"]).xl).toEqual([176, 204]);
		expect(bentoRows([]).xl).toEqual([]);
	});

	test("md: hero 6, notice/feature 3/3 or 6, collection 6, stat/categories 2/4 or 6", () => {
		const full = bentoPlacement([
			"hero",
			"notice",
			"feature",
			"collection",
			"stat",
			"categories",
		]);
		expect(full.hero?.md).toEqual(area(1, 6, 1, 1));
		expect(full.notice?.md).toEqual(area(1, 3, 2, 1));
		expect(full.feature?.md).toEqual(area(4, 3, 2, 1));
		expect(full.collection?.md).toEqual(area(1, 6, 3, 1));
		expect(full.stat?.md).toEqual(area(1, 2, 4, 1));
		expect(full.categories?.md).toEqual(area(3, 4, 4, 1));
		expect(full.stat?.mdSize).toBe("tall");
		expect(full.categories?.mdSize).toBe("tall");

		const sparse = bentoPlacement(["notice", "categories"]);
		expect(sparse.notice?.md).toEqual(area(1, 6, 1, 1));
		expect(sparse.categories?.md).toEqual(area(1, 6, 2, 1));
		expect(sparse.categories?.mdSize).toBe("short");
		expect(bentoPlacement(["hero", "feature"]).feature?.md).toEqual(
			area(1, 6, 2, 1),
		);
		expect(bentoPlacement(["stat"]).stat?.md).toEqual(area(1, 6, 1, 1));
		expect(
			bentoRows([
				"hero",
				"notice",
				"feature",
				"collection",
				"stat",
				"categories",
			]).md,
		).toEqual([360, 204, 200, 200]);
		expect(bentoRows(["notice", "stat"]).md).toEqual([176, 92]);
	});
});

describe("row layouts", () => {
	const trending: ExploreResolvedItem[] = [
		A.copilot,
		A.research,
		A.invoice,
		P.codeInterpreter,
		P.youtube,
		P.typst,
	];

	test("trending, dev: app | package pair | app", () => {
		expect(
			layoutTrending(trending, true).map((cell) => [
				itemId(cell.item),
				cell.col,
				cell.span,
				cell.row,
				cell.rowSpan,
			]),
		).toEqual([
			["app:app-customer-support-copilot", 1, 3, 1, 2],
			["package:code-interpreter", 4, 6, 1, 1],
			["package:youtube-tools", 4, 6, 2, 1],
			["app:app-research-assistant", 10, 3, 1, 2],
		]);
	});

	test("trending, non-dev: four portrait apps and no packages", () => {
		const cells = layoutTrending([...trending, A.sales], false);
		expect(cells.map((cell) => itemId(cell.item))).toEqual([
			"app:app-customer-support-copilot",
			"app:app-research-assistant",
			"app:app-invoice-autopilot",
			"app:app-sales-pulse",
		]);
		expect(cells.map((cell) => cell.col)).toEqual([1, 4, 7, 10]);
		expect(cells.every((cell) => cell.span === 3 && cell.rowSpan === 2)).toBe(
			true,
		);
	});

	test("trending, packages only: two landscape pairs", () => {
		const cells = layoutTrending(
			[P.codeInterpreter, P.youtube, P.typst, P.pdfTools, P.feedReader],
			true,
		);
		expect(
			cells.map((cell) => [itemId(cell.item), cell.col, cell.row]),
		).toEqual([
			["package:code-interpreter", 1, 1],
			["package:youtube-tools", 7, 1],
			["package:typst-documents", 1, 2],
			["package:pdf-tools", 7, 2],
		]);
	});

	test("trending, dev with one app: the package pair fills the rest of the row", () => {
		expect(
			layoutTrending([A.copilot, P.codeInterpreter, P.youtube], true).map(
				(cell) => [itemId(cell.item), cell.col, cell.span, cell.row],
			),
		).toEqual([
			["app:app-customer-support-copilot", 1, 3, 1],
			["package:code-interpreter", 4, 9, 1],
			["package:youtube-tools", 4, 9, 2],
		]);
	});

	test("trending, packages only: a package without a partner spans the row", () => {
		const cells = (items: ExploreResolvedItem[]) =>
			layoutTrending(items, true).map((cell) => [
				itemId(cell.item),
				cell.col,
				cell.span,
				cell.row,
			]);
		expect(cells([P.codeInterpreter, P.youtube, P.typst])).toEqual([
			["package:code-interpreter", 1, 6, 1],
			["package:youtube-tools", 7, 6, 1],
			["package:typst-documents", 1, 12, 2],
		]);
		expect(cells([P.typst])).toEqual([["package:typst-documents", 1, 12, 1]]);
	});

	test("trending, dev with fewer than two packages stays apps-only", () => {
		expect(
			layoutTrending([A.copilot, A.research, P.youtube], true).map((cell) =>
				itemId(cell.item),
			),
		).toEqual([
			"app:app-customer-support-copilot",
			"app:app-research-assistant",
		]);
	});

	test("top paid, dev: two apps and two packages", () => {
		const items = [
			A.invoice,
			A.sales,
			A.research,
			P.invoiceExtraction,
			P.privacyBlur,
			P.typst,
		];
		expect(layoutTopPaid(items, true).map(itemId)).toEqual([
			"app:app-invoice-autopilot",
			"app:app-sales-pulse",
			"package:invoice-extraction",
			"package:privacy-blur",
		]);
	});

	test("top paid, dev: a short side is topped up from the other kind", () => {
		expect(
			layoutTopPaid(
				[A.invoice, P.invoiceExtraction, P.privacyBlur, P.typst, P.pdfTools],
				true,
			).map(itemId),
		).toEqual([
			"app:app-invoice-autopilot",
			"package:invoice-extraction",
			"package:privacy-blur",
			"package:typst-documents",
		]);
		expect(
			layoutTopPaid(
				[A.invoice, A.sales, A.research, A.copilot, P.privacyBlur],
				true,
			).map(itemId),
		).toEqual([
			"app:app-invoice-autopilot",
			"app:app-sales-pulse",
			"app:app-research-assistant",
			"package:privacy-blur",
		]);
	});

	test("top paid, non-dev: four apps", () => {
		expect(
			layoutTopPaid(
				[
					A.invoice,
					A.sales,
					A.research,
					A.copilot,
					A.bounded,
					P.invoiceExtraction,
				],
				false,
			).map(itemId),
		).toEqual([
			"app:app-invoice-autopilot",
			"app:app-sales-pulse",
			"app:app-research-assistant",
			"app:app-customer-support-copilot",
		]);
	});
});

describe("parseResolvedExplore", () => {
	test("keeps every view of the fixture", () => {
		const fixture = exploreFixture({ dev: true });
		const parsed = parseResolvedExplore(roundTripJson(fixture));
		expect(parsed.viewer).toEqual(fixture.viewer);
		expect(parsed.typeCounts).toEqual(fixture.typeCounts);
		expect(viewIds(parsed.views.all)).toEqual(viewIds(fixture.views.all));
		expect(viewIds(parsed.views.apps as ExploreView)).toEqual(
			viewIds(fixture.views.apps as ExploreView),
		);
		expect(viewIds(parsed.views.packages as ExploreView)).toEqual(
			viewIds(fixture.views.packages as ExploreView),
		);
	});

	test("absent views stay null for a non-dev page", () => {
		const parsed = parseResolvedExplore(
			roundTripJson(exploreFixture({ dev: false })),
		);
		expect(parsed.views.apps).toBeNull();
		expect(parsed.views.packages).toBeNull();
		expect(parsed.viewer.dev).toBe(false);
	});

	test("rejects bodies that are not objects", () => {
		expect(() => parseResolvedExplore("<!doctype html><html>")).toThrow(
			/expected an object, got text/,
		);
		expect(() => parseResolvedExplore(null)).toThrow(/got null/);
		expect(() => parseResolvedExplore([])).toThrow(/got an array/);
		expect(() => parseResolvedExplore(undefined)).toThrow(/got undefined/);
	});

	test("rejects a page without views.all", () => {
		expect(() => parseResolvedExplore({ revision: "r", views: {} })).toThrow(
			/views\.all/,
		);
	});

	test("drops malformed items, unknown kinds and rows instead of crashing", () => {
		const parsed = parseResolvedExplore({
			revision: "r",
			views: {
				all: {
					grid: {
						hero: { placementId: "h", rotationSeconds: 8, slides: [] },
						notice: "nope",
						feature: { placementId: "f", slide: { item: { kind: "widget" } } },
						stat: { placementId: "s", rail: "new_count", stat: { total: 3 } },
					},
					rows: [
						{
							kind: "rail",
							rail: {
								placementId: "t",
								rail: "trending",
								items: [
									{ kind: "app", app: { id: "a" }, metadata: null },
									{ kind: "app", app: null },
									{ kind: "package", package: { name: "no id" } },
									{ kind: "sponsored" },
									"garbage",
								],
							},
						},
						{ kind: "rail", rail: { placementId: "x", rail: "mystery" } },
						{ kind: "banner" },
					],
				},
			},
		});
		const { grid, rows } = parsed.views.all;
		expect(grid.hero).toBeNull();
		expect(grid.notice).toBeNull();
		expect(grid.feature).toBeNull();
		expect(grid.stat?.stat).toEqual({
			total: 3,
			apps: 0,
			packages: 0,
			freeApps: 0,
			paidApps: 0,
			verifiedPackages: 0,
		});
		expect(rows).toHaveLength(1);
		expect(rows[0].kind === "rail" && rows[0].rail.items.map(itemId)).toEqual([
			"app:a",
		]);
		expect(parsed.typeCounts).toEqual({ apps: 0, packages: 0 });
	});
});

describe("parseResolvedExplore trust boundary", () => {
	function gridOf(grid: Record<string, unknown>) {
		return parseResolvedExplore({ views: { all: { grid, rows: [] } } }).views
			.all.grid;
	}

	function notice(content: Record<string, unknown>) {
		return gridOf({
			notice: {
				placementId: "n",
				title: "Hello",
				body: "World",
				ctaLabel: "Go",
				...content,
			},
		}).notice;
	}

	function spotlight(
		slide: Record<string, unknown>,
		rotationSeconds?: unknown,
	) {
		return gridOf({
			hero: {
				placementId: "h",
				rotationSeconds,
				slides: [{ item: { kind: "app", app: { id: "a" } }, ...slide }],
			},
		}).hero;
	}

	test("a CTA is an app path or an https URL", () => {
		for (const href of [
			"/store/explore?type=packages",
			"/",
			"https://flow-like.com/blog",
		]) {
			expect(notice({ ctaHref: href })).toMatchObject({
				ctaHref: href,
				ctaLabel: "Go",
			});
		}
	});

	test("any other CTA is dropped together with its label", () => {
		for (const href of [
			"//evil.test",
			"/\\evil.test",
			"/\t/evil.test",
			"/path with space",
			"javascript:alert(1)",
			"data:text/html,<script>1</script>",
			"file:///etc/passwd",
			"flow-like://open",
			"http://flow-like.com",
			"https://user:pw@flow-like.com",
			`/${"a".repeat(2048)}`,
			42,
		]) {
			expect(notice({ ctaHref: href })).toMatchObject({
				ctaHref: null,
				ctaLabel: null,
			});
		}
	});

	test("images and artwork are https only", () => {
		expect(notice({ imageUrl: "https://cdn.test/a.webp" })?.imageUrl).toBe(
			"https://cdn.test/a.webp",
		);
		expect(
			spotlight({ artworkUrl: "https://cdn.test/a.webp" })?.slides[0]
				.artworkUrl,
		).toBe("https://cdn.test/a.webp");
		for (const url of [
			"http://cdn.test/a.webp",
			"//cdn.test/a.webp",
			"/local.webp",
			"data:image/png;base64,AAAA",
			"https://u:p@cdn.test/a.webp",
		]) {
			expect(notice({ imageUrl: url })?.imageUrl).toBeNull();
			expect(spotlight({ artworkUrl: url })?.slides[0].artworkUrl).toBeNull();
		}
	});

	test("rotation stays within 5–20 s and defaults to 8", () => {
		const rotation = (value: unknown) => spotlight({}, value)?.rotationSeconds;
		expect(rotation(12)).toBe(12);
		expect(rotation(1)).toBe(5);
		expect(rotation(0.2)).toBe(5);
		expect(rotation(999)).toBe(20);
		expect(rotation(-1)).toBe(8);
		expect(rotation(0)).toBe(8);
		expect(rotation("10")).toBe(8);
		expect(rotation(undefined)).toBe(8);
	});
});

describe("parseExploreSearch", () => {
	test("keeps the fixture's groups and facets", () => {
		const fixture = exploreSearchFixture({ dev: true });
		const parsed = parseExploreSearch(roundTripJson(fixture));
		expect(parsed.apps.items.map(itemId)).toEqual([
			"app:app-invoice-autopilot",
		]);
		expect(
			parsed.packages.items.map((hit) => [
				hit.package.id,
				hit.matchedVia,
				hit.collectionTitle,
			]),
		).toEqual([
			["invoice-extraction", "name", null],
			["pdf-tools", "collection", "Automate your invoices"],
			["typst-documents", "collection", "Automate your invoices"],
		]);
		expect(parsed.facets).toEqual(fixture.facets);
		expect(parsed.collections.map((c) => c.placementId)).toEqual([
			"collection-invoices",
		]);
	});

	test("drops facet values that do not parse and non-app app hits", () => {
		const parsed = parseExploreSearch({
			query: "x",
			apps: {
				items: [
					{ kind: "package", package: { id: "p" } },
					{ kind: "app", app: { id: "a" }, metadata: null },
				],
				total: 2,
				hasMore: true,
			},
			facets: {
				categories: [
					{ value: "app:Finance", kind: "app", count: 2 },
					{ value: "Finance", kind: "app", count: 9 },
					{ value: "package:Education", kind: "package", count: 1 },
				],
			},
		});
		expect(parsed.apps.items.map(itemId)).toEqual(["app:a"]);
		expect(parsed.apps.hasMore).toBe(true);
		expect(parsed.facets.categories).toEqual([
			{ value: "app:Finance", kind: "app", count: 2 },
		]);
		expect(parsed.packages).toEqual({ items: [], total: 0, hasMore: false });
	});

	test("rejects bodies that are not objects", () => {
		expect(() => parseExploreSearch("<html>")).toThrow(/expected an object/);
		expect(() => parseExploreSearch(null)).toThrow(/got null/);
	});
});

describe("toExploreError", () => {
	test("a codeless 404 means the hub has no Explore", () => {
		const routeMiss = new ApiResponseError({
			status: 404,
			message: "Not Found",
		});
		const mapped = toExploreError(routeMiss);
		expect(mapped).toBeInstanceOf(ExploreUnsupportedError);
		expect(isExploreUnsupportedError(mapped)).toBe(true);
		expect(
			toExploreError(
				new ApiResponseError({ status: 404, code: " ", message: "x" }),
			),
		).toBeInstanceOf(ExploreUnsupportedError);
	});

	test("a 404 with a code stays an API error", () => {
		const missing = new ApiResponseError({
			status: 404,
			code: "NOT_FOUND",
			message: "Explore collection c1 is not available",
		});
		expect(toExploreError(missing)).toBe(missing);
	});

	test("other failures pass through", () => {
		const unauthorized = new ApiResponseError({
			status: 401,
			message: "Unauthorized",
		});
		const offline = new Error("Network unavailable: GET /store/explore");
		expect(toExploreError(unauthorized)).toBe(unauthorized);
		expect(toExploreError(offline)).toBe(offline);
		expect(toExploreError("boom")).toBe("boom");
	});
});

describe("exploreSupportFromQuery", () => {
	test("undefined only while pending", () => {
		expect(
			exploreSupportFromQuery({ status: "pending", error: null }),
		).toBeUndefined();
	});

	test("false only for a hub without Explore", () => {
		expect(
			exploreSupportFromQuery({
				status: "error",
				error: new ExploreUnsupportedError(),
			}),
		).toBe(false);
	});

	test("true on success and on every other error", () => {
		expect(exploreSupportFromQuery({ status: "success", error: null })).toBe(
			true,
		);
		for (const error of [
			new ApiResponseError({ status: 401, message: "Unauthorized" }),
			new Error("Network unavailable: GET /store/explore"),
			new Error("Explore needs a hub connection"),
			new ApiResponseError({ status: 500, message: "Internal" }),
		]) {
			expect(exploreSupportFromQuery({ status: "error", error })).toBe(true);
		}
	});
});

describe("exploreViewerSignedIn", () => {
	test("a push counts once OIDC has finished loading", () => {
		for (const signedIn of [true, false]) {
			expect(
				exploreViewerSignedIn({ signedIn, authLoading: false, stalled: false }),
			).toBe(signedIn);
		}
	});

	test("unknown while nothing was pushed or the host still loads OIDC", () => {
		for (const signedIn of [undefined, false, true]) {
			expect(
				exploreViewerSignedIn({ signedIn, authLoading: true, stalled: false }),
			).toBeUndefined();
		}
		expect(
			exploreViewerSignedIn({
				signedIn: undefined,
				authLoading: false,
				stalled: false,
			}),
		).toBeUndefined();
	});

	test("after the grace period a host that never settled counts as signed out", () => {
		expect(
			exploreViewerSignedIn({
				signedIn: undefined,
				authLoading: false,
				stalled: true,
			}),
		).toBe(false);
		expect(
			exploreViewerSignedIn({
				signedIn: undefined,
				authLoading: true,
				stalled: true,
			}),
		).toBe(false);
		expect(
			exploreViewerSignedIn({
				signedIn: true,
				authLoading: true,
				stalled: true,
			}),
		).toBe(true);
	});
});

describe("write-side URL rules", () => {
	test("isExploreCtaHref takes an app path or an https URL", () => {
		for (const href of [
			"/",
			"/store/explore?type=packages",
			"https://a.test",
		]) {
			expect(isExploreCtaHref(href)).toBe(true);
		}
		for (const href of [
			"//evil.test",
			"/\\evil.test",
			"/path with space",
			"/tab\u0085",
			"javascript:alert(1)",
			"http://a.test",
			"https://user:pw@a.test",
			`/${"a".repeat(2048)}`,
		]) {
			expect(isExploreCtaHref(href)).toBe(false);
		}
	});

	test("isExploreHttpsUrl takes https with a host and no credentials", () => {
		expect(isExploreHttpsUrl("https://cdn.test/a.webp")).toBe(true);
		for (const url of [
			"http://cdn.test/a.webp",
			"/local.webp",
			"https://u:p@cdn.test/a.webp",
			"https://cdn.test/\u0007",
			`https://cdn.test/${"a".repeat(2048)}`,
			"not a url",
		]) {
			expect(isExploreHttpsUrl(url)).toBe(false);
		}
	});
});

describe("exploreRetry", () => {
	test("never retries a hub without Explore or a client error", () => {
		for (const error of [
			new ExploreUnsupportedError(),
			new ApiResponseError({ status: 400, message: "q too long" }),
			new ApiResponseError({ status: 401, message: "Unauthorized" }),
			new ApiResponseError({ status: 403, message: "Forbidden" }),
			new ApiResponseError({
				status: 404,
				code: "NOT_FOUND",
				message: "Collection not found",
			}),
		]) {
			expect(exploreRetry(0, error)).toBe(false);
		}
	});

	test("retries a transient failure once", () => {
		for (const error of [
			new ApiResponseError({ status: 408, message: "Timeout" }),
			new ApiResponseError({ status: 429, message: "Slow down" }),
			new ApiResponseError({ status: 500, message: "Internal" }),
			new ApiResponseError({ status: 503, message: "Unavailable" }),
			new Error("Network unavailable: GET /store/explore"),
		]) {
			expect(exploreRetry(0, error)).toBe(true);
			expect(exploreRetry(1, error)).toBe(false);
		}
	});
});

describe("request paths", () => {
	test("the landing adds platform, language and dev", () => {
		expect(exploreRequestPath({ language: "de", dev: true }, "desktop")).toBe(
			"store/explore?platform=desktop&language=de&dev=true",
		);
	});

	test("search serializes lists as one comma-joined param and paging as snake_case", () => {
		const path = exploreSearchRequestPath(
			{
				language: "en",
				dev: false,
				q: " invoice ",
				type: "apps",
				categories: ["app:Finance", "package:EDUCATION"],
				price: "free",
				verified: true,
				permissions: ["network", "models"],
				sort: "newest",
				collection: "c1",
				appsOffset: 24,
				appsLimit: 24,
				packagesOffset: 0,
				packagesLimit: 12,
			},
			"web",
		);
		expect(path.startsWith("store/explore/search?")).toBe(true);
		const params = new URLSearchParams(path.slice(path.indexOf("?") + 1));
		expect(Object.fromEntries(params)).toEqual({
			platform: "web",
			language: "en",
			dev: "false",
			q: "invoice",
			type: "apps",
			categories: "app:Finance,package:EDUCATION",
			price: "free",
			verified: "true",
			permissions: "network,models",
			sort: "newest",
			collection: "c1",
			apps_offset: "24",
			apps_limit: "24",
			packages_offset: "0",
			packages_limit: "12",
		});
		expect(params.getAll("categories")).toHaveLength(1);
	});

	test("empty filters are omitted", () => {
		expect(
			exploreSearchRequestPath(
				{ language: "en", dev: true, categories: [], verified: false },
				"desktop",
			),
		).toBe("store/explore/search?platform=desktop&language=en&dev=true");
	});

	test("query, lists and page sizes stay within the server's limits", () => {
		const categories = [
			...Object.values(IAppCategory).map((category) => `app:${category}`),
		];
		expect(categories.length).toBeGreaterThan(EXPLORE_SEARCH_LIMITS.categories);
		const path = exploreSearchRequestPath(
			{
				language: "en",
				dev: true,
				q: "q".repeat(250),
				categories: [categories[0], ...categories],
				permissions: ["none", "network", "none", "models", "storage"],
				appsLimit: 500,
				packagesLimit: 12,
			},
			"desktop",
		);
		const params = new URLSearchParams(path.slice(path.indexOf("?") + 1));
		expect(params.get("q")).toHaveLength(EXPLORE_SEARCH_LIMITS.q);
		expect(params.get("categories")?.split(",")).toEqual(
			categories.slice(0, EXPLORE_SEARCH_LIMITS.categories),
		);
		expect(params.get("permissions")).toBe("none,network,models,storage");
		expect(params.get("apps_limit")).toBe(
			String(EXPLORE_SEARCH_LIMITS.pageSize),
		);
		expect(params.get("packages_limit")).toBe("12");
	});
});

describe("dismissStorage", () => {
	function memoryStorage(): Storage {
		const data = new Map<string, string>();
		return {
			get length() {
				return data.size;
			},
			clear: () => data.clear(),
			getItem: (key) => data.get(key) ?? null,
			key: (index) => [...data.keys()][index] ?? null,
			removeItem: (key) => {
				data.delete(key);
			},
			setItem: (key, value) => {
				data.set(key, value);
			},
		};
	}

	test("dismiss, read and restore", () => {
		const store = createDismissStorage(memoryStorage);
		const storage = memoryStorage();
		const shared = createDismissStorage(() => storage);
		expect(store.read()).toEqual([]);
		shared.dismiss("a:1");
		shared.dismiss("b:2");
		shared.dismiss("a:1");
		expect(shared.read()).toEqual(["b:2", "a:1"]);
		expect(shared.isDismissed("a:1")).toBe(true);
		shared.restore("a:1");
		expect(shared.isDismissed("a:1")).toBe(false);
		expect(shared.read()).toEqual(["b:2"]);
	});

	test("keeps only the most recent keys", () => {
		const storage = memoryStorage();
		const store = createDismissStorage(() => storage);
		for (let index = 0; index < 60; index++) store.dismiss(`k:${index}`);
		const keys = store.read();
		expect(keys).toHaveLength(50);
		expect(keys.at(-1)).toBe("k:59");
		expect(keys[0]).toBe("k:10");
	});

	test("survives a throwing or missing localStorage", () => {
		const throwingAccess = createDismissStorage(() => {
			throw new Error("SecurityError");
		});
		const throwingStorage = createDismissStorage(() => ({
			...memoryStorage(),
			getItem: () => {
				throw new Error("denied");
			},
			setItem: () => {
				throw new Error("QuotaExceededError");
			},
		}));
		const missing = createDismissStorage(() => undefined);
		for (const store of [throwingAccess, throwingStorage, missing]) {
			expect(() => store.dismiss("a:1")).not.toThrow();
			expect(() => store.restore("a:1")).not.toThrow();
			expect(store.read()).toEqual([]);
			expect(store.isDismissed("a:1")).toBe(false);
		}
	});

	test("ignores a corrupt stored value", () => {
		const storage = memoryStorage();
		storage.setItem("flow-like.explore.dismissed-announcements", "{not json");
		expect(createDismissStorage(() => storage).read()).toEqual([]);
		storage.setItem(
			"flow-like.explore.dismissed-announcements",
			JSON.stringify(["a", 3, null]),
		);
		expect(createDismissStorage(() => storage).read()).toEqual(["a"]);
	});
});

describe("accentColor", () => {
	test("a category accent wins over the item's category", () => {
		expect(accentColor("category:Finance", A.copilot)).toBe(
			categoryColor("Finance"),
		);
	});

	test("auto uses the item's own category", () => {
		expect(accentColor("auto", A.copilot)).toBe(categoryColor("Business"));
		expect(accentColor(null, A.invoice)).toBe(categoryColor("Finance"));
		expect(accentColor(undefined, P.invoiceExtraction)).toBe(
			categoryColor(appCategoryForPackage("DOCUMENT_PROCESSING")),
		);
		expect(appCategoryForPackage("DOCUMENT_PROCESSING")).toBe(
			IAppCategory.Productivity,
		);
		expect(accentColor("auto", P.privacyBlur)).toBe(categoryColor("Other"));
	});

	test("a collection takes its first preview item's color", () => {
		const collection = exploreFixture().views.all.grid.hero?.slides[1]?.item;
		expect(collection?.kind).toBe("collection");
		expect(accentColor("auto", collection as ExploreResolvedItem)).toBe(
			categoryColor("Finance"),
		);
	});
});
