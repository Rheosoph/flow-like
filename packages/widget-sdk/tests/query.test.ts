import { describe, expect, test } from "bun:test";
import type { WidgetContract } from "../src/contract";
import { dispatchWidgetQuery } from "../src/query";

const contract: WidgetContract = {
	contractVersion: 1,
	id: "map",
	queries: {
		upsertEntities: {
			mutation: true,
			argsSchema: {
				type: "object",
				properties: { commandId: { type: "string" } },
				required: ["commandId"],
			},
			resultSchema: {
				type: "object",
				properties: { accepted: { type: "boolean" } },
				required: ["accepted"],
			},
		},
		getView: {
			argsSchema: { type: "string" },
			resultSchema: { type: "string" },
		},
	},
};

describe("dispatchWidgetQuery", () => {
	test("rejects invalid mutation arguments before invoking the handler", async () => {
		let calls = 0;
		const result = await dispatchWidgetQuery(
			contract,
			new Map([
				[
					"upsertEntities",
					() => {
						calls += 1;
						return { accepted: true };
					},
				],
			]),
			{ queryId: "q-1", name: "upsertEntities", args: {} },
		);

		expect(calls).toBe(0);
		expect(result.ok).toBeFalse();
		expect(result.error).toContain("Invalid arguments for mutation");
	});

	test("invokes valid mutations and validates their acknowledgement", async () => {
		const valid = await dispatchWidgetQuery(
			contract,
			new Map([["upsertEntities", () => ({ accepted: true })]]),
			{
				queryId: "q-2",
				name: "upsertEntities",
				args: { commandId: "update-1" },
			},
		);
		expect(valid).toEqual({
			queryId: "q-2",
			ok: true,
			value: { accepted: true },
		});

		const invalid = await dispatchWidgetQuery(
			contract,
			new Map([["upsertEntities", () => ({ accepted: "yes" })]]),
			{
				queryId: "q-3",
				name: "upsertEntities",
				args: { commandId: "update-2" },
			},
		);
		expect(invalid.ok).toBeFalse();
		expect(invalid.error).toContain("Invalid result from mutation");
	});

	test("keeps legacy read queries permissive", async () => {
		const result = await dispatchWidgetQuery(
			contract,
			new Map([["getView", (args) => ({ received: args })]]),
			{ queryId: "q-4", name: "getView", args: 42 },
		);
		expect(result).toEqual({
			queryId: "q-4",
			ok: true,
			value: { received: 42 },
		});
	});

	test("requires a registered handler", async () => {
		const result = await dispatchWidgetQuery(contract, new Map(), {
			queryId: "q-5",
			name: "upsertEntities",
			args: { commandId: "update-3" },
		});
		expect(result).toEqual({
			queryId: "q-5",
			ok: false,
			error: 'Unknown query "upsertEntities"',
		});
	});
});
