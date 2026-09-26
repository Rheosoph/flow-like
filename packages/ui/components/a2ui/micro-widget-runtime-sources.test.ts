import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import type {
	PlatformStorageScope,
	WidgetNetworkInputSlot,
	WidgetRuntimeSourceRequest,
} from "./micro-widget-policy";
import {
	RUNTIME_CROSS_SLOT_ISSUE_SLOT,
	RUNTIME_EXTRACTION_LIMITS,
	RUNTIME_VALUE_ISSUE_CODES,
	type RuntimeValueIssue,
	extractRuntimeSources,
} from "./micro-widget-runtime-sources";

interface ExtractionCase {
	name: string;
	slots: string[];
	appId: string | null;
	excluded: string[];
	props: Record<string, unknown>;
	expected: {
		request: WidgetRuntimeSourceRequest[];
		issues: RuntimeValueIssue[];
	};
}

const fixture = JSON.parse(
	readFileSync(
		join(
			import.meta.dir,
			"../../../wasm/schema/tests/fixtures/widget_runtime_extraction.json",
		),
		"utf8",
	),
) as {
	limits: typeof RUNTIME_EXTRACTION_LIMITS;
	issueCodes: string[];
	crossSlotIssueSlot: string;
	slots: Record<string, WidgetNetworkInputSlot>;
	platformStorage: PlatformStorageScope[];
	cases: ExtractionCase[];
};

function generate(value: unknown): unknown {
	if (Array.isArray(value)) return value.map(generate);
	if (typeof value !== "object" || value === null) return value;
	const record = value as Record<string, unknown>;
	if ("$repeat" in record) {
		return Array.from({ length: record.count as number }, () =>
			generate(record.$repeat),
		);
	}
	if ("$string" in record) {
		const prefix = record.$string as string;
		return prefix.padEnd(record.length as number, record.fill as string);
	}
	return Object.fromEntries(
		Object.entries(record).map(([key, field]) => [key, generate(field)]),
	);
}

function run(entry: ExtractionCase) {
	return extractRuntimeSources(
		entry.slots.map((name) => {
			const slot = fixture.slots[name];
			if (!slot) throw new Error(`fixture has no slot ${name}`);
			return slot;
		}),
		fixture.platformStorage,
		entry.appId,
		generate(entry.props) as Record<string, unknown>,
		new Set(entry.excluded),
	);
}

describe("extractRuntimeSources", () => {
	test("limits, issue codes and the cross-slot slot match the fixture", () => {
		expect(fixture.limits).toEqual({ ...RUNTIME_EXTRACTION_LIMITS });
		expect(fixture.issueCodes).toEqual([...RUNTIME_VALUE_ISSUE_CODES]);
		expect(fixture.crossSlotIssueSlot).toBe(RUNTIME_CROSS_SLOT_ISSUE_SLOT);
	});

	for (const entry of fixture.cases) {
		test(entry.name, () => {
			const { request, issues } = run(entry);
			expect({ request, issues }).toEqual(entry.expected);
		});
	}

	test("the key identifies the request", () => {
		const keys = new Set(fixture.cases.map((entry) => run(entry).key));
		const requests = new Set(
			fixture.cases.map((entry) => JSON.stringify(run(entry).request)),
		);
		expect(keys.size).toBe(requests.size);
		const empty = fixture.cases.find(
			(entry) => entry.name === "unknown placeholder",
		);
		if (!empty) throw new Error("fixture case missing");
		expect(run(empty).key).toBe("[]");
	});

	test("issues never carry the values they describe", () => {
		for (const entry of fixture.cases) {
			for (const issue of run(entry).issues) {
				expect(Object.keys(issue).sort()).toEqual(["code", "count", "slot"]);
			}
		}
	});

	test("inputs the widget never receives are never read", () => {
		const slot: WidgetNetworkInputSlot = {
			path: "layers[].url",
			purpose: 0,
			directives: ["imgSrc"],
		};
		const inherited = Object.create({
			layers: [{ url: "https://inherited.example.com/" }],
		}) as Record<string, unknown>;
		expect(
			extractRuntimeSources([slot], [], null, inherited, new Set()).request,
		).toEqual([]);
		const accessor = {
			get layers() {
				return [{ url: "https://getter.example.com/" }];
			},
		};
		expect(
			extractRuntimeSources([slot], [], null, accessor, new Set()).request,
		).toEqual([
			{ slot: "layers[].url", sources: ["https://getter.example.com"] },
		]);
	});

	test("switch values must be host labels", () => {
		const slot: WidgetNetworkInputSlot = {
			path: "tileUrl",
			purpose: 0,
			directives: ["imgSrc"],
			template: {},
		};
		const extract = (tileUrl: string) =>
			extractRuntimeSources([slot], [], null, { tileUrl }, new Set());
		expect(
			extract("https://{switch:evil.example/x,a}.maps.example.org/").issues,
		).toEqual([{ slot: "tileUrl", code: "template-unexpanded", count: 1 }]);
		expect(extract("https://{a-c.maps.example.org/").issues).toEqual([
			{ slot: "tileUrl", code: "template-unexpanded", count: 1 },
		]);
		expect(extract("https://{a-c}}.maps.example.org/").issues).toEqual([
			{ slot: "tileUrl", code: "template-unexpanded", count: 1 },
		]);
	});

	test("platform storage needs a well-formed app id", () => {
		const slot: WidgetNetworkInputSlot = {
			path: "url",
			purpose: 0,
			directives: ["imgSrc"],
		};
		const result = extractRuntimeSources(
			[slot],
			fixture.platformStorage,
			"../app",
			{
				url: "https://s3.eu-central-1.amazonaws.com/flow-like-content/apps/../app/x.png",
			},
			new Set(),
		);
		expect(result.request).toEqual([]);
		expect(result.issues).toEqual([
			{ slot: "url", code: "platform-storage", count: 1 },
		]);
	});
});
