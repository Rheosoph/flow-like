import { describe, expect, test } from "bun:test";
import { CONTRACT_VERSION, type WidgetContract } from "../src/contract-types";
import { buildCsp, injectCspMeta } from "../src/csp";
import { injectContractScript, inlineHtml } from "../src/inline";

const ENCODER = new TextEncoder();

const SAMPLE_CONTRACT: WidgetContract = {
	contractVersion: CONTRACT_VERSION,
	id: "sample",
	inputs: {
		note: { type: "string", default: "</script><img>" },
	},
	events: {},
	queries: {},
	sizing: { defaultHeight: 320, resizable: true },
};

describe("buildCsp", () => {
	test("includes the serving prefix in every fetch directive", () => {
		const csp = buildCsp("flow-widget://pkg@abc/", ["https://api.example.com"]);
		expect(csp).toBe(
			"default-src 'none'; " +
				"script-src 'unsafe-inline' 'self' flow-widget: http://flow-widget.localhost flow-widget://pkg@abc/; " +
				"style-src 'unsafe-inline' 'self' flow-widget: http://flow-widget.localhost flow-widget://pkg@abc/; " +
				"img-src data: blob: 'self' flow-widget: http://flow-widget.localhost flow-widget://pkg@abc/; " +
				"font-src data: 'self' flow-widget: http://flow-widget.localhost flow-widget://pkg@abc/; " +
				"connect-src https://api.example.com",
		);
	});

	test("null prefix permits only supported bundle asset origins", () => {
		expect(buildCsp(null, [])).toBe(
			"default-src 'none'; script-src 'unsafe-inline' 'self' flow-widget: http://flow-widget.localhost; style-src 'unsafe-inline' 'self' flow-widget: http://flow-widget.localhost; img-src data: blob: 'self' flow-widget: http://flow-widget.localhost; font-src data: 'self' flow-widget: http://flow-widget.localhost; connect-src 'none'",
		);
	});
});

describe("injectCspMeta", () => {
	test("inserts the meta at the start of head", () => {
		const html = "<html><head><title>x</title></head><body></body></html>";
		const out = injectCspMeta(html, "default-src 'none'");
		expect(out).toBe(
			'<html><head><meta http-equiv="Content-Security-Policy" content="default-src \'none\'" /><title>x</title></head><body></body></html>',
		);
	});

	test("creates a head when the document has none", () => {
		const out = injectCspMeta(
			"<html><body></body></html>",
			"default-src 'none'",
		);
		expect(out).toContain("<html><head><meta http-equiv=");
		expect(out).toContain("</head><body>");
	});

	test("escapes the attribute value", () => {
		const out = injectCspMeta("<head></head>", 'x"y<z>&');
		expect(out).toContain('content="x&quot;y&lt;z&gt;&amp;"');
	});
});

describe("inlineHtml", () => {
	const assets: Record<string, string> = {
		"index.js": 'console.log("</script> breakout");',
		"style.css": "#root { color: red; }",
	};
	const resolveAsset = (rel: string) =>
		assets[rel] !== undefined ? ENCODER.encode(assets[rel]) : null;

	test("inlines local scripts and styles, rewrites shared refs", () => {
		const html = [
			"<html><head>",
			'<link rel="modulepreload" crossorigin href="/shared/react-1.js" />',
			'<link rel="stylesheet" href="./style.css" />',
			"</head><body>",
			'<script type="module" crossorigin src="/shared/react-1.js"></script>',
			'<script type="module" src="./index.js"></script>',
			'<script src="https://cdn.example.com/x.js"></script>',
			"</body></html>",
		].join("\n");

		const result = inlineHtml(html, resolveAsset);
		expect(result.inlined.sort()).toEqual(["index.js", "style.css"]);
		expect(result.external).toEqual(["shared/react-1.js"]);
		expect(result.html).toContain('src="../../shared/react-1.js"');
		expect(result.html).toContain('href="../../shared/react-1.js"');
		expect(result.html).toContain(
			'<script type="module">console.log("<\\/script> breakout");</script>',
		);
		expect(result.html).toContain("<style>#root { color: red; }</style>");
		expect(result.html).toContain('src="https://cdn.example.com/x.js"');
		expect(result.html).not.toContain('src="./index.js"');
		expect(result.html).not.toContain('href="./style.css"');
	});

	test("rewrites relative shared refs (../shared, ../../shared)", () => {
		const html =
			'<head></head><body><script src="../../shared/a.js"></script><script src="../shared/b.js"></script></body>';
		const result = inlineHtml(html, () => null);
		expect(result.external).toEqual(["shared/a.js", "shared/b.js"]);
		expect(result.html).toContain('src="../../shared/a.js"');
		expect(result.html).toContain('src="../../shared/b.js"');
	});

	test("drops modulepreload links for inlined local chunks", () => {
		const html =
			'<head><link rel="modulepreload" href="./index.js" /></head><body><script src="./index.js"></script></body>';
		const result = inlineHtml(html, resolveAsset);
		expect(result.html).not.toContain("modulepreload");
	});

	test("throws on unresolvable local references", () => {
		expect(() =>
			inlineHtml('<script src="./missing.js"></script>', () => null),
		).toThrow(/missing\.js/);
	});
});

describe("injectContractScript", () => {
	test("adds the contract as the first script in head and escapes it", () => {
		const html =
			"<html><head><title>t</title><script>other();</script></head><body></body></html>";
		const out = injectContractScript(html, SAMPLE_CONTRACT);
		const firstScript = out.indexOf("<script>");
		expect(out.slice(firstScript)).toStartWith(
			"<script>globalThis.__FLW_CONTRACT__ = ",
		);
		expect(out.indexOf("globalThis.__FLW_CONTRACT__")).toBeLessThan(
			out.indexOf("other();"),
		);
		expect(out).not.toContain("</script><img>");
		expect(out).toContain("\\u003c/script>\\u003cimg>");
	});

	test("replaces a previously injected contract script", () => {
		const html =
			"<html><head><script>globalThis.__FLW_CONTRACT__ = {};</script></head><body></body></html>";
		const out = injectContractScript(html, SAMPLE_CONTRACT);
		const occurrences = out.split("__FLW_CONTRACT__").length - 1;
		expect(occurrences).toBe(1);
		expect(out).toContain('"id":"sample"');
	});
});
