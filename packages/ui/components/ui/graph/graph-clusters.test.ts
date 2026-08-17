import { describe, expect, test } from "bun:test";
import type {
	GraphOverlay,
	SubgraphEdge,
	SubgraphNode,
	SubgraphResult,
} from "../../../state/backend-state/graph-state";
import {
	MAX_CLUSTERS,
	buildClusterModel,
	formatRepresented,
} from "./graph-clusters";
import { DEFAULT_LABEL_STYLE } from "./subgraph-utils";

const SPINE = "HAS_CHUNK";
/** Words that would claim structure a bounded sample cannot support. */
const FORBIDDEN_WORDS = ["cluster", "community", "component"];

function node(
	id: string,
	label: string,
	extra: Partial<SubgraphNode> = {},
): SubgraphNode {
	return { id, label, caption: id, props: {}, ...extra };
}

function edge(source: string, target: string, label = SPINE): SubgraphEdge {
	return {
		id: `${label}:${source}->${target}`,
		source,
		target,
		label,
		props: {},
	};
}

function overlay(spine = SPINE): GraphOverlay {
	return {
		id: "overlay",
		name: "Overlay",
		nodes: [],
		edges: [
			{
				label: spine,
				table: spine,
				src_column: "parent",
				dst_column: "child",
				src_label: "Doc",
				dst_label: "Chunk",
				containment: true,
				property_columns: [],
				style: DEFAULT_LABEL_STYLE,
			},
		],
		object_views: [],
		actions: [],
		exposed: false,
		bindings_enabled: false,
		default_limit: 200,
		created_at: "",
		updated_at: "",
	};
}

/** `parents` maps a document id to how many chunks of it are loaded. */
function corpus(parents: Record<string, number>): SubgraphResult {
	const nodes: SubgraphNode[] = [];
	const edges: SubgraphEdge[] = [];
	for (const [parent, childCount] of Object.entries(parents)) {
		nodes.push(node(parent, "Doc"));
		for (let index = 0; index < childCount; index += 1) {
			const childId = `${parent}-c${index}`;
			nodes.push(node(childId, "Chunk"));
			edges.push(edge(parent, childId));
		}
	}
	return { nodes, edges, truncated: false };
}

function shuffled<T>(items: readonly T[]): T[] {
	const copy = [...items];
	// Deterministic reversal-and-interleave: order changes, contents do not.
	return copy
		.filter((_, index) => index % 2 === 1)
		.concat(copy.filter((_, index) => index % 2 === 0).reverse());
}

function snapshot(model: ReturnType<typeof buildClusterModel>): string {
	return JSON.stringify({
		epoch: model?.epoch,
		clusters: model?.clusters,
		byNode: model ? [...model.byNode] : null,
	});
}

