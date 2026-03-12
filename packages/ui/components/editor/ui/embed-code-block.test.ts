import { describe, expect, test } from "bun:test";
import { parseOgFromHtml } from "./embed-code-block";

describe("parseOgFromHtml", () => {
	test("parses standard quoted OG tags", () => {
		const html = `
			<html><head>
				<meta property="og:title" content="My Page" />
				<meta property="og:description" content="A cool description" />
				<meta property="og:image" content="https://example.com/img.png" />
			</head></html>
		`;
		const og = parseOgFromHtml(html);
		expect(og).not.toBeNull();
		expect(og?.title).toBe("My Page");
		expect(og?.description).toBe("A cool description");
		expect(og?.image).toBe("https://example.com/img.png");
	});

	test("falls back to <title> tag", () => {
		const html = `<html><head><title>Only Title</title></head></html>`;
		const og = parseOgFromHtml(html);
		expect(og).not.toBeNull();
		expect(og?.title).toBe("Only Title");
		expect(og?.description).toBeUndefined();
		expect(og?.image).toBeUndefined();
	});

	test("handles reversed attribute order (content before property)", () => {
		const html = `
			<html><head>
				<meta content="Reversed Title" property="og:title" />
				<meta content="Reversed Desc" property="og:description" />
			</head></html>
		`;
		const og = parseOgFromHtml(html);
		expect(og?.title).toBe("Reversed Title");
		expect(og?.description).toBe("Reversed Desc");
	});

	test("handles single-quoted attributes", () => {
		const html = `
			<html><head>
				<meta property='og:title' content='Single Quotes' />
			</head></html>
		`;
		const og = parseOgFromHtml(html);
		expect(og?.title).toBe("Single Quotes");
	});

	test("handles name attribute for description", () => {
		const html = `
			<html><head>
				<meta name="description" content="Name-based desc" />
			</head></html>
		`;
		const og = parseOgFromHtml(html);
		expect(og?.description).toBe("Name-based desc");
	});

	test("handles unquoted attributes (Astro minified HTML)", () => {
		const html = `<meta content=website property=og:type><meta content=Flow-Like property=og:site_name><meta content="Flow-Like | Open Source" property=og:title><meta content="Build type-safe flows" property=og:description><meta content=https://flow-like.com/og.png property=og:image>`;
		const og = parseOgFromHtml(html);
		expect(og?.title).toBe("Flow-Like | Open Source");
		expect(og?.description).toBe("Build type-safe flows");
		expect(og?.image).toBe("https://flow-like.com/og.png");
	});

	test("returns null for empty HTML", () => {
		expect(parseOgFromHtml("")).toBeNull();
	});

	test("returns null for HTML with no OG or title tags", () => {
		const html = `<html><head><link rel="stylesheet" href="/style.css"></head><body>Hello</body></html>`;
		expect(parseOgFromHtml(html)).toBeNull();
	});

	test("only parses head section, ignores body", () => {
		const html = `
			<html><head>
				<meta property="og:title" content="Head Title" />
			</head><body>
				<meta property="og:title" content="Body Title" />
			</body></html>
		`;
		const og = parseOgFromHtml(html);
		expect(og?.title).toBe("Head Title");
	});

	test("og:title takes precedence over <title>", () => {
		const html = `
			<html><head>
				<title>Fallback Title</title>
				<meta property="og:title" content="OG Title" />
			</head></html>
		`;
		const og = parseOgFromHtml(html);
		expect(og?.title).toBe("OG Title");
	});

	test("og:description takes precedence over name=description", () => {
		const html = `
			<html><head>
				<meta name="description" content="Name Desc" />
				<meta property="og:description" content="OG Desc" />
			</head></html>
		`;
		const og = parseOgFromHtml(html);
		expect(og?.description).toBe("OG Desc");
	});

	test("extracts OG from real flow-like.com HTML snapshot", () => {
		// Real minified HTML from flow-like.com (Astro output)
		const html = `<!DOCTYPE html><html class=scroll-smooth lang=en><head><meta charset=utf-8><meta content="width=device-width,initial-scale=1" name=viewport><meta content=website property=og:type><meta content=Flow-Like property=og:site_name><meta content="Flow-Like | Open Source Workflow Engine - Local-First, Rust-Powered, Type-Safe" property=og:title><meta content="Open source workflow engine for local-first and self-hosted automation. Build type-safe, auditable flows in Rust and deploy on-prem or air-gapped." property=og:description><meta content=https://flow-like.com property=og:url><meta content=https://flow-like.com/og.png property=og:image><title>Flow-Like | Open Source Workflow Engine</title></head><body></body></html>`;

		const og = parseOgFromHtml(html);
		expect(og).not.toBeNull();
		expect(og?.title).toContain("Flow-Like");
		expect(og?.description).toContain("workflow engine");
		expect(og?.image).toBe("https://flow-like.com/og.png");
	});
});
