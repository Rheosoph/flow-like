import { describe, expect, test } from "bun:test";
import type { LessonAssetView } from "../../lib/learn/types";
import {
	removeDuplicateLessonTitle,
	resolveAssetReferences,
} from "./lesson-content-utils";

const imageAsset: LessonAssetView = {
	id: "asset-1",
	name: "AppAnatomy",
	mime_type: "image/svg+xml",
	kind: "IMAGE",
	signed_url: "https://assets.example.test/image.svg?sig=fresh",
};

describe("lesson content normalization", () => {
	test("removes a duplicate leading Markdown title", () => {
		const content = [
			"# Find Your Way Around Flow-Like",
			"",
			"The lesson starts here.",
		].join("\n");

		expect(
			removeDuplicateLessonTitle(content, "Find Your Way Around Flow-Like"),
		).toBe("The lesson starts here.");
	});

	test("keeps a leading Markdown title with different meaning", () => {
		const content = "# Product map\n\nThe lesson starts here.";

		expect(removeDuplicateLessonTitle(content, "Studio orientation")).toBe(
			content,
		);
	});

	test("removes a duplicate title with closing hashes and CRLF endings", () => {
		expect(
			removeDuplicateLessonTitle(
				"  # Product map ###\r\n\r\nBody",
				"Product map",
			),
		).toBe("Body");
	});

	test("stays linear on heading lines padded with whitespace", () => {
		const content = `#${"\t".repeat(50000)}Product map\r x`;
		const started = performance.now();
		expect(removeDuplicateLessonTitle(content, "Product map")).toBe(content);
		expect(performance.now() - started).toBeLessThan(1000);
	});

	test("removes a duplicate leading Plate title", () => {
		const content = `plate_json::${JSON.stringify([
			{
				type: "h1",
				children: [
					{ text: "Find Your " },
					{ text: "Way Around Flow-Like", bold: true },
				],
			},
			{ type: "p", children: [{ text: "The lesson starts here." }] },
		])}`;

		const normalized = removeDuplicateLessonTitle(
			content,
			"Find Your Way Around Flow-Like",
		);
		expect(normalized).toStartWith("plate_json::");
		expect(JSON.parse(normalized.slice("plate_json::".length))).toEqual([
			{ type: "p", children: [{ text: "The lesson starts here." }] },
		]);
	});
});

describe("lesson asset references", () => {
	test("uses a readable image label and the fresh signed URL", () => {
		expect(
			resolveAssetReferences("Before\n\n@AppAnatomy\n\nAfter", [imageAsset]),
		).toBe(
			"Before\n\n![App Anatomy](https://assets.example.test/image.svg?sig=fresh)\n\nAfter",
		);
	});

	test("refreshes a Plate asset URL while preserving authored presentation", () => {
		const content = `plate_json::${JSON.stringify([
			{
				type: "img",
				assetName: "AppAnatomy",
				url: "https://expired.example.test/image.svg",
				width: 640,
				caption: [{ text: "App boundary" }],
				children: [{ text: "" }],
			},
		])}`;

		const resolved = resolveAssetReferences(content, [imageAsset]);
		const [node] = JSON.parse(resolved.slice("plate_json::".length));
		expect(node).toMatchObject({
			assetName: "AppAnatomy",
			url: imageAsset.signed_url,
			width: 640,
			alt: "App Anatomy",
			caption: [{ text: "App boundary" }],
		});
	});
});
