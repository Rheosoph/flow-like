import { describe, expect, it } from "bun:test";
import {
	BOOK_ID,
	BOOK_ORIGIN,
	bookEntryPath,
	bookSocialImagePath,
	buildBookStructuredData,
	getBookBreadcrumbs,
	normalizeBookEntryId,
	resolveBookSeo,
	serializeStructuredData,
} from "./seo";

const chapterData = {
	title: "10. Branches, loops, parallelism, and return",
	description:
		"Learn FlowScript control flow through branches, loops, parallel execution, and return behavior.",
	seo: {
		title: "FlowScript Control Flow: Branches, Loops & Parallelism",
		topics: ["FlowScript control flow", "parallel workflows"],
	},
} as const;

function graphTypes(value: Record<string, unknown> | undefined): string[] {
	if (!value || !Array.isArray(value["@graph"])) return [];
	return value["@graph"].flatMap((node) => {
		if (!node || typeof node !== "object") return [];
		const type = (node as Record<string, unknown>)["@type"];
		return Array.isArray(type) ? type.map(String) : type ? [String(type)] : [];
	});
}

describe("book SEO route resolution", () => {
	it("normalizes entry IDs and creates stable page and image paths", () => {
		expect(normalizeBookEntryId("/part-1/01-the-3-am-call/")).toBe(
			"part-1/01-the-3-am-call",
		);
		expect(bookEntryPath("index")).toBe("/");
		expect(bookEntryPath("contents")).toBe("/contents/");
		expect(bookSocialImagePath("index")).toBe("/social/index.png");
		expect(bookSocialImagePath("404")).toBe("/og.png");
	});

	it("resolves chapter metadata, hierarchy, and collision-proof title copy", () => {
		const seo = resolveBookSeo(
			"part-2/10-branches-loops-parallelism-return",
			chapterData,
		);

		expect(seo.pageType).toBe("reading");
		expect(seo.documentTitle).toBe(chapterData.seo.title);
		expect(seo.location?.number).toBe(10);
		expect(seo.breadcrumbs.map(({ path }) => path)).toEqual([
			"/",
			"/part-2/",
			"/part-2/10-branches-loops-parallelism-return/",
		]);
	});

	it("keeps 404 pages out of structured data and canonical page semantics", () => {
		const seo = resolveBookSeo("404", {
			title: "Missing",
			description: "Missing page",
		});

		expect(seo.pageType).toBe("not-found");
		expect(seo.breadcrumbs).toEqual([]);
		expect(
			buildBookStructuredData(
				seo,
				{ title: "Missing" },
				`${BOOK_ORIGIN}/404/`,
				`${BOOK_ORIGIN}/og.png`,
			),
		).toBeUndefined();
	});
});

describe("book structured data", () => {
	it("describes the site, book, chapter, image, and breadcrumbs as one graph", () => {
		const seo = resolveBookSeo(
			"part-2/10-branches-loops-parallelism-return",
			chapterData,
		);
		const canonical = `${BOOK_ORIGIN}${seo.path}`;
		const structured = buildBookStructuredData(
			seo,
			chapterData,
			canonical,
			`${BOOK_ORIGIN}${seo.imagePath}`,
		);

		expect(graphTypes(structured)).toEqual(
			expect.arrayContaining([
				"Organization",
				"WebSite",
				"Book",
				"WebPage",
				"ImageObject",
				"Chapter",
				"BreadcrumbList",
			]),
		);
		expect(JSON.stringify(structured)).toContain(`\"@id\":\"${BOOK_ID}\"`);
	});

	it("does not invent unsupported book identity fields", () => {
		const seo = resolveBookSeo("index", {
			title: "FlowBook",
			description: "The open FlowScript book.",
		});
		const structured = buildBookStructuredData(
			seo,
			{ title: "FlowBook" },
			`${BOOK_ORIGIN}/`,
			`${BOOK_ORIGIN}/social/index.png`,
		);
		const serialized = JSON.stringify(structured);

		expect(serialized).not.toContain('"author"');
		expect(serialized).not.toContain('"isbn"');
		expect(serialized).not.toContain('"datePublished"');
		expect(serialized).not.toContain('"dateModified"');
	});

	it("escapes markup-significant characters before inline serialization", () => {
		expect(serializeStructuredData({ value: "</script>" })).toBe(
			'{"value":"\\u003c/script>"}',
		);
	});
});

describe("book breadcrumbs", () => {
	it("uses the part hub as the parent of a chapter", () => {
		expect(
			getBookBreadcrumbs("part-1/01-the-3-am-call", "1. The 3 A.M. Call"),
		).toEqual([
			{ name: "FlowBook", path: "/" },
			{ name: "Part I: Software That Explains Itself", path: "/part-1/" },
			{
				name: "Chapter 1: The 3 A.M. Call",
				path: "/part-1/01-the-3-am-call/",
			},
		]);
	});
});
