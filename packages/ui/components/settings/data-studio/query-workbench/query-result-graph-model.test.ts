import { describe, expect, test } from "bun:test";
import { buildQueryResultGraph } from "./query-result-graph-model";

const employee = { column: "employee_id", label: "Employee Id" };
const manager = { column: "manager_id", label: "Manager Id" };

describe("buildQueryResultGraph", () => {
	test("merges equal values into one node and keeps each row on its edge", () => {
		const rows = [
			{ employee_id: 2, manager_id: 1, title: "Lead" },
			{ employee_id: 3, manager_id: 1, title: "Dev" },
		];
		const { data } = buildQueryResultGraph(rows, employee, manager);

		expect(data.nodes.map((node) => node.id)).toEqual(["2", "1", "3"]);
		expect(data.edges).toHaveLength(2);
		expect(data.edges[0]).toMatchObject({
			id: "row:0",
			source: "2",
			target: "1",
			label: "Employee Id → Manager Id",
			props: rows[0],
		});
		expect(data.truncated).toBe(false);
	});

	test("labels a node by its source column once it appears as a source", () => {
		const rows = [
			{ employee_id: 2, manager_id: 1 },
			{ employee_id: 1, manager_id: 0 },
		];
		const { data } = buildQueryResultGraph(rows, employee, manager);
		const node = data.nodes.find((entry) => entry.id === "1");

		expect(node?.label).toBe("Employee Id");
		expect(node?.props).toEqual({ manager_id: 1, employee_id: 1 });
		expect(data.nodes.find((entry) => entry.id === "0")?.label).toBe(
			"Manager Id",
		);
	});

	test("skips rows with a null or empty endpoint", () => {
		const rows = [
			{ employee_id: 1, manager_id: null },
			{ employee_id: "", manager_id: 2 },
			{ employee_id: 3, manager_id: 4 },
		];
		const { data } = buildQueryResultGraph(rows, employee, manager);

		expect(data.edges.map((edge) => edge.id)).toEqual(["row:2"]);
		expect(data.nodes.map((node) => node.id)).toEqual(["3", "4"]);
	});

	test("stops before a row would exceed the node limit", () => {
		const rows = [
			{ employee_id: 1, manager_id: 2 },
			{ employee_id: 1, manager_id: 2 },
			{ employee_id: 3, manager_id: 4 },
		];
		const { data } = buildQueryResultGraph(rows, employee, manager, {
			nodes: 3,
			edges: 10,
		});

		expect(data.nodes).toHaveLength(2);
		expect(data.edges).toHaveLength(2);
		expect(data.truncated).toBe(true);
	});

	test("styles every node and edge label through the synthetic overlay", () => {
		const { overlay, data } = buildQueryResultGraph(
			[{ employee_id: 2, manager_id: 1 }],
			employee,
			manager,
		);

		expect(overlay.nodes.map((node) => node.label)).toEqual([
			"Employee Id",
			"Manager Id",
		]);
		expect(overlay.edges.map((edge) => edge.label)).toEqual([
			"Employee Id → Manager Id",
		]);
		expect(data.nodes.every((node) => node.style)).toBe(true);
		expect(data.edges[0].style).toBeDefined();
	});

	test("uses the column label alone when source and target are one column", () => {
		const { data } = buildQueryResultGraph(
			[{ employee_id: 7 }],
			employee,
			employee,
		);

		expect(data.nodes).toHaveLength(1);
		expect(data.edges[0]).toMatchObject({
			source: "7",
			target: "7",
			label: "Employee Id",
		});
	});
});
