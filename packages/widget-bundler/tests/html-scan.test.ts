import { describe, expect, test } from "bun:test";
import { CONTRACT_VERSION, type WidgetContract } from "../src/contract-types";
import { findTag, insertAtHeadStart } from "../src/html";
import {
	contractScriptContent,
	injectContractScript,
	inlineHtml,
} from "../src/inline";

const ENCODER = new TextEncoder();

const ASSETS: Record<string, string> = {
	"index.js": 'console.log("entry");',
	"style.css": "#root { color: red; }",
};

const resolveAsset = (rel: string) =>
	ASSETS[rel] !== undefined ? ENCODER.encode(ASSETS[rel]) : null;

const CONTRACT: WidgetContract = {
	contractVersion: CONTRACT_VERSION,
	id: "pinned",
	inputs: {},
	events: {},
	queries: {},
	sizing: { defaultHeight: 200, resizable: true },
};

const CONTRACT_TAG = `<script>${contractScriptContent(CONTRACT)}</script>`;

function countContracts(html: string): number {
	return html.split("__FLW_CONTRACT__").length - 1;
}

interface FindTagCase {
	readonly label: string;
	readonly html: string;
	readonly tag: string;
	readonly expected: string | null;
}

const FIND_TAG_CASES: readonly FindTagCase[] = [
	{
		label: "plain start tag",
		html: '<a><link rel="x"><b>',
		tag: "link",
		expected: '<link rel="x">',
	},
	{
		label: "bare tag with no attributes",
		html: "<p><link><p>",
		tag: "link",
		expected: "<link>",
	},
	{
		label: "word-boundary guard rejects <linkage>",
		html: '<linkage rel="stylesheet" href="./style.css">',
		tag: "link",
		expected: null,
	},
	{
		label: "word-boundary guard rejects <header> when looking for head",
		html: "<header>x</header>",
		tag: "head",
		expected: null,
	},
	{
		label: "word-boundary guard rejects <htmlx> when looking for html",
		html: "<htmlx></htmlx>",
		tag: "html",
		expected: null,
	},
	{
		label: "word-boundary guard rejects a trailing underscore",
		html: "<link_ rel='x'>",
		tag: "link",
		expected: null,
	},
	{
		label: "a hyphen is not a word char, so <link-2> matches",
		html: "<link-2>",
		tag: "link",
		expected: "<link-2>",
	},
	{
		label: "scan continues past a rejected near-match",
		html: "<linkage><link>",
		tag: "link",
		expected: "<link>",
	},
	{
		label: "uppercase tag name",
		html: '<LINK REL="stylesheet">',
		tag: "link",
		expected: '<LINK REL="stylesheet">',
	},
	{
		label: "mixed case tag name",
		html: '<ScRiPt src="a.js">',
		tag: "script",
		expected: '<ScRiPt src="a.js">',
	},
	{
		label: "self-closing without a space",
		html: "<link/>",
		tag: "link",
		expected: "<link/>",
	},
	{
		label: "self-closing with a space",
		html: "<link />",
		tag: "link",
		expected: "<link />",
	},
	{
		label: "newline between name and attributes",
		html: '<link\n\trel="x">',
		tag: "link",
		expected: '<link\n\trel="x">',
	},
	{
		label: "space before the closing bracket",
		html: "<head >",
		tag: "head",
		expected: "<head >",
	},
	{
		label: "a close tag is not a start tag",
		html: "</link><link>",
		tag: "link",
		expected: "<link>",
	},
	{
		label: "unterminated tag with no '>' anywhere",
		html: '<div></div><link rel="x"',
		tag: "link",
		expected: null,
	},
	{
		label: "document shorter than the tag name",
		html: "<lin",
		tag: "link",
		expected: null,
	},
];

