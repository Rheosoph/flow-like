import { describe, expect, test } from "bun:test";
import { createSlateEditor } from "platejs";
import {
	parsePlateDocument,
	serializePlateDocument,
} from "../../lib/plate-document";
import { BaseEditorKit } from "./editor-base-kit";
import { resolveEditorAssetUrl } from "./hooks/use-editor-asset-url";
import {
	isPublicationUrl,
	preparePublicationContent,
	preparePublicationValue,
} from "./publication";
import { renderPublicationHtml } from "./publication-html";
import { deserializeMarkdownFile } from "./ui/import-toolbar-button";
import { parseEditorStorageReference, toStorageUrl } from "./upload-context";

const image = (url: string) => ({
	type: "img",
	url,
	alt: "Harbor at dawn",
	credit: "A. Reporter",
	license: "CC BY 4.0",
	focalPoint: { x: 20, y: 75 },
	children: [{ text: "" }],
});
const reader = {
	downloadStorageItems: async () => [
		{ url: "data:image/png;base64,aGVsbG8=", prefix: "" },
	],
	downloadStorageItemsUser: async () => [
		{ url: "data:image/png;base64,aGVsbG8=", prefix: "" },
	],
};

describe("publication media", () => {
	test("owned private references survive a different hosting editor", async () => {
		const reference = toStorageUrl("article", "image.png", {
			appId: "owner/one",
			scope: "user",
		});
		expect(parseEditorStorageReference(reference, "wrong", "app")).toEqual({
			appId: "owner/one",
			scope: "user",
			path: "article/image.png",
		});
		const calls: unknown[] = [];
		await resolveEditorAssetUrl(
			reference,
			{
				...reader,
				downloadStorageItems: async () => {
					throw new Error("wrong scope");
				},
				downloadStorageItemsUser: async (appId, path) => {
					calls.push([appId, path]);
					return [{ url: "https://example.test/private.png", prefix: "" }];
				},
			},
			"wrong",
		);
		expect(calls).toEqual([["owner/one", ["article/image.png"]]]);
	});
	test("legacy paths inherit scope", () => {
		expect(
			parseEditorStorageReference("storage://editor/a.png", "app", "user"),
		).toEqual({ appId: "app", scope: "user", path: "editor/a.png" });
	});
	test("portable snapshot embeds bytes without changing draft or review history", async () => {
		const content = serializePlateDocument({
			version: 1,
			children: [image("storage://editor/a.png")],
			discussions: [],
			users: { author: { id: "author", name: "Author" } },
			documentId: "article",
		});
		const publication = await preparePublicationContent(
			content,
			{ appId: "app" },
			reader,
		);
		const parsed = parsePlateDocument(publication);
		if (!parsed) throw new Error("Exported document could not be parsed.");
		expect(parsed.users).toEqual({ author: { id: "author", name: "Author" } });
		expect(parsed.children[0].url).toBe("data:image/png;base64,aGVsbG8=");
		expect(parsePlateDocument(content)?.children[0].url).toBe(
			"storage://editor/a.png",
		);
		const html = await renderPublicationHtml(parsed.children);
		expect(html).toContain('src="data:image/png;base64,aGVsbG8="');
		expect(html).toContain('alt="Harbor at dawn"');
		expect(html).toContain("A. Reporter");
		expect(html).toContain("CC BY 4.0");
		expect(html).toContain("object-position:20% 75%");
	});
	test("limits assets and rejects active links", async () => {
		await expect(
			preparePublicationValue(
				[image("storage://a.png")],
				{ appId: "a", maxAssetBytes: 2 },
				reader,
			),
		).rejects.toThrow("size limit");
		expect(isPublicationUrl("javascript:alert(1)", "a")).toBe(false);
		expect(isPublicationUrl("data:text/html;base64,PHNjcmlwdD4=", "file")).toBe(
			false,
		);
		expect(isPublicationUrl("data:image/png;base64,aGVsbG8=", "a")).toBe(false);
	});
	test("publisher must supply HTTPS", async () => {
		await expect(
			preparePublicationValue(
				[image("storage://a.png")],
				{ resolveAsset: async () => "javascript:alert(1)" },
				reader,
			),
		).rejects.toThrow("HTTPS");
	});
	test("export preserves expired rights metadata while resolving draft media", async () => {
		const source = {
			...image("storage://editor/a.png"),
			rightsExpiresAt: 1,
		};
		const exported = await preparePublicationValue(
			[source],
			{ appId: "app" },
			reader,
		);
		expect(exported).toEqual([
			{ ...source, url: "data:image/png;base64,aGVsbG8=" },
		]);
		expect(source.url).toBe("storage://editor/a.png");
		expect(source.rightsExpiresAt).toBe(1);
	});
	test("existing embedded media consumes the same publication budget", async () => {
		await expect(
			preparePublicationValue(
				[image("data:image/png;base64,aGVsbG8=")],
				{ maxAssetBytes: 4 },
				reader,
			),
		).rejects.toThrow("size limit");
		await expect(
			preparePublicationValue(
				[
					image("data:image/png;base64,aGVsbG8="),
					image("data:image/png;base64,d29ybGQ="),
				],
				{ maxTotalBytes: 9 },
				reader,
			),
		).rejects.toThrow("size limit");
	});
});

