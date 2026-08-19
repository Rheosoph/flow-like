import { describe, expect, test } from "bun:test";
import type {
	SubgraphEdge,
	SubgraphNode,
	SubgraphResult,
} from "../../../state/backend-state/graph-state";
import type { ClusterModel, GraphCluster } from "./graph-clusters";
import {
	COLLAPSED_GROUP_PREFIX,
	collapseClusters,
	collapsedGroupClusterId,
	isCollapsedGroupId,
} from "./graph-collapse";

function node(id: string, label = "Item"): SubgraphNode {
	return { id, label, caption: id, props: {} };
}

function edge(source: string, target: string, label = "LINKS"): SubgraphEdge {
	return {
		id: `${label}:${source}->${target}`,
		source,
		target,
		label,
		props: {},
	};
}

/** Two groups of three, joined by two edges between them. */
function fixture(): { data: SubgraphResult; clusters: ClusterModel } {
	const data: SubgraphResult = {
		nodes: ["a1", "a2", "a3", "b1", "b2", "b3"].map((id) => node(id)),
		edges: [
			edge("a1", "a2"),
			edge("a2", "a3"),
			edge("b1", "b2"),
			edge("b2", "b3"),
			edge("a1", "b1"),
			edge("a3", "b3"),
		],
		truncated: false,
	};

	const groups: GraphCluster[] = [
		{
			id: "community:a1",
			kind: "community",
			title: "A",
			memberIds: ["a1", "a2", "a3"],
			hubId: "a1",
			childIds: ["a2", "a3"],
			represented: 3,
			exact: true,
		},
		{
			id: "community:b1",
			kind: "community",
			title: "B",
			memberIds: ["b1", "b2", "b3"],
			hubId: "b1",
			childIds: ["b2", "b3"],
			represented: 3,
			exact: true,
		},
	];

	const byNode = new Map(
		groups.flatMap((group) =>
			group.memberIds.map((id) => [
				id,
				{ clusterId: group.id, isHub: id === group.hubId, represented: 0 },
			]),
		),
	);

	return { data, clusters: { clusters: groups, byNode, epoch: "x" } };
}

describe("collapseClusters", () => {
	test("leaves the graph untouched when nothing is collapsed", () => {
		const { data, clusters } = fixture();
		const result = collapseClusters(data, clusters, new Set());
		expect(result.data).toBe(data);
		expect(result.hiddenNodeCount).toBe(0);
	});

	test("folds a group into one node and keeps its outside connections", () => {
		const { data, clusters } = fixture();
		const result = collapseClusters(data, clusters, new Set(["community:a1"]));

		expect(result.data.nodes).toHaveLength(4);
		expect(result.hiddenNodeCount).toBe(3);

		const group = result.data.nodes.find((n) => isCollapsedGroupId(n.id));
		expect(group?.caption).toContain("3");
		expect(group?.props.objects).toBe(3);

		// a1→b1 and a3→b3 both become group→b*, and neither may be lost.
		const fromGroup = result.data.edges.filter((e) =>
			isCollapsedGroupId(e.source),
		);
		expect(fromGroup.map((e) => e.target).sort()).toEqual(["b1", "b3"]);
	});

	test("drops edges that live entirely inside a collapsed group", () => {
		const { data, clusters } = fixture();
		const result = collapseClusters(data, clusters, new Set(["community:a1"]));
		expect(
			result.data.edges.some((e) => e.source === "a1" || e.target === "a2"),
		).toBe(false);
	});

	test("merges parallel edges between two collapsed groups and counts them", () => {
		const { data, clusters } = fixture();
		const result = collapseClusters(
			data,
			clusters,
			new Set(["community:a1", "community:b1"]),
		);

		expect(result.data.nodes).toHaveLength(2);
		expect(result.data.edges).toHaveLength(1);
		expect(result.data.edges[0].props.collapsed_edges).toBe(2);
		expect(result.hiddenNodeCount).toBe(6);
	});

	test("refuses to collapse a group of one", () => {
		const data: SubgraphResult = {
			nodes: [node("solo")],
			edges: [],
			truncated: false,
		};
		const clusters: ClusterModel = {
			clusters: [
				{
					id: "community:solo",
					kind: "community",
					title: "Solo",
					memberIds: ["solo"],
					hubId: "solo",
					childIds: [],
					represented: 1,
					exact: true,
				},
			],
			byNode: new Map([
				["solo", { clusterId: "community:solo", isHub: true, represented: 0 }],
			]),
			epoch: "x",
		};

		const result = collapseClusters(
			data,
			clusters,
			new Set(["community:solo"]),
		);
		expect(result.data.nodes).toHaveLength(1);
		expect(result.data.nodes[0].id).toBe("solo");
	});

	test("ignores a collapsed id the grouping no longer contains", () => {
		const { data, clusters } = fixture();
		const result = collapseClusters(
			data,
			clusters,
			new Set(["community:gone"]),
		);
		expect(result.data.nodes).toHaveLength(6);
	});

	test("round-trips a group id", () => {
		expect(
			collapsedGroupClusterId(`${COLLAPSED_GROUP_PREFIX}community:a1`),
		).toBe("community:a1");
		expect(isCollapsedGroupId("a1")).toBe(false);
	});
});
