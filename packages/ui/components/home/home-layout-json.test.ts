import { describe, expect, it, spyOn } from "bun:test";
import {
	MAX_HOME_LAYOUT_BYTES,
	MAX_HOME_WIDGET_CLASS_NAME_BYTES,
} from "./home-layout";
import {
	formatHomeLayoutJson,
	homeLayoutFingerprint,
	homeLayoutsEqual,
	parseHomeLayoutJson,
	serializeHomeLayout,
	trySerializeHomeLayout,
} from "./home-layout-json";
import {
	MAX_HOME_JSON_DEPTH,
	MAX_HOME_JSON_TEXT_BYTES,
} from "./home-layout-json-document";
import type { IHomeLayout } from "./types";

const layout: IHomeLayout = {
	version: 1,
	title: "Operations",
	widgets: [
		{
			id: "status",
			type: "information",
			title: "Status",
			size: { columns: 6, rows: 3 },
			appearance: { variant: "card", accent: "neutral" },
			config: { body: "Everything is running." },
		},
	],
};

describe("home layout JSON transfer", () => {
	it("serializes and parses a portable layout", () => {
		const source = serializeHomeLayout(layout);
		expect(source).toContain('\n  "widgets": [');
		expect(parseHomeLayoutJson(source)).toEqual({ ok: true, layout });
	});

	it("normalizes optional widget fields before applying pasted JSON", () => {
		const result = parseHomeLayoutJson(
			JSON.stringify({
				version: 1,
				widgets: [{ id: "future", type: "future-widget" }],
			}),
		);
		expect(result).toEqual({
			ok: true,
			layout: {
				version: 1,
				widgets: [
					{
						id: "future",
						type: "future-widget",
						size: { columns: 6, rows: 3 },
						appearance: { variant: "card", accent: "neutral" },
						config: {},
					},
				],
			},
		});
	});

	it("compares layouts by their persisted JSON value", () => {
		const left = structuredClone(layout);
		const right = structuredClone(layout);
		left.widgets[0].config = { first: 1, second: 2 };
		right.widgets[0].config = { second: 2, first: 1 };
		expect(homeLayoutsEqual(left, right)).toBe(true);
		right.widgets[0].title = "Different";
		expect(homeLayoutsEqual(left, right)).toBe(false);
	});

	it("fingerprints structurally equal config objects consistently", () => {
		const left = structuredClone(layout);
		const right = structuredClone(layout);
		left.widgets[0].config = { first: 1, second: 2 };
		right.widgets[0].config = { second: 2, first: 1 };
		expect(homeLayoutFingerprint(left)).toBe(homeLayoutFingerprint(right));
		right.widgets[0].config.second = 3;
		expect(homeLayoutFingerprint(left)).not.toBe(homeLayoutFingerprint(right));
	});

	it("rejects malformed JSON and layouts with ambiguous widget identity", () => {
		expect(parseHomeLayoutJson("{")).toMatchObject({
			ok: false,
			error: expect.stringContaining("Invalid JSON"),
		});
		expect(
			parseHomeLayoutJson(
				JSON.stringify({
					...layout,
					widgets: [layout.widgets[0], layout.widgets[0]],
				}),
			),
		).toMatchObject({
			ok: false,
			error: expect.stringContaining("unique id"),
		});
	});

	it("keeps widget Tailwind classes through export, import and fingerprinting", () => {
		const styled = structuredClone(layout);
		styled.widgets[0].appearance.className = "rounded-3xl [&_h2]:text-lg";
		expect(parseHomeLayoutJson(serializeHomeLayout(styled))).toEqual({
			ok: true,
			layout: styled,
		});
		expect(homeLayoutFingerprint(styled)).not.toBe(
			homeLayoutFingerprint(layout),
		);
		const unstyled = structuredClone(styled);
		unstyled.widgets[0].appearance.className = "   ";
		const parsed = parseHomeLayoutJson(JSON.stringify(unstyled));
		expect(parsed.ok && homeLayoutFingerprint(parsed.layout)).toBe(
			homeLayoutFingerprint(layout),
		);
	});

	it("rejects a pasted layout that cannot fit the save limit", () => {
		const oversized = structuredClone(layout);
		oversized.widgets[0].config.body = "x".repeat(MAX_HOME_LAYOUT_BYTES);
		expect(parseHomeLayoutJson(JSON.stringify(oversized))).toEqual({
			ok: false,
			error: "This layout exceeds the 128 KiB save limit.",
		});
	});

	it("rejects text that the backend cannot persist", () => {
		const invalidValues: Array<{
			update: (value: IHomeLayout) => void;
			error: string;
		}> = [
			{
				update: (value) => {
					value.title = "x".repeat(257);
				},
				error: "Layout title must not exceed 256 bytes.",
			},
			{
				update: (value) => {
					value.widgets[0].id = "é".repeat(65);
				},
				error: "Widget 1 id must not exceed 128 bytes.",
			},
			{
				update: (value) => {
					value.widgets[0].type = "x".repeat(81);
				},
				error: "Widget 1 type must not exceed 80 bytes.",
			},
			{
				update: (value) => {
					value.widgets[0].appearance.variant = "";
				},
				error: "Widget 1 appearance variant must contain 1 to 80 bytes.",
			},
			{
				update: (value) => {
					value.widgets[0].appearance.className = "p".repeat(
						MAX_HOME_WIDGET_CLASS_NAME_BYTES + 1,
					);
				},
				error: `Widget 1 appearance className must not exceed ${MAX_HOME_WIDGET_CLASS_NAME_BYTES} bytes.`,
			},
			{
				update: (value) => {
					value.widgets[0].description = "x".repeat(2001);
				},
				error: "Widget 1 description must not exceed 2000 bytes.",
			},
		];

		for (const invalid of invalidValues) {
			const value = structuredClone(layout);
			invalid.update(value);
			expect(parseHomeLayoutJson(JSON.stringify(value))).toEqual({
				ok: false,
				error: invalid.error,
			});
		}
	});

	it("rejects numeric overflow before formatting or applying", () => {
		const source = JSON.stringify({
			...layout,
			widgets: [{ ...layout.widgets[0], config: { score: 1 } }],
		}).replace('"score":1', '"score":1e400');
		expect(parseHomeLayoutJson(source)).toMatchObject({
			ok: false,
			error: expect.stringContaining("finite"),
		});
		expect(formatHomeLayoutJson(source)).toMatchObject({
			ok: false,
			error: expect.stringContaining("finite"),
		});
	});

	it("rejects 20 KB of deeply nested JSON without a stack overflow", () => {
		const source = `{"version":1,"widgets":[{"id":"future","type":"future","config":{"tree":${"[".repeat(10_000)}0${"]".repeat(10_000)}}}]}`;
		expect(source.length).toBeGreaterThan(20_000);
		expect(source.length).toBeLessThan(21_000);
		for (const parse of [parseHomeLayoutJson, formatHomeLayoutJson]) {
			expect(parse(source)).toMatchObject({
				ok: false,
				error: expect.stringContaining("nested"),
			});
		}
	});

	it("reserves nesting headroom for serde profile and request envelopes", () => {
		const nestedLayout = (depth: number) =>
			`{"version":1,"widgets":[{"id":"future","type":"future","config":{"tree":${"[".repeat(depth - 4)}0${"]".repeat(depth - 4)}}}]}`;
		const source = nestedLayout(MAX_HOME_JSON_DEPTH);
		expect(parseHomeLayoutJson(source).ok).toBe(true);
		const formatted = formatHomeLayoutJson(source);
		expect(formatted.ok).toBe(true);
		if (formatted.ok) expect(parseHomeLayoutJson(formatted.json).ok).toBe(true);
		for (const depth of [MAX_HOME_JSON_DEPTH + 1, 150]) {
			for (const parse of [parseHomeLayoutJson, formatHomeLayoutJson])
				expect(parse(nestedLayout(depth))).toMatchObject({
					ok: false,
					error: expect.stringContaining("nested"),
				});
		}
	});

	it("bounds a 400 KB wide array before expanding a traversal stack", () => {
		const source = `{"version":1,"widgets":[{"id":"future","type":"future","config":{"values":[${"0,".repeat(199_999)}0]}}]}`;
		expect(source.length).toBeGreaterThan(400_000);
		expect(source.length).toBeLessThan(401_000);
		for (const parse of [parseHomeLayoutJson, formatHomeLayoutJson])
			expect(parse(source)).toMatchObject({
				ok: false,
				error: expect.stringContaining("values"),
			});
	});

	it("preserves large valid arrays in unknown widget configs", () => {
		const value = structuredClone(layout);
		value.widgets[0].type = "future-widget";
		value.widgets[0].config = {
			values: Array.from({ length: 50_000 }, () => 0),
		};
		const source = JSON.stringify(value);
		expect(parseHomeLayoutJson(source)).toEqual({ ok: true, layout: value });
		const formatted = formatHomeLayoutJson(source);
		expect(formatted.ok).toBe(true);
		if (formatted.ok) expect(JSON.parse(formatted.json)).toEqual(value);
	});

	it("budgets indentation before expanding a compact document", () => {
		const source = `${"[".repeat(MAX_HOME_JSON_DEPTH)}${"0,".repeat(8_999)}0${"]".repeat(MAX_HOME_JSON_DEPTH)}`;
		expect(source.length).toBeLessThan(20_000);
		for (const parse of [parseHomeLayoutJson, formatHomeLayoutJson])
			expect(parse(source)).toMatchObject({
				ok: false,
				error: expect.stringContaining("Formatted JSON exceeds"),
			});
	});

	it("caps raw input by UTF-8 bytes, including formatting whitespace", () => {
		for (const source of [
			`${" ".repeat(MAX_HOME_JSON_TEXT_BYTES)}{}`,
			JSON.stringify({ text: "é".repeat(MAX_HOME_JSON_TEXT_BYTES / 2) }),
		]) {
			for (const parse of [parseHomeLayoutJson, formatHomeLayoutJson])
				expect(parse(source)).toEqual({
					ok: false,
					error: "JSON exceeds the 1 MiB editor input limit.",
				});
		}
	});

	it("rejects lone surrogates in nested values and object keys", () => {
		for (const invalid of ["\ud800", "\udfff", "\ud800x", "x\udfff"]) {
			for (const config of [{ text: invalid }, { [invalid]: "value" }]) {
				const value = structuredClone(layout);
				value.widgets[0].config = config;
				const source = JSON.stringify(value);
				for (const parse of [parseHomeLayoutJson, formatHomeLayoutJson])
					expect(parse(source)).toMatchObject({
						ok: false,
						error: expect.stringContaining("Unicode"),
					});
				expect(trySerializeHomeLayout(value)).toMatchObject({ ok: false });
			}
		}
		// JSON.parse also accepts a literal unpaired UTF-16 code unit, without an escape.
		expect(
			formatHomeLayoutJson(`{"text":"${String.fromCharCode(0xd800)}"}`),
		).toMatchObject({ ok: false, error: expect.stringContaining("Unicode") });
	});

	it("round trips emoji, escaped surrogate pairs, and structural characters in text", () => {
		const value = structuredClone(layout);
		value.widgets[0].config = {
			"🚀": {
				text: `😀 é \"${"[{}]".repeat(200)}\\\"`,
				controls: "\b\t\n\f\r\u0001",
			},
		};
		const source = JSON.stringify(value).replaceAll("😀", "\\ud83d\\ude00");
		const formatted = formatHomeLayoutJson(source);
		expect(formatted.ok).toBe(true);
		if (!formatted.ok) throw new Error(formatted.error);
		expect(formatted.json).toBe(JSON.stringify(value, null, 2));
		expect(parseHomeLayoutJson(formatted.json)).toEqual({
			ok: true,
			layout: value,
		});
	});

	it("does not hide invalid serde values behind duplicate object keys", () => {
		for (const source of [
			'{"version":1,"widgets":[],"unused":"\\ud800","unused":"safe"}',
			'{"version":1,"widgets":[],"unused":1e400,"unused":1}',
		])
			for (const parse of [parseHomeLayoutJson, formatHomeLayoutJson])
				expect(parse(source)).toMatchObject({ ok: false });
	});

	it("returns serialization errors for invalid in-memory data", () => {
		const cyclic = structuredClone(layout);
		cyclic.widgets[0].config.self = cyclic;
		expect(trySerializeHomeLayout(cyclic)).toMatchObject({ ok: false });
		const unsupported = structuredClone(layout);
		unsupported.widgets[0].config.value = 1n;
		expect(trySerializeHomeLayout(unsupported)).toMatchObject({ ok: false });
		const throwing = structuredClone(layout);
		Object.defineProperty(throwing.widgets[0].config, "broken", {
			enumerable: true,
			get: () => {
				throw new Error("fixture getter");
			},
		});
		expect(trySerializeHomeLayout(throwing)).toMatchObject({ ok: false });
	});

	it("returns an error even if native formatting unexpectedly throws", () => {
		const stringify = spyOn(JSON, "stringify").mockImplementation(() => {
			throw new RangeError("fixture formatter failure");
		});
		let result: ReturnType<typeof formatHomeLayoutJson>;
		try {
			result = formatHomeLayoutJson('{"text":"safe"}');
		} finally {
			stringify.mockRestore();
		}
		expect(result).toEqual({
			ok: false,
			error: "Could not format this JSON document.",
		});
	});
});
