import { describe, expect, test } from "bun:test";
import type {
	EdgeLabelMapping,
	GraphOverlay,
	NodeLabelMapping,
} from "../../../state/backend-state/graph-state";
import { subgraphFromCypherRows } from "./cypher-subgraph";

const style = {
	color: "#000000",
	icon: "circle",
	size: { mode: "fixed" as const, value: 10 },
};

function nodeMapping(partial: Partial<NodeLabelMapping>): NodeLabelMapping {
	return {
		label: "Node",
		table: "node",
		id_column: "id",
		property_columns: [],
		style,
		...partial,
	};
}

function edgeMapping(partial: Partial<EdgeLabelMapping>): EdgeLabelMapping {
	return {
		label: "EDGE",
		table: "edge",
		src_column: "src",
		dst_column: "dst",
		src_label: "Node",
		dst_label: "Node",
		property_columns: [],
		style,
		...partial,
	};
}

function overlayWith(
	nodes: NodeLabelMapping[],
	edges: EdgeLabelMapping[],
): GraphOverlay {
	return {
		id: "test",
		name: "Test",
		nodes,
		edges,
		object_views: [],
		actions: [],
		exposed: false,
		bindings_enabled: false,
		default_limit: 100,
		created_at: "",
		updated_at: "",
	};
}

const overlay = overlayWith(
	[
		nodeMapping({
			label: "FeedbackSubmission",
			table: "feedback_submission",
			id_column: "id",
			display_column: "title",
			property_columns: [
				{ name: "title", data_type: "string" },
				{ name: "status", data_type: "string" },
			],
		}),
		nodeMapping({
			label: "Reporter",
			table: "feedback_reporter",
			id_column: "sub",
		}),
	],
	[
		edgeMapping({
			label: "SUBMITTED_BY",
			table: "submitted_by",
			src_column: "submission_id",
			dst_column: "reporter_sub",
			src_label: "FeedbackSubmission",
			dst_label: "Reporter",
		}),
	],
);

describe("subgraphFromCypherRows", () => {
	test("resolves node variables through their id and display columns", () => {
		const result = subgraphFromCypherRows(
			[
				{ "n.id": "s1", "n.title": "Broken login", "n.status": "open" },
				{ "n.id": "s2", "n.title": "Slow search", "n.status": "closed" },
			],
			overlay,
		);

		expect(result).not.toBeNull();
		expect(result?.nodes.map((node) => node.id).sort()).toEqual([
			"FeedbackSubmission:s1",
			"FeedbackSubmission:s2",
		]);
		const first = result?.nodes.find(
			(node) => node.id === "FeedbackSubmission:s1",
		);
		expect(first?.caption).toBe("Broken login");
		expect(first?.props.status).toBe("open");
	});

	test("resolves edge variables and creates stub endpoints", () => {
		const result = subgraphFromCypherRows(
			[
				{
					"n.id": "s1",
					"n.title": "Broken login",
					"r.submission_id": "s1",
					"r.reporter_sub": "u1",
				},
			],
			overlay,
		);

		expect(result?.edges).toHaveLength(1);
		const edge = result?.edges[0];
		expect(edge?.label).toBe("SUBMITTED_BY");
		expect(edge?.source).toBe("FeedbackSubmission:s1");
		expect(edge?.target).toBe("Reporter:u1");
		// The stub endpoint exists so the canvas can draw the relationship.
		expect(result?.nodes.some((node) => node.id === "Reporter:u1")).toBeTrue();
		// The full row beats the stub for the submission itself.
		const submission = result?.nodes.find(
			(node) => node.id === "FeedbackSubmission:s1",
		);
		expect(submission?.caption).toBe("Broken login");
	});

	test("returns null for rows that carry no mappable structure", () => {
		expect(subgraphFromCypherRows([{ count: 42 }], overlay)).toBeNull();
		expect(subgraphFromCypherRows([], overlay)).toBeNull();
		expect(
			subgraphFromCypherRows([{ "x.mystery": "value" }], overlay),
		).toBeNull();
	});

	test("deduplicates repeated nodes and edges across rows", () => {
		const result = subgraphFromCypherRows(
			[
				{
					"n.id": "s1",
					"r.submission_id": "s1",
					"r.reporter_sub": "u1",
				},
				{
					"n.id": "s1",
					"r.submission_id": "s1",
					"r.reporter_sub": "u1",
				},
			],
			overlay,
		);
		expect(result?.edges).toHaveLength(1);
		expect(
			result?.nodes.filter((node) => node.id === "FeedbackSubmission:s1"),
		).toHaveLength(1);
	});
});
