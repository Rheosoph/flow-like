import { describe, expect, test } from "bun:test";
import type {
	NodeLabelMapping,
	PropertyColumn,
} from "../../../state/backend-state/graph-state";
import {
	type RelationshipEndpoint,
	apiName,
	buildEdge,
	displayName,
	endpointMatchesStem,
	foreignKeyStem,
	isValidGraphIdentifier,
	nodeToEndpoint,
	reversedEdge,
	toEdgeMapping,
	uniqueLabel,
} from "./relationship-form";

function endpoint(
	overrides: Partial<RelationshipEndpoint> & { table: string },
): RelationshipEndpoint {
	return {
		id: overrides.table,
		label: displayName(overrides.table),
		api_name: apiName(overrides.table),
		id_column: "id",
		columns: [],
		color: "#2563eb",
		...overrides,
	};
}

function column(name: string): PropertyColumn {
	return { name, data_type: "Utf8", nullable: true };
}

describe("apiName", () => {
	test("splits camelCase and singularizes only the trailing word", () => {
		expect(apiName("Users")).toBe("user");
		expect(apiName("OrderItems")).toBe("order_item");
		expect(apiName("order_items")).toBe("order_item");
		expect(apiName("Companies")).toBe("company");
		expect(apiName("Addresses")).toBe("address");
		expect(apiName("customerId")).toBe("customer_id");
	});

	test("leaves words that only look plural intact", () => {
		expect(apiName("sales_status")).toBe("sales_status");
		expect(apiName("Analysis")).toBe("analysis");
		expect(apiName("Series")).toBe("series");
		expect(apiName("Class")).toBe("class");
	});

	test("undoes the plurals that used to be mangled", () => {
		expect(apiName("Boxes")).toBe("box");
		expect(apiName("Statuses")).toBe("status");
		expect(apiName("Analyses")).toBe("analysis");
	});
});

describe("displayName", () => {
	test("produces labels the server accepts as query identifiers", () => {
		for (const table of [
			"order_items",
			"invoice_lines",
			"sales_status",
			"Users",
			"OrderItems",
			"a-b.c",
		]) {
			expect(isValidGraphIdentifier(displayName(table))).toBe(true);
		}
	});

	test("keeps a leading digit from producing an invalid identifier", () => {
		expect(displayName("2fa_tokens")).toBe("_2faToken");
		expect(isValidGraphIdentifier(displayName("2fa_tokens"))).toBe(true);
	});

	test("is PascalCase, not spaced", () => {
		expect(displayName("order_items")).toBe("OrderItem");
	});
});

describe("foreignKeyStem", () => {
	test("normalizes the stem the same way object API names are normalized", () => {
		expect(foreignKeyStem("customer_id")).toBe("customer");
		expect(foreignKeyStem("users_id")).toBe("user");
		expect(foreignKeyStem("customerId")).toBe("customer");
		expect(foreignKeyStem("customer_ids")).toBe("customer");
	});

	test("accepts the wider suffix set and the fk_ prefix", () => {
		expect(foreignKeyStem("customer_uuid")).toBe("customer");
		expect(foreignKeyStem("customer_key")).toBe("customer");
		expect(foreignKeyStem("customer_ref")).toBe("customer");
		expect(foreignKeyStem("fk_customer")).toBe("customer");
	});

	test("ignores columns that are not key-shaped", () => {
		expect(foreignKeyStem("total")).toBeUndefined();
		expect(foreignKeyStem("id")).toBeUndefined();
	});
});