describe("findTag", () => {
	for (const { label, html, tag, expected } of FIND_TAG_CASES) {
		test(label, () => {
			const span = findTag(html, tag);
			expect(span === null ? null : html.slice(span.start, span.end)).toBe(
				expected,
			);
		});
	}

	test("continues from the `from` offset", () => {
		const html = '<link id="1"><link id="2">';
		const first = findTag(html, "link");
		expect(first).not.toBeNull();
		if (first === null) return;
		expect(html.slice(first.start, first.end)).toBe('<link id="1">');
		const second = findTag(html, "link", first.end);
		expect(second).not.toBeNull();
		if (second === null) return;
		expect(html.slice(second.start, second.end)).toBe('<link id="2">');
		expect(findTag(html, "link", second.end)).toBeNull();
	});

	test("current behavior: a quoted '>' ends the tag span", () => {
		const html = '<link rel="stylesheet" title="a>b" href="./style.css">';
		const span = findTag(html, "link");
		expect(span).not.toBeNull();
		if (span === null) return;
		expect(html.slice(span.start, span.end)).toBe(
			'<link rel="stylesheet" title="a>',
		);
	});

	test("current behavior: an unterminated tag swallows the next tag's '>'", () => {
		const html = '<link<link id="2">';
		const span = findTag(html, "link");
		expect(span).not.toBeNull();
		if (span === null) return;
		expect(html.slice(span.start, span.end)).toBe(html);
	});
});

interface InsertCase {
	readonly label: string;
	readonly html: string;
	readonly expected: string;
}

const FRAGMENT = '<meta id="f" />';

const INSERT_CASES: readonly InsertCase[] = [
	{
		label: "inserts at the start of an existing head",
		html: "<html><head><title>t</title></head><body></body></html>",
		expected: `<html><head>${FRAGMENT}<title>t</title></head><body></body></html>`,
	},
	{
		label: "head with attributes",
		html: '<html><head class="a"><title>t</title></head></html>',
		expected: `<html><head class="a">${FRAGMENT}<title>t</title></head></html>`,
	},
	{
		label: "head with a space before the bracket",
		html: "<html><head ></head></html>",
		expected: `<html><head >${FRAGMENT}</head></html>`,
	},
	{
		label: "uppercase HEAD",
		html: "<HTML><HEAD><TITLE>t</TITLE></HEAD></HTML>",
		expected: `<HTML><HEAD>${FRAGMENT}<TITLE>t</TITLE></HEAD></HTML>`,
	},
	{
		label: "<header> is not a head: a head is created after <html>",
		html: "<html><body><header>x</header></body></html>",
		expected: `<html><head>${FRAGMENT}</head><body><header>x</header></body></html>`,
	},
	{
		label: "uppercase HTML with no head",
		html: "<HTML><BODY></BODY></HTML>",
		expected: `<HTML><head>${FRAGMENT}</head><BODY></BODY></HTML>`,
	},
	{
		label: "<htmlx> is not html: falls through to the doctype branch",
		html: "<!doctype html><htmlx></htmlx>",
		expected: `<!doctype html><head>${FRAGMENT}</head><htmlx></htmlx>`,
	},
	{
		label: "uppercase doctype",
		html: "<!DOCTYPE HTML><div></div>",
		expected: `<!DOCTYPE HTML><head>${FRAGMENT}</head><div></div>`,
	},
	{
		label: "<!doctypex> is not a doctype",
		html: "<!doctypex html><div></div>",
		expected: `<head>${FRAGMENT}</head><!doctypex html><div></div>`,
	},
	{
		label: "no head, html or doctype",
		html: "<div>x</div>",
		expected: `<head>${FRAGMENT}</head><div>x</div>`,
	},
];

describe("insertAtHeadStart", () => {
	for (const { label, html, expected } of INSERT_CASES) {
		test(label, () => {
			expect(insertAtHeadStart(html, FRAGMENT)).toBe(expected);
		});
	}
});

interface InlineCase {
	readonly label: string;
	readonly html: string;
	readonly expected: string;
	readonly inlined?: readonly string[];
	readonly external?: readonly string[];
}

