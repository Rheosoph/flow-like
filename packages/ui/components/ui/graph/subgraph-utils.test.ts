import { describe, expect, test } from "bun:test";
import type {
	SubgraphEdge,
	SubgraphNode,
} from "../../../state/backend-state/graph-state";
import {
	DEFAULT_LABEL_STYLE,
	buildOverlayFromSubgraph,
	enrichSubgraphWithStyles,
} from "./subgraph-utils";

function node(id: string, label: string): SubgraphNode {
	return { id, label, props: {} };
}

function edge(id: string, label: string): SubgraphEdge {
	return { id, source: "a", target: "b", label, props: {} };
}

describe("buildOverlayFromSubgraph", () => {
	test("derives one mapping per distinct label, in first-seen order", () => {
		const overlay = buildOverlayFromSubgraph(
			[node("a", "Person"), node("b", "Team"), node("c", "Person")],
			[edge("e1", "MEMBER_OF"), edge("e2", "MEMBER_OF")],
		);

		expect(overlay.nodes.map((mapping) => mapping.label)).toEqual([
			"Person",
			"Team",
		]);
		expect(overlay.edges.map((mapping) => mapping.label)).toEqual([
			"MEMBER_OF",
		]);
	});

	test("keeps a label's generated colour stable across rebuilds", () => {
		const first = buildOverlayFromSubgraph([node("a", "Person")], []);
		const second = buildOverlayFromSubgraph(
			[node("z", "Person"), node("y", "Team")],
			[],
		);

		expect(second.nodes[0].style.color).toBe(first.nodes[0].style.color);
	});

	test("applies supplied label styles over the generated ones", () => {
		const overlay = buildOverlayFromSubgraph([node("a", "Person")], [], {
			labelStyles: {
				Person: { color: "#ff0000", icon: "user", size: { mode: "fixed" } },
			},
		});

		expect(overlay.nodes[0].style.color).toBe("#ff0000");
		expect(overlay.nodes[0].style.icon).toBe("user");
	});

	test("ignores labelless nodes and edges", () => {
		const overlay = buildOverlayFromSubgraph(
			[node("a", ""), node("b", "Person")],
			[edge("e1", "")],
		);

		expect(overlay.nodes).toHaveLength(1);
		expect(overlay.edges).toHaveLength(0);
	});
});

describe("enrichSubgraphWithStyles", () => {
	test("resolves styles by label and falls back to the neutral default", () => {
		const overlay = buildOverlayFromSubgraph([node("a", "Person")], [], {
			labelStyles: {
				Person: { color: "#ff0000", icon: "user", size: { mode: "fixed" } },
			},
		});

		const enriched = enrichSubgraphWithStyles(
			{
				nodes: [node("a", "Person"), node("b", "Unmapped")],
				edges: [edge("e1", "Unmapped")],
				truncated: false,
			},
			overlay,
		);

		expect(enriched.nodes[0].style?.color).toBe("#ff0000");
		expect(enriched.nodes[1].style).toEqual(DEFAULT_LABEL_STYLE);
		expect(enriched.edges[0].style).toEqual(DEFAULT_LABEL_STYLE);
	});
});