describe("buildClusterModel", () => {
	test("centres each parent on the children only it holds", () => {
		const model = buildClusterModel(
			corpus({ "doc-a": 12, "doc-b": 8, "doc-c": 6 }),
			overlay(),
		);

		expect(model?.clusters.map((cluster) => cluster.id)).toEqual([
			"hub:doc-a",
			"hub:doc-b",
			"hub:doc-c",
		]);
		expect(model?.clusters[0].memberIds).toHaveLength(13);
		expect(model?.byNode.get("doc-a")?.isHub).toBe(true);
		expect(model?.byNode.get("doc-a-c0")?.isHub).toBe(false);
		expect(model?.byNode.get("doc-a-c0")?.clusterId).toBe("hub:doc-a");
	});

	test("gives a contested child to the larger parent", () => {
		const data = corpus({ "doc-small": 6, "doc-large": 20 });
		data.edges.push(edge("doc-small", "doc-large-c0"));
		data.edges.push(edge("doc-small", "doc-large-c1"));

		const model = buildClusterModel(data, overlay());
		const large = model?.clusters.find(
			(cluster) => cluster.id === "hub:doc-large",
		);
		const small = model?.clusters.find(
			(cluster) => cluster.id === "hub:doc-small",
		);

		expect(large?.childIds).toContain("doc-large-c0");
		expect(small?.childIds).not.toContain("doc-large-c0");
		expect(small?.childIds).toHaveLength(6);
	});

	test("ranks by the sampled population, not by what is on screen", () => {
		const data = corpus({ "doc-wide": 6, "doc-narrow": 16 });
		const wide = data.nodes.find((entry) => entry.id === "doc-wide");
		if (wide) {
			wide.stats = {
				out_by_label: [{ label: SPINE, count: 412 }],
				exact: false,
			};
		}

		const model = buildClusterModel(data, overlay());

		expect(model?.clusters[0].id).toBe("hub:doc-wide");
		expect(model?.clusters[0].represented).toBe(412);
		expect(model?.byNode.get("doc-wide")?.badge).toBe("≥412");
		expect(model?.byNode.get("doc-narrow")?.badge).toBe("16");
	});

	test("is byte-identical across repeated calls", () => {
		const data = corpus({ "doc-a": 12, "doc-b": 8, "doc-c": 6 });

		expect(snapshot(buildClusterModel(data, overlay()))).toBe(
			snapshot(buildClusterModel(data, overlay())),
		);
	});

	test("does not depend on the order rows arrived in", () => {
		const data = corpus({ "doc-a": 12, "doc-b": 8, "doc-c": 6 });
		const reordered: SubgraphResult = {
			...data,
			nodes: shuffled(data.nodes),
			edges: shuffled(data.edges),
		};

		expect(snapshot(buildClusterModel(reordered, overlay()))).toBe(
			snapshot(buildClusterModel(data, overlay())),
		);
	});

	test("groups edge-free input by object type", () => {
		const nodes = [
			...Array.from({ length: 12 }, (_, index) => node(`p${index}`, "Person")),
			...Array.from({ length: 10 }, (_, index) => node(`t${index}`, "Team")),
			...Array.from({ length: 4 }, (_, index) => node(`s${index}`, "Site")),
		];

		const model = buildClusterModel(
			{ nodes, edges: [], truncated: false },
			overlay(),
		);

		expect(model?.clusters.map((cluster) => cluster.title)).toEqual([
			"Person",
			"Team",
			"Site",
		]);
		expect(model?.clusters.every((cluster) => cluster.kind === "type")).toBe(
			true,
		);
		expect(model?.clusters[0].subtitle).toBe("no connections in this view");
	});

	test("keeps a connected set clear of the no-connections wording", () => {
		const data = corpus({ "doc-a": 12, "doc-b": 12 });

		const model = buildClusterModel(data, overlay());

		expect(
			model?.clusters.every(
				(cluster) => cluster.subtitle !== "no connections in this view",
			),
		).toBe(true);
	});

	test("falls back to modularity when by-label grouping crosses every edge", () => {
		// An undeclared spine with no sampler counts leaves only by-label groups,
		// and grouping by label would put every edge across a boundary. Rather than
		// stepping aside for a hairball, the model asks the edges where the groups
		// are: each document and its own chunks stay together.
		const plain = overlay();
		plain.edges[0].containment = false;

		const data = corpus({ "doc-a": 12, "doc-b": 12 });
		const model = buildClusterModel(data, plain);

		expect(model).not.toBeNull();
		const crossing = data.edges.filter(
			(e) =>
				model?.byNode.get(e.source)?.clusterId !==
				model?.byNode.get(e.target)?.clusterId,
		);
		expect(crossing).toHaveLength(0);
		expect(model?.byNode.get("doc-a")?.clusterId).not.toBe(
			model?.byNode.get("doc-b")?.clusterId,
		);
	});

	test("a modularity group never claims a fan-out it does not have", () => {
		// Only a declared parent earns a badge: a community's centre is its label
		// anchor, and a count beside it would read as "stands for N others".
		const plain = overlay();
		plain.edges[0].containment = false;

		const model = buildClusterModel(
			corpus({ "doc-a": 12, "doc-b": 12 }),
			plain,
		);
		const anchors = [...(model?.byNode.values() ?? [])].filter((a) => a.isHub);

		expect(anchors.length).toBeGreaterThan(0);
		expect(anchors.every((a) => a.badge === undefined)).toBe(true);
	});

	test("treats a label the sampler counted as a spine, undeclared or not", () => {
		const plain = overlay();
		plain.edges[0].containment = false;

		const data = corpus({ "doc-a": 12, "doc-b": 12 });
		for (const node of data.nodes) {
			if (node.label !== "Doc") continue;
			node.stats = {
				out_by_label: [{ label: SPINE, count: 400 }],
				exact: false,
			};
		}

		const model = buildClusterModel(data, plain);

		expect(model?.clusters.map((cluster) => cluster.title)).toEqual([
			"doc-a",
			"doc-b",
		]);
		expect(
			model?.clusters.every((cluster) => cluster.memberIds.length === 13),
		).toBe(true);
	});

	test("returns null when the plain layout already reads fine", () => {
		expect(
			buildClusterModel(corpus({ "doc-a": 5, "doc-b": 4 }), overlay()),
		).toBeNull();
	});

	test("returns null when everything lands in one group", () => {
		const data = corpus({ "doc-a": 30 });

		expect(buildClusterModel(data, overlay())).toBeNull();
	});

	test("folds the long tail so the group count stays bounded", () => {
		const nodes = Array.from({ length: MAX_CLUSTERS + 120 }, (_, index) =>
			node(`n${index}`, `Label${String(index).padStart(4, "0")}`),
		);

		const model = buildClusterModel(
			{ nodes, edges: [], truncated: false },
			overlay(),
		);

		expect(model?.clusters).toHaveLength(MAX_CLUSTERS);
		expect(model?.clusters.at(-1)?.memberIds.length).toBe(121);
		expect(model?.byNode.size).toBe(nodes.length);
	});

	test("never claims structure the sample cannot support", () => {
		const data = corpus({ "doc-a": 12, "doc-b": 8 });
		data.nodes.push(
			...Array.from({ length: 6 }, (_, index) => node(`free${index}`, "Note")),
		);

		const model = buildClusterModel(data, overlay());
		const copy = (model?.clusters ?? [])
			.flatMap((cluster) => [cluster.title, cluster.subtitle ?? ""])
			.join(" ")
			.toLowerCase();

		for (const word of FORBIDDEN_WORDS) {
			expect(copy).not.toContain(word);
		}
	});
});

