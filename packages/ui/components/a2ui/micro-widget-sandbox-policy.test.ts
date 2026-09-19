import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { WidgetPolicy } from "./micro-widget-policy";
import {
	describeMicroWidgetSandbox,
	isMicroWidgetLocalSource,
	isMicroWidgetNetworkSource,
	serializeMicroWidgetCsp,
} from "./micro-widget-sandbox-policy";

interface DocumentCspCase {
	name: string;
	bundleSources: string[];
	policy: WidgetPolicy;
	includeSandbox: boolean;
	localMedia?: boolean;
	expected: string;
}

const fixture = JSON.parse(
	readFileSync(
		join(
			import.meta.dir,
			"../../../wasm/schema/tests/fixtures/widget_csp.json",
		),
		"utf8",
	),
) as { documentCsp: DocumentCspCase[] };

/** These cases exercise the Rust builder's input hardening; descriptors never carry such values. */
const BACKEND_HARDENING_CASES = new Set([
	"invalid and duplicate bundle sources are dropped",
	"policy with invalid csp contributes no sources",
]);

const BUNDLE = [
	"flow-widget://localhost/com.example.maps/a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90/",
];

function directive(name: string, policy: WidgetPolicy) {
	const row = describeMicroWidgetSandbox(policy).csp.find(
		(entry) => entry.directive === name,
	);
	if (!row) throw new Error(`missing ${name}`);
	return row;
}

function directiveNames(csp: string): string[] {
	return csp.split("; ").map((entry) => entry.split(" ")[0]);
}

