import { describe, expect, test } from "bun:test";
import type {
	GraphOverlay,
	SubgraphEdge,
	SubgraphNode,
	SubgraphResult,
} from "../../../state/backend-state/graph-state";
import {
	DEFAULT_LABEL_STYLE,
	buildOverlayFromSubgraph,
	enrichSubgraphWithStyles,
	patchSubgraphEdge,
	patchSubgraphNode,
	patchSubgraphRowCopies,
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

	test("derives a label's colour from the label alone, not its position", () => {
		const first = buildOverlayFromSubgraph([node("a", "Person")], []);
		// "Person" now arrives second, and alongside different data.
		const second = buildOverlayFromSubgraph(
			[node("y", "Team"), node("z", "Person")],
			[edge("e1", "MEMBER_OF")],
		);

		const personStyle = second.nodes.find(
			(mapping) => mapping.label === "Person",
		)?.style;
		expect(personStyle?.color).toBe(first.nodes[0].style.color);
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

describe("patchSubgraphNode", () => {
	const overlay = {
		nodes: [{ label: "Person", id_column: "id", display_column: "name" }],
		edges: [],
	} as unknown as GraphOverlay;

	function scene(): SubgraphResult {
		return {
			nodes: [
				{
					id: "Person:1",
					label: "Person",
					caption: "Ada",
					props: { id: 1, name: "Ada", role: "Engineer" },
					property_metadata: { name: { note: "kept" } },
					stats: { out_by_label: [], exact: true },
					style: DEFAULT_LABEL_STYLE,
				},
				{
					id: "Person:2",
					label: "Person",
					caption: "Grace",
					props: { id: 2, name: "Grace" },
				},
			],
			edges: [edge("e1", "KNOWS")],
			truncated: true,
			warnings: ["partial"],
		};
	}

	test("replaces only the patched node and keeps order, ids and untouched identity", () => {
		const before = scene();
		const after = patchSubgraphNode(
			before,
			"Person:1",
			{ id: 1, name: "Ada", role: "Lead", salary: 10, hidden: "x" },
			overlay,
			["role", "salary"],
		);

		expect(after?.nodes.map((candidate) => candidate.id)).toEqual([
			"Person:1",
			"Person:2",
		]);
		expect(after?.nodes[1]).toBe(before.nodes[1]);
		expect(after?.edges).toBe(before.edges);
		expect(after?.truncated).toBe(true);
		expect(after?.warnings).toEqual(["partial"]);
		const patched = after?.nodes[0];
		expect(patched).not.toBe(before.nodes[0]);
		expect(patched?.props).toEqual({
			id: 1,
			name: "Ada",
			role: "Lead",
			salary: 10,
		});
		expect(patched?.caption).toBe("Ada");
		expect(patched?.style).toBe(DEFAULT_LABEL_STYLE);
		expect(patched?.stats).toBe(before.nodes[0].stats);
		expect(patched?.property_metadata).toBe(before.nodes[0].property_metadata);
		expect(before.nodes[0].props.role).toBe("Engineer");
	});

	test("recomputes the caption only when the display column changes", () => {
		const renamed = patchSubgraphNode(
			scene(),
			"Person:1",
			{ name: "Ada Lovelace" },
			overlay,
			["name"],
		);
		expect(renamed?.nodes[0].caption).toBe("Ada Lovelace");

		const other = patchSubgraphNode(
			{
				...scene(),
				nodes: [{ ...scene().nodes[0], caption: "from cypher" }],
			},
			"Person:1",
			{ name: "Ada", role: "Lead" },
			overlay,
			["role"],
		);
		expect(other?.nodes[0].caption).toBe("from cypher");

		const cleared = patchSubgraphNode(
			scene(),
			"Person:1",
			{ name: null },
			overlay,
			["name"],
		);
		expect(cleared?.nodes[0].caption).toBe("1");
		expect(cleared?.nodes[0].props.name).toBeNull();
	});

	test("an unknown node id or missing data is a no-op", () => {
		const before = scene();
		expect(
			patchSubgraphNode(before, "Person:9", { name: "x" }, overlay, ["name"]),
		).toBe(before);
		expect(
			patchSubgraphNode(null, "Person:1", { name: "x" }, overlay, ["name"]),
		).toBeNull();
	});
});

describe("patchSubgraphEdge", () => {
	test("merges the fresh properties into the matching edge only", () => {
		const first: SubgraphEdge = {
			...edge("e1", "KNOWS"),
			props: { since: 2020, note: "old" },
		};
		const second = edge("e2", "KNOWS");
		const before: SubgraphResult = {
			nodes: [node("a", "Person")],
			edges: [first, second],
			truncated: false,
		};
		const after = patchSubgraphEdge(before, "e1", { note: "new" });

		expect(after?.edges.map((candidate) => candidate.id)).toEqual(["e1", "e2"]);
		expect(after?.edges[0].props).toEqual({ since: 2020, note: "new" });
		expect(after?.edges[1]).toBe(second);
		expect(after?.nodes).toBe(before.nodes);
		expect(first.props.note).toBe("old");
		expect(patchSubgraphEdge(before, "missing", { note: "x" })).toBe(before);
	});
});
