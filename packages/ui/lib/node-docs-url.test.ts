import { describe, expect, test } from "bun:test";
import { buildNodeDocsUrl } from "./node-docs-url";
import type { INode } from "./schema/flow/node";

function node(overrides: Partial<INode>): INode {
	return {
		category: "AI/Agents",
		description: "",
		friendly_name: "Invoke Agent",
		id: "node-id",
		name: "agent_invoke",
		pins: {},
		...overrides,
	};
}

describe("buildNodeDocsUrl", () => {
	test("uses explicit docs urls when provided", () => {
		expect(
			buildNodeDocsUrl(
				node({
					docs: " https://example.com/custom-node-docs ",
				}),
			),
		).toBe("https://example.com/custom-node-docs");
	});

	test("rejects unsafe explicit docs urls", () => {
		expect(
			buildNodeDocsUrl(
				node({
					docs: "javascript:alert(1)",
				}),
			),
		).toBe("https://docs.flow-like.com/nodes/ai/agents/agent-invoke/");
	});

	test("matches generated docs slug format for catalog nodes", () => {
		expect(buildNodeDocsUrl(node({}))).toBe(
			"https://docs.flow-like.com/nodes/ai/agents/agent-invoke/",
		);
	});

	test("slugifies category segments and node names like the docs generator", () => {
		expect(
			buildNodeDocsUrl(
				node({
					category: "Web / API / Request",
					name: "http_set_bearer_auth",
				}),
			),
		).toBe(
			"https://docs.flow-like.com/nodes/web/api/request/http-set-bearer-auth/",
		);
	});

	test("falls back to the uncategorized node path for empty values", () => {
		expect(buildNodeDocsUrl(node({ category: " / ", name: "" }))).toBe(
			"https://docs.flow-like.com/nodes/uncategorized/node/",
		);
	});

	test("falls back safely when runtime node values are missing", () => {
		expect(
			buildNodeDocsUrl(
				node({
					category: null as unknown as string,
					name: null as unknown as string,
				}),
			),
		).toBe("https://docs.flow-like.com/nodes/uncategorized/node/");
	});
});