describe("describeMicroWidgetSandbox", () => {
	test("lists the directives of the fixture baseline in the same order", () => {
		const baseline = fixture.documentCsp.find(
			(entry) => entry.name === "desktop baseline",
		);
		if (!baseline) throw new Error("fixture has no desktop baseline");
		const described = describeMicroWidgetSandbox({});
		expect([...described.csp.map((row) => row.directive), "sandbox"]).toEqual(
			directiveNames(baseline.expected),
		);
		expect(described.sandbox).toEqual([{ token: "allow-scripts" }]);
	});

	test("serializes exactly like the backend for every well-formed fixture case", () => {
		const cases = fixture.documentCsp.filter(
			(entry) => !BACKEND_HARDENING_CASES.has(entry.name),
		);
		expect(cases.length).toBeGreaterThan(5);
		for (const entry of cases) {
			expect({
				name: entry.name,
				csp: serializeMicroWidgetCsp(
					describeMicroWidgetSandbox(entry.policy, {
						localMedia: entry.localMedia,
					}),
					entry.bundleSources,
					entry.includeSandbox,
				),
			}).toEqual({ name: entry.name, csp: entry.expected });
		}
	});

	test("the baseline grants nothing and closes every non-extendable directive", () => {
		const policy = describeMicroWidgetSandbox({});
		expect(policy.csp.every((row) => row.grantedBy === undefined)).toBe(true);
		expect(policy.csp.every((row) => row.network === undefined)).toBe(true);
		for (const name of ["connect-src", "media-src"]) {
			expect(directive(name, {}).sources).toEqual(["data:", "blob:"]);
		}
		for (const name of [
			"worker-src",
			"frame-src",
			"child-src",
			"object-src",
			"manifest-src",
			"base-uri",
			"form-action",
		]) {
			expect(directive(name, {}).sources).toEqual(["'none'"]);
		}
		const csp = serializeMicroWidgetCsp(policy, BUNDLE, true);
		expect(csp).not.toContain("'self'");
		expect(csp).not.toContain("wasm-unsafe-eval");
		expect(csp).not.toContain("https:");
	});

	test("each capability opens exactly its directives", () => {
		expect(directive("worker-src", { workers: true })).toEqual({
			directive: "worker-src",
			sources: ["blob:", "bundle"],
			grantedBy: "workers",
		});
		expect(directive("connect-src", { workers: true })).toEqual({
			directive: "connect-src",
			sources: ["data:", "blob:", "bundle"],
			grantedBy: "workers",
		});
		expect(directive("connect-src", { media: true }).sources).toEqual([
			"data:",
			"blob:",
		]);
		expect(directive("media-src", { media: true })).toEqual({
			directive: "media-src",
			sources: ["data:", "blob:", "bundle"],
			grantedBy: "media",
		});
		expect(directive("script-src", { wasm: true })).toEqual({
			directive: "script-src",
			sources: ["'unsafe-inline'", "'wasm-unsafe-eval'", "bundle"],
			grantedBy: "wasm",
		});
		expect(describeMicroWidgetSandbox({ downloads: true }).sandbox).toEqual([
			{ token: "allow-scripts" },
			{ token: "allow-downloads", grantedBy: "downloads" },
		]);
		expect(describeMicroWidgetSandbox({ microphone: true })).toEqual(
			describeMicroWidgetSandbox({}),
		);
	});

	test("approved hosts appear only in their own directive and are marked as network rows", () => {
		const policy: WidgetPolicy = {
			csp: {
				connectSrc: ["https://api.maptiler.com", "wss://live.example.com"],
				imgSrc: ["https://tiles.example.org"],
			},
		};
		expect(directive("connect-src", policy)).toEqual({
			directive: "connect-src",
			sources: [
				"data:",
				"blob:",
				"https://api.maptiler.com",
				"wss://live.example.com",
			],
			network: "connectSrc",
		});
		expect(directive("img-src", policy)).toEqual({
			directive: "img-src",
			sources: ["data:", "blob:", "bundle", "https://tiles.example.org"],
			network: "imgSrc",
		});
		expect(directive("worker-src", policy).sources).toEqual(["'none'"]);
		expect(directive("font-src", policy).network).toBeUndefined();
		const csp = serializeMicroWidgetCsp(
			describeMicroWidgetSandbox(policy),
			BUNDLE,
		);
		expect(csp.match(/api\.maptiler\.com/g)).toHaveLength(1);
		expect(csp.match(/tiles\.example\.org/g)).toHaveLength(1);
	});

	test("hosts combine with the capability that opens the same directive", () => {
		expect(
			directive("media-src", {
				media: true,
				csp: { mediaSrc: ["https://media.example.org"] },
			}),
		).toEqual({
			directive: "media-src",
			sources: ["data:", "blob:", "bundle", "https://media.example.org"],
			grantedBy: "media",
			network: "mediaSrc",
		});
	});

	test("local data is listed once, first, and never as a network source", () => {
		const policy: WidgetPolicy = {
			workers: true,
			media: true,
			csp: {
				connectSrc: ["https://api.cesium.com"],
				mediaSrc: ["https://media.example.org"],
			},
		};
		for (const name of ["connect-src", "media-src"]) {
			const { sources } = directive(name, policy);
			expect(sources.slice(0, 2)).toEqual(["data:", "blob:"]);
			expect(sources.filter(isMicroWidgetLocalSource)).toEqual([
				"data:",
				"blob:",
			]);
			expect(sources.filter(isMicroWidgetNetworkSource)).toHaveLength(1);
		}
	});

	test("an engine without local media keeps media-src to the bundle and approved hosts", () => {
		const media = (policy: WidgetPolicy) =>
			describeMicroWidgetSandbox(policy, { localMedia: false }).csp.find(
				(row) => row.directive === "media-src",
			)?.sources;
		expect(media({})).toEqual(["'none'"]);
		expect(media({ media: true })).toEqual(["bundle"]);
		expect(media({ csp: { mediaSrc: ["https://media.example.org"] } })).toEqual(
			["https://media.example.org"],
		);
		expect(
			describeMicroWidgetSandbox({}, { localMedia: false }).csp.find(
				(row) => row.directive === "connect-src",
			)?.sources,
		).toEqual(["data:", "blob:"]);
		expect(describeMicroWidgetSandbox({}, { localMedia: true })).toEqual(
			describeMicroWidgetSandbox({}),
		);
	});

	test("network sources are told apart from keywords", () => {
		expect(isMicroWidgetNetworkSource("https://api.maptiler.com")).toBe(true);
		expect(isMicroWidgetNetworkSource("wss://live.example.com")).toBe(true);
		for (const keyword of ["'none'", "blob:", "data:", "bundle"]) {
			expect(isMicroWidgetNetworkSource(keyword)).toBe(false);
		}
	});
});