test("published embeds keep provider and direct video remains a native video", async () => {
	const html = await renderPublicationHtml([
		{
			type: "media_embed",
			url: "https://vimeo.com/123456789",
			children: [{ text: "" }],
		},
		{
			type: "video",
			url: "https://example.test/report.mp4",
			children: [{ text: "" }],
		},
	]);
	expect(html).toContain('src="https://player.vimeo.com/video/123456789"');
	expect(html).not.toContain("i.ytimg.com");
	expect(html).toContain("<video");
	expect(html).toContain('src="https://example.test/report.mp4"');
});

test("TOC links target rendered heading anchors", async () => {
	const html = await renderPublicationHtml([
		{ type: "toc", children: [{ text: "" }] },
		{ type: "h2", id: "heading-a", children: [{ text: "World news" }] },
	]);
	expect(html).toContain('href="#heading-1"');
	expect(html).toContain('id="heading-1"');
});

test("Markdown import keeps dollar prices as text", () => {
	const editor = createSlateEditor({ plugins: BaseEditorKit });
	const imported = deserializeMarkdownFile(editor, "Costs $5 to $10 today.");
	expect(JSON.stringify(imported)).not.toContain("inline_equation");
	expect(JSON.stringify(imported)).toContain("Costs $5 to $10 today.");
});

test("Markdown round trip preserves separate alt, caption, credit, rights and focal point", () => {
	const editor = createSlateEditor({ plugins: BaseEditorKit });
	const source = {
		...image("https://example.test/photo.png"),
		caption: [{ text: "The morning ferry" }],
		rightsExpiresAt: 1924991999999,
	};
	const markdown = editor.api.markdown.serialize({ value: [source] });
	const imported = deserializeMarkdownFile(editor, markdown);
	expect(imported[0]).toMatchObject({
		alt: source.alt,
		credit: source.credit,
		license: source.license,
		focalPoint: source.focalPoint,
		caption: source.caption,
		rightsExpiresAt: source.rightsExpiresAt,
	});
});

test("reference links, repeated footnotes and equations survive standalone export", async () => {
	const editor = createSlateEditor({ plugins: BaseEditorKit });
	const value = deserializeMarkdownFile(
		editor,
		"Read [the source][source]. A claim.[^note] Again.[^note]\n\n[source]: https://example.test/source\n\n[^note]: Supporting evidence.\n\n$$x^2$$",
	);
	const html = await renderPublicationHtml(value);
	expect(html).toContain('href="https://example.test/source"');
	expect(html).toContain("the source");
	expect(html).toContain("Supporting evidence.");
	const anchors = [...html.matchAll(/href="#(fn-[^"]+)"/g)].map(
		(match) => match[1],
	);
	expect(anchors.length).toBeGreaterThanOrEqual(3);
	const referenceIds = [...html.matchAll(/id="(fn-ref-[^"]+)"/g)].map(
		(match) => match[1],
	);
	expect(new Set(referenceIds).size).toBe(2);
	for (const anchor of anchors) expect(html).toContain(`id="${anchor}"`);
	expect(html).toContain("<math");
	expect(html).not.toContain("https://cdn.jsdelivr.net");
});
