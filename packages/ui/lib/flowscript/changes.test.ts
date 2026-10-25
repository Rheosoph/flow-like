import { describe, expect, test } from "bun:test";
import { summarizeFlowScriptChanges } from "./changes";

describe("FlowScript change summaries", () => {
	test("retains a description-only apply and its before/after values", () => {
		const changed = {
			node_id: "tool",
			node_type: "events_generic",
			before: "",
			after: "Look up an order by its ID.",
		};
		const cleared = { ...changed, before: changed.after, after: "" };
		expect(
			summarizeFlowScriptChanges(
				{ descriptions_changed: [changed, cleared] },
				1,
			),
		).toEqual({ descriptions_changed: { total: 2, items: [changed] } });
		expect(
			summarizeFlowScriptChanges({ descriptions_changed: [cleared] }),
		).toEqual({ descriptions_changed: { total: 1, items: [cleared] } });
	});
});