describe("endpointMatchesStem", () => {
	const customers = endpoint({ table: "Customers", id_column: "customer_id" });

	test("matches on api name, table, label and id column", () => {
		expect(endpointMatchesStem(customers, "customer")).toBe(true);
		expect(
			endpointMatchesStem(endpoint({ table: "Customers" }), "customer"),
		).toBe(true);
		expect(
			endpointMatchesStem(
				endpoint({ table: "t", api_name: "", label: "", id_column: "user_id" }),
				"user",
			),
		).toBe(true);
	});

	test("irregular plurals are not resolved — person_id needs the manual form", () => {
		expect(endpointMatchesStem(endpoint({ table: "people" }), "person")).toBe(
			false,
		);
	});

	test("does not match an unrelated stem", () => {
		expect(endpointMatchesStem(customers, "order")).toBe(false);
		expect(endpointMatchesStem(customers, "")).toBe(false);
	});

	test("an id column of plain `id` never produces a matchable stem", () => {
		expect(endpointMatchesStem(endpoint({ table: "orders" }), "id")).toBe(
			false,
		);
	});
});

describe("uniqueLabel", () => {
	test("suffixes rather than colliding, and registers what it hands out", () => {
		const taken = new Set(["order_has_customer"]);
		expect(uniqueLabel("order_has_customer", taken)).toBe(
			"order_has_customer_2",
		);
		expect(uniqueLabel("order_has_customer", taken)).toBe(
			"order_has_customer_3",
		);
	});

	test("repairs a base that is not a valid identifier", () => {
		expect(isValidGraphIdentifier(uniqueLabel("2way", new Set()))).toBe(true);
	});
});

describe("buildEdge / toEdgeMapping", () => {
	const orders = endpoint({
		table: "orders",
		columns: [column("id"), column("customer_id")],
	});
	const customers = endpoint({ table: "customers", columns: [column("id")] });

	test("both join columns address the same table", () => {
		const edge = buildEdge({
			originKey: "manual:1",
			manual: true,
			source: orders,
			target: customers,
			table: "orders",
			srcColumn: "id",
			dstColumn: "customer_id",
			label: "order_has_customer",
		});
		expect(edge.table).toBe("orders");
		expect(edge.src_column).toBe("id");
		expect(edge.dst_column).toBe("customer_id");
		expect(edge.src_label).toBe(orders.label);
		expect(edge.dst_label).toBe(customers.label);
	});

	test("strips the surface-local bookkeeping before the payload is sent", () => {
		const mapping = toEdgeMapping(
			buildEdge({
				originKey: "manual:1",
				manual: true,
				source: orders,
				target: customers,
				table: "orders",
				srcColumn: "id",
				dstColumn: "customer_id",
				label: "order_has_customer",
			}),
		);
		expect(Object.keys(mapping)).not.toContain("origin_key");
		expect(Object.keys(mapping)).not.toContain("manual");
		expect(Object.keys(mapping)).not.toContain("src_object_id");
		expect(Object.keys(mapping)).not.toContain("dst_object_id");
		expect(mapping.label).toBe("order_has_customer");
	});
});

describe("reversedEdge", () => {
	test("swaps the join columns with the endpoints", () => {
		const reversed = reversedEdge({
			label: "order_has_customer",
			table: "orders",
			src_column: "id",
			dst_column: "customer_id",
			src_label: "Order",
			dst_label: "Customer",
			property_columns: [],
			style: { color: "#000", icon: "arrow-right", size: { mode: "fixed" } },
		});
		expect(reversed.src_label).toBe("Customer");
		expect(reversed.dst_label).toBe("Order");
		expect(reversed.src_column).toBe("customer_id");
		expect(reversed.dst_column).toBe("id");
		expect(reversed.table).toBe("orders");
	});
});

describe("nodeToEndpoint", () => {
	test("reads the column list off a saved node", () => {
		const node: NodeLabelMapping = {
			id: "node-1",
			api_name: "customer",
			label: "Customer",
			table: "customers",
			id_column: "id",
			property_columns: [column("id"), column("name")],
			style: { color: "#111", icon: "database", size: { mode: "fixed" } },
		};
		const result = nodeToEndpoint(node);
		expect(result.id).toBe("node-1");
		expect(result.columns.map((c) => c.name)).toEqual(["id", "name"]);
		expect(result.color).toBe("#111");
	});
});