const INLINE_CASES: readonly InlineCase[] = [
	{
		label: "rewrites a shared module script (multi-widget build shape)",
		html: '<!doctype html><html><head><script type="module" crossorigin src="/shared/react-1.js"></script></head></html>',
		expected:
			'<!doctype html><html><head><script type="module" crossorigin src="../../shared/react-1.js"></script></head></html>',
		external: ["shared/react-1.js"],
	},
	{
		label: "preserves the body of a shared script",
		html: '<script src="/shared/a.js">console.log(1)</script>',
		expected: '<script src="../../shared/a.js">console.log(1)</script>',
		external: ["shared/a.js"],
	},
	{
		label: "</scriptx> does not close a script",
		html: '<script src="/shared/a.js">var s = "</scriptx>";</script>',
		expected: '<script src="../../shared/a.js">var s = "</scriptx>";</script>',
		external: ["shared/a.js"],
	},
	{
		label: "'</script >' close form",
		html: '<script src="/shared/a.js"></script >',
		expected: '<script src="../../shared/a.js"></script>',
		external: ["shared/a.js"],
	},
	{
		label: "'</script\\t>' close form",
		html: '<script src="/shared/a.js"></script\t>',
		expected: '<script src="../../shared/a.js"></script>',
		external: ["shared/a.js"],
	},
	{
		label: "close tag carrying attributes",
		html: '<script src="/shared/a.js"></script data-x="1">',
		expected: '<script src="../../shared/a.js"></script>',
		external: ["shared/a.js"],
	},
	{
		label: "uppercase SCRIPT open and close",
		html: '<SCRIPT SRC="/shared/a.js"></SCRIPT>',
		expected: '<script SRC="../../shared/a.js"></script>',
		external: ["shared/a.js"],
	},
	{
		label: "mixed case script open and close",
		html: '<ScRiPt src="/shared/a.js"></ScRiPt>',
		expected: '<script src="../../shared/a.js"></script>',
		external: ["shared/a.js"],
	},
	{
		label: "two shared scripts around a plain inline script",
		html: '<script src="/shared/a.js"></script><script>boot();</script><script src="/shared/b.js"></script>',
		expected:
			'<script src="../../shared/a.js"></script><script>boot();</script><script src="../../shared/b.js"></script>',
		external: ["shared/a.js", "shared/b.js"],
	},
	{
		label: "a script with no close tag is left alone",
		html: '<head></head><script src="./index.js">',
		expected: '<head></head><script src="./index.js">',
	},
	{
		label: "an unterminated script tag is left alone",
		html: '<head></head><script src="./index.js"',
		expected: '<head></head><script src="./index.js"',
	},
	{
		label: "uppercase LINK stylesheet is inlined",
		html: '<head><LINK REL="stylesheet" HREF="./style.css" /></head>',
		expected: "<head><style>#root { color: red; }</style></head>",
		inlined: ["style.css"],
	},
	{
		label: "<linkage> is not a link",
		html: '<head><linkage rel="stylesheet" href="./style.css"></head>',
		expected: '<head><linkage rel="stylesheet" href="./style.css"></head>',
	},
	{
		label: "self-closing link without a space",
		html: '<head><link rel="stylesheet" href="./style.css"/></head>',
		expected: "<head><style>#root { color: red; }</style></head>",
		inlined: ["style.css"],
	},
	{
		label: "a link with no href is left alone",
		html: "<head><link /></head>",
		expected: "<head><link /></head>",
	},
	{
		label: "current behavior: a quoted '>' in a link truncates the tag span",
		html: '<link rel="stylesheet" href="./style.css" title="a>b">',
		expected: '<style>#root { color: red; }</style>b">',
		inlined: ["style.css"],
	},
	{
		label: "current behavior: a quoted '>' in a script truncates the open tag",
		html: '<script src="./index.js" data-x="a>b"></script>',
		expected: '<script>console.log("entry");</script>',
		inlined: ["index.js"],
	},
];

describe("inlineHtml scanning", () => {
	for (const { label, html, expected, inlined, external } of INLINE_CASES) {
		test(label, () => {
			const result = inlineHtml(html, resolveAsset);
			expect(result.html).toBe(expected);
			expect(result.inlined).toEqual([...(inlined ?? [])]);
			expect(result.external).toEqual([...(external ?? [])]);
		});
	}

	test("realistic vite build document", () => {
		const html = [
			"<!doctype html>",
			'<html lang="en">',
			"\t<head>",
			'\t\t<meta charset="utf-8" />',
			'\t\t<link rel="modulepreload" crossorigin href="/shared/react-1.js" />',
			'\t\t<link rel="stylesheet" href="./style.css" />',
			"\t</head>",
			"\t<body>",
			'\t\t<div id="root"></div>',
			'\t\t<script type="module" crossorigin src="/shared/react-1.js"></script>',
			'\t\t<script type="module" src="./index.js"></script>',
			"\t</body>",
			"</html>",
		].join("\n");
		const expected = [
			"<!doctype html>",
			'<html lang="en">',
			"\t<head>",
			'\t\t<meta charset="utf-8" />',
			'\t\t<link rel="modulepreload" crossorigin href="../../shared/react-1.js" />',
			"\t\t<style>#root { color: red; }</style>",
			"\t</head>",
			"\t<body>",
			'\t\t<div id="root"></div>',
			'\t\t<script type="module" crossorigin src="../../shared/react-1.js"></script>',
			'\t\t<script type="module">console.log("entry");</script>',
			"\t</body>",
			"</html>",
		].join("\n");

		const result = inlineHtml(html, resolveAsset);
		expect(result.html).toBe(expected);
		expect(result.inlined).toEqual(["index.js", "style.css"]);
		expect(result.external).toEqual(["shared/react-1.js"]);
	});
});

