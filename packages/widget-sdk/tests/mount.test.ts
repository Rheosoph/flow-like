import { describe, expect, test } from "bun:test";
import { CONTRACT_VERSION, type WidgetContract } from "../src/contract";
import { defineWidget } from "../src/define";
import {
	INIT_TIMEOUT_MS,
	acceptEnvelope,
	filterPropsPatch,
	mergeInitProps,
	mountFlowWidget,
} from "../src/mount";
import { createEnvelope } from "../src/protocol";
import { applyTheme } from "../src/theme";

const contract: WidgetContract = {
	contractVersion: CONTRACT_VERSION,
	id: "sales-chart",
	inputs: {
		title: { type: "string", default: "Sales" },
		variant: { type: "enum", choices: ["bar", "line"], default: "bar" },
		limit: { type: "number", min: 1, max: 500, default: 50 },
		rows: { type: "json", schema: { type: "array" } },
	},
};

describe("acceptEnvelope", () => {
	const init = createEnvelope(
		"init",
		{
			props: {},
			theme: { mode: "light", tokens: {} },
			locale: "en",
			instanceId: "i-1",
			capabilities: {},
		},
		"nonce-1",
		"i-1",
	);

	test("drops messages not coming from the parent window", () => {
		expect(acceptEnvelope(init, null, false)).toBeNull();
	});

	test("drops non-envelope data", () => {
		expect(acceptEnvelope({ hello: true }, null, true)).toBeNull();
		expect(acceptEnvelope("init", null, true)).toBeNull();
		expect(acceptEnvelope(null, null, true)).toBeNull();
	});

	test("before init only accepts init", () => {
		expect(acceptEnvelope(init, null, true)).toBe(init);
		const update = createEnvelope(
			"props:update",
			{ props: {} },
			"nonce-1",
			"i-1",
		);
		expect(acceptEnvelope(update, null, true)).toBeNull();
	});

	test("after init drops nonce mismatches", () => {
		const update = createEnvelope(
			"props:update",
			{ props: {} },
			"other-nonce",
			"i-1",
		);
		expect(acceptEnvelope(update, "nonce-1", true)).toBeNull();
	});

	test("after init accepts matching nonce", () => {
		const update = createEnvelope(
			"props:update",
			{ props: { title: "Q3" } },
			"nonce-1",
			"i-1",
		);
		expect(acceptEnvelope(update, "nonce-1", true)).toBe(update);
	});

	test("drops envelopes with a wrong protocol", () => {
		const forged = { ...init, protocol: "flw/2" };
		expect(acceptEnvelope(forged, null, true)).toBeNull();
	});
});

describe("mergeInitProps", () => {
	test("contract defaults fill missing props", () => {
		expect(mergeInitProps(contract, { title: "Q3" })).toEqual({
			title: "Q3",
			variant: "bar",
			limit: 50,
		});
	});

	test("init props win over defaults", () => {
		expect(mergeInitProps(contract, { variant: "line", rows: [] })).toEqual({
			title: "Sales",
			variant: "line",
			limit: 50,
			rows: [],
		});
	});

	test("no contract means init props only", () => {
		expect(mergeInitProps(undefined, { a: 1 })).toEqual({ a: 1 });
	});
});

describe("filterPropsPatch", () => {
	test("passes everything through without a contract", () => {
		const { accepted, rejected } = filterPropsPatch(undefined, {
			anything: [1, 2],
		});
		expect(accepted).toEqual({ anything: [1, 2] });
		expect(rejected).toEqual([]);
	});

	test("accepts valid values", () => {
		const { accepted, rejected } = filterPropsPatch(contract, {
			title: "Q3",
			variant: "line",
			limit: 10,
			rows: [{ x: "Q1", y: 1 }],
		});
		expect(accepted).toEqual({
			title: "Q3",
			variant: "line",
			limit: 10,
			rows: [{ x: "Q1", y: 1 }],
		});
		expect(rejected).toEqual([]);
	});

	test("drops invalid values and names the key", () => {
		const { accepted, rejected } = filterPropsPatch(contract, {
			title: 42,
			variant: "pie",
			limit: 1000,
			rows: "not-an-array",
		});
		expect(accepted).toEqual({});
		expect(rejected.map((entry) => entry.key).sort()).toEqual([
			"limit",
			"rows",
			"title",
			"variant",
		]);
		for (const entry of rejected) {
			expect(entry.errors.length).toBeGreaterThan(0);
		}
	});

	test("drops keys not declared in the contract", () => {
		const { accepted, rejected } = filterPropsPatch(contract, {
			title: "ok",
			unknownKey: true,
		});
		expect(accepted).toEqual({ title: "ok" });
		expect(rejected).toEqual([
			{ key: "unknownKey", errors: ["not declared in the contract"] },
		]);
	});

	test("mixes accepted and rejected entries", () => {
		const { accepted, rejected } = filterPropsPatch(contract, {
			limit: 5,
			variant: "donut",
		});
		expect(accepted).toEqual({ limit: 5 });
		expect(rejected.length).toBe(1);
		expect(rejected[0]?.key).toBe("variant");
	});

	test("keeps undefined as a deletion for optional inputs", () => {
		const contractWithOptional: WidgetContract = {
			...contract,
			inputs: {
				...contract.inputs,
				note: { type: "string", optional: true },
			},
		};
		const { accepted, rejected } = filterPropsPatch(contractWithOptional, {
			note: undefined,
			title: undefined,
		});

		expect(accepted).toEqual({ note: undefined });
		expect(rejected).toEqual([
			{ key: "title", errors: ["$: value is required"] },
		]);
	});
});

describe("non-DOM environment guards", () => {
	test("INIT_TIMEOUT_MS is 300", () => {
		expect(INIT_TIMEOUT_MS).toBe(300);
	});

	test("mountFlowWidget requires a window", () => {
		expect(typeof window).toBe("undefined");
		const definition = defineWidget({
			id: "sales-chart",
			name: "Sales Chart",
			description: "test",
		});
		expect(() => mountFlowWidget(definition)).toThrow(
			"mountFlowWidget requires a browser environment",
		);
	});

	test("applyTheme is a no-op without a document", () => {
		expect(() =>
			applyTheme({ mode: "dark", tokens: { "--background": "black" } }),
		).not.toThrow();
	});
});