describe("formatRepresented", () => {
	test("marks a windowed count as a lower bound", () => {
		expect(formatRepresented(412, false)).toBe("≥412");
	});

	test("states an exact count plainly", () => {
		expect(formatRepresented(412, true)).toBe("412");
	});

	test("never renders a negative population", () => {
		expect(formatRepresented(-3, true)).toBe("0");
	});
});

describe("epoch", () => {
	test("survives an expansion that only adds members to known groups", () => {
		// doc-c overtakes doc-a, so the ranked array reorders while the set of
		// groups on screen does not. Hashing the order would relayout here.
		const before = buildClusterModel(
			corpus({ "doc-a": 12, "doc-b": 8, "doc-c": 6 }),
			overlay(),
		);
		const after = buildClusterModel(
			corpus({ "doc-a": 12, "doc-b": 8, "doc-c": 26 }),
			overlay(),
		);

		expect(before?.clusters[0].title).toBe("doc-a");
		expect(after?.clusters[0].title).toBe("doc-c");
		expect(before?.epoch).toBe(after?.epoch as string);
	});

	test("changes when a different sample brings different groups", () => {
		const before = buildClusterModel(
			corpus({ "doc-a": 12, "doc-b": 8, "doc-c": 6 }),
			overlay(),
		);
		const after = buildClusterModel(
			corpus({ "doc-a": 12, "doc-b": 8, "doc-c": 6, "doc-d": 9 }),
			overlay(),
		);

		expect(before?.epoch).not.toBe(after?.epoch as string);
	});
});