describe("injectContractScript", () => {
	test("injects exactly one contract into a document that has none", () => {
		const out = injectContractScript(
			"<html><head><title>t</title></head><body></body></html>",
			CONTRACT,
		);
		expect(out).toBe(
			`<html><head>${CONTRACT_TAG}<title>t</title></head><body></body></html>`,
		);
		expect(countContracts(out)).toBe(1);
	});

	test("replaces a previously injected contract, leaving no fragments", () => {
		const out = injectContractScript(
			'<html><head><script>globalThis.__FLW_CONTRACT__ = {"x":1};</script><title>t</title></head></html>',
			CONTRACT,
		);
		expect(out).toBe(
			`<html><head>${CONTRACT_TAG}<title>t</title></head></html>`,
		);
		expect(countContracts(out)).toBe(1);
	});

	test("skips leading whitespace inside the contract script", () => {
		const out = injectContractScript(
			'<html><head><script>\n\t\tglobalThis.__FLW_CONTRACT__ = {"x":1};\n\t</script><title>t</title></head></html>',
			CONTRACT,
		);
		expect(out).toBe(
			`<html><head>${CONTRACT_TAG}<title>t</title></head></html>`,
		);
		expect(countContracts(out)).toBe(1);
	});

	test("strips a contract that is not the first script in the document", () => {
		const out = injectContractScript(
			'<html><head><script src="./boot.js"></script><script>globalThis.__FLW_CONTRACT__ = {"x":1};</script></head></html>',
			CONTRACT,
		);
		expect(out).toBe(
			`<html><head>${CONTRACT_TAG}<script src="./boot.js"></script></head></html>`,
		);
		expect(countContracts(out)).toBe(1);
	});

	test("keeps a plain inline script that precedes the contract script", () => {
		const out = injectContractScript(
			'<html><head><script>boot();</script><script>globalThis.__FLW_CONTRACT__ = {"x":1};</script></head></html>',
			CONTRACT,
		);
		expect(out).toBe(
			`<html><head>${CONTRACT_TAG}<script>boot();</script></head></html>`,
		);
		expect(countContracts(out)).toBe(1);
	});

	test("a non-contract inline script is never stripped", () => {
		const out = injectContractScript(
			"<html><head><script>globalThisNot.__FLW_OTHER__ = 1;</script></head></html>",
			CONTRACT,
		);
		expect(out).toBe(
			`<html><head>${CONTRACT_TAG}<script>globalThisNot.__FLW_OTHER__ = 1;</script></head></html>`,
		);
		expect(countContracts(out)).toBe(1);
	});

	test("current behavior: only the first of two contract scripts is stripped", () => {
		const dup =
			'<script>globalThis.__FLW_CONTRACT__ = {"x":1};</script><script>globalThis.__FLW_CONTRACT__ = {"y":2};</script>';
		const out = injectContractScript(
			`<html><head>${dup}</head></html>`,
			CONTRACT,
		);
		expect(out).toBe(
			`<html><head>${CONTRACT_TAG}<script>globalThis.__FLW_CONTRACT__ = {"y":2};</script></head></html>`,
		);
		expect(countContracts(out)).toBe(2);
	});

	test("current behavior: a contract script with attributes is not stripped", () => {
		const out = injectContractScript(
			'<html><head><script type="text/javascript">globalThis.__FLW_CONTRACT__ = {"x":1};</script></head></html>',
			CONTRACT,
		);
		expect(countContracts(out)).toBe(2);
	});

	test("current behavior: an uppercase contract SCRIPT is not stripped", () => {
		const out = injectContractScript(
			'<html><head><SCRIPT>globalThis.__FLW_CONTRACT__ = {"x":1};</SCRIPT></head></html>',
			CONTRACT,
		);
		expect(countContracts(out)).toBe(2);
	});

	test("an unterminated contract script is left in place", () => {
		const html =
			'<html><head><script>globalThis.__FLW_CONTRACT__ = {"x":1};</head></html>';
		const out = injectContractScript(html, CONTRACT);
		expect(out).toContain('globalThis.__FLW_CONTRACT__ = {"x":1};');
		expect(countContracts(out)).toBe(2);
	});
});
