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

describe("patchSubgraphRowCopies", () => {
	test("an object edit reaches its foreign-key edges and nothing else", () => {
		const overlay = {
			nodes: [
				{ label: "Employee", table: "employees", id_column: "id" },
				{ label: "Department", table: "departments", id_column: "id" },
			],
			edges: [
				{
					label: "WORKS_IN",
					table: "employees",
					src_column: "id",
					dst_column: "department_id",
					src_label: "Employee",
					dst_label: "Department",
					property_columns: [],
				},
			],
		} as unknown as GraphOverlay;
		const ada = {
			id: 1,
			name: "Ada",
			salary: 50000,
			department_id: 7,
		};
		const before: SubgraphResult = {
			nodes: [
				{ id: "Employee:1", label: "Employee", props: ada },
				{
					id: "Employee:2",
					label: "Employee",
					props: { id: 2, name: "Grace", salary: 40000, department_id: 7 },
				},
				{ id: "Department:7", label: "Department", props: { id: 7 } },
			],
			edges: [
				{
					id: "w1",
					source: "Employee:1",
					target: "Department:7",
					label: "WORKS_IN",
					props: { name: "Ada", salary: 50000 },
				},
				{
					id: "w2",
					source: "Employee:2",
					target: "Department:7",
					label: "WORKS_IN",
					props: { name: "Grace", salary: 40000 },
				},
			],
			truncated: false,
		};
		const after = patchSubgraphRowCopies(
			before,
			overlay,
			{ table: "employees", known: ada, fresh: { ...ada, salary: 60000 } },
			{ nodeId: "Employee:1" },
		);

		expect(after?.edges[0].props).toEqual({ name: "Ada", salary: 60000 });
		expect(after?.edges[1]).toBe(before.edges[1]);
		expect(after?.nodes).toEqual(before.nodes);
		expect(before.edges[0].props.salary).toBe(50000);
	});

	test("another label on the same table follows the saved row and its caption", () => {
		const overlay = {
			nodes: ["Person", "Author"].map((label) => ({
				label,
				table: "people",
				id_column: "id",
				display_column: "name",
			})),
			edges: [],
		} as unknown as GraphOverlay;
		const person = (id: string, name: string, props = {}): SubgraphNode => ({
			id,
			label: id.split(":")[0],
			caption: name,
			props: { id: Number(id.split(":")[1]), name, ...props },
		});
		const before: SubgraphResult = {
			nodes: [
				person("Person:1", "Ada"),
				person("Author:1", "Ada", { books: 3 }),
				person("Author:2", "Grace"),
			],
			edges: [],
			truncated: false,
		};
		const after = patchSubgraphRowCopies(
			before,
			overlay,
			{
				table: "people",
				known: { id: 1, name: "Ada" },
				fresh: { id: 1, name: "Ada L.", role: "x" },
			},
			{ nodeId: "Person:1" },
		);

		expect(after?.nodes[0]).toBe(before.nodes[0]);
		expect(after?.nodes[1].props).toEqual({ id: 1, name: "Ada L.", books: 3 });
		expect(after?.nodes[1].caption).toBe("Ada L.");
		expect(after?.nodes[2]).toBe(before.nodes[2]);
	});

	describe("a join table also mapped as an object", () => {
		const overlay = {
			nodes: [
				{ label: "Person", table: "people", id_column: "id" },
				{ label: "Team", table: "teams", id_column: "id" },
				{ label: "Membership", table: "memberships", id_column: "mid" },
			],
			edges: [
				{
					label: "MEMBER_OF",
					table: "memberships",
					src_column: "person_id",
					dst_column: "team_id",
					src_label: "Person",
					dst_label: "Team",
					property_columns: ["role"],
				},
			],
		} as unknown as GraphOverlay;
		const lead = { mid: 5, person_id: 1, team_id: 2, role: "lead" };
		function scene(): SubgraphResult {
			return {
				nodes: [
					{ id: "Person:1", label: "Person", props: { id: 1 } },
					{ id: "Team:2", label: "Team", props: { id: 2 } },
					{ id: "Membership:5", label: "Membership", props: lead },
					{
						id: "Membership:6",
						label: "Membership",
						props: { mid: 6, person_id: 1, team_id: 3, role: "lead" },
					},
				],
				edges: [
					{
						id: "m1",
						source: "Person:1",
						target: "Team:2",
						label: "MEMBER_OF",
						props: { role: "lead" },
					},
					{
						id: "m2",
						source: "Person:1",
						target: "Team:3",
						label: "MEMBER_OF",
						props: { role: "lead" },
					},
				],
				truncated: false,
			};
		}

		test("an object edit reaches the relationship on its endpoint pair", () => {
			const before = scene();
			const after = patchSubgraphRowCopies(
				before,
				overlay,
				{
					table: "memberships",
					known: lead,
					fresh: { ...lead, role: "member" },
				},
				{ nodeId: "Membership:5" },
			);

			expect(after?.edges[0].props).toEqual({ role: "member" });
			expect(after?.edges[1]).toBe(before.edges[1]);
			expect(after?.nodes[3]).toBe(before.nodes[3]);
		});

		test("a relationship edit reaches the object stored on the same row", () => {
			const before = scene();
			const after = patchSubgraphRowCopies(
				before,
				overlay,
				{
					table: "memberships",
					known: { role: "lead", person_id: "1", team_id: "2" },
					fresh: { role: "member" },
				},
				{ edgeId: "m1" },
			);

			expect(after?.nodes[2].props).toEqual({ ...lead, role: "member" });
			expect(after?.nodes[3]).toBe(before.nodes[3]);
			expect(after?.edges).toEqual(before.edges);
		});
	});

	test("nothing that reads the row, or no overlay, keeps the same data", () => {
		const before: SubgraphResult = {
			nodes: [{ id: "Person:1", label: "Person", props: { id: 1 } }],
			edges: [],
			truncated: false,
		};
		const overlay = {
			nodes: [{ label: "Person", table: "people", id_column: "id" }],
			edges: [],
		} as unknown as GraphOverlay;
		const saved = { table: "people", known: { id: 1 }, fresh: { id: 1 } };
		expect(
			patchSubgraphRowCopies(before, overlay, saved, { nodeId: "Person:1" }),
		).toBe(before);
		expect(patchSubgraphRowCopies(before, null, saved, {})).toBe(before);
		expect(patchSubgraphRowCopies(null, overlay, saved, {})).toBeNull();
	});
});
