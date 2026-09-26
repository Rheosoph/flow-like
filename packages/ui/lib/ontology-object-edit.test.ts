import { describe, expect, test } from "bun:test";
import type {
	EdgeLabelMapping,
	GraphOverlay,
	LabelStyle,
	NodeLabelMapping,
	SubgraphEdge,
	SubgraphNode,
} from "../state/backend-state/graph-state";
import {
	type ColumnLock,
	type ObjectEditField,
	StaleObjectError,
	buildObjectUpdate,
	buildRelationshipUpdate,
	changedProperties,
	draftFromValue,
	effectiveIdentityColumn,
	foreignKeyRowIdentity,
	lockedObjectColumns,
	lockedRelationshipColumns,
	lockedTableColumns,
	objectEditFields,
	objectTypeKey,
	parsePropertyDraft,
	propertyEditability,
	resolveObjectIdentity,
	resolveRelationshipIdentity,
} from "./ontology-object-edit";

const UNSAFE_INTEGER = 2 ** 53;

const style: LabelStyle = {
	color: "#000000",
	icon: "circle",
	size: { mode: "fixed", value: 1 },
};

function objectType(
	label: string,
	table: string,
	idColumn = "id",
	extra: Partial<NodeLabelMapping> = {},
): NodeLabelMapping {
	return {
		label,
		table,
		id_column: idColumn,
		property_columns: [],
		style,
		...extra,
	};
}

function relationship(
	label: string,
	table: string,
	src: [string, string],
	dst: [string, string],
	extra: Partial<EdgeLabelMapping> = {},
): EdgeLabelMapping {
	return {
		label,
		table,
		src_label: src[0],
		src_column: src[1],
		dst_label: dst[0],
		dst_column: dst[1],
		property_columns: [],
		style,
		...extra,
	};
}

function overlayOf(
	nodes: NodeLabelMapping[],
	edges: EdgeLabelMapping[],
): GraphOverlay {
	return {
		id: "ontology",
		name: "Ontology",
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

const person = objectType("Person", "people", "id", {
	id: "person-type",
	api_name: "person",
});
const team = objectType("Team", "teams");
const employee = objectType("Employee", "employees");
const department = objectType("Department", "departments");
const membership = objectType("Membership", "memberships", "membership_id");

const memberOf = relationship(
	"MEMBER_OF",
	"memberships",
	["Person", "person_email"],
	["Team", "team_id"],
	{ src_node_column: "email" },
);
const worksIn = relationship(
	"WORKS_IN",
	"employees",
	["Employee", "id"],
	["Department", "dept_id"],
);
const reportsTo = relationship(
	"MANAGES",
	"employees",
	["Employee", "manager_id"],
	["Employee", "id"],
);

const overlay = overlayOf(
	[person, team, employee, department, membership],
	[memberOf, worksIn, reportsTo],
);

function node(
	id: string,
	label: string,
	props: Record<string, unknown>,
): SubgraphNode {
	return { id, label, props };
}

function edge(
	label: string,
	source: string,
	target: string,
	props: Record<string, unknown> = {},
): SubgraphEdge {
	return { id: `${source}-${label}->${target}`, source, target, label, props };
}

function field(
	name: string,
	kind: ObjectEditField["kind"],
	extra: Partial<ObjectEditField> = {},
): ObjectEditField {
	return {
		name,
		kind,
		integer: false,
		nullable: true,
		temporal: null,
		...extra,
	};
}

describe("effectiveIdentityColumn", () => {
	test("an edge's node-column override wins over the node's id column", () => {
		expect(effectiveIdentityColumn(overlay, "Person")).toBe("email");
		expect(effectiveIdentityColumn(overlay, "Team")).toBe("id");
	});

	test("conflicting overrides identify nothing", () => {
		const conflicting = overlayOf(
			[person, team],
			[
				memberOf,
				relationship("LEADS", "teams", ["Person", "lead"], ["Team", "id"], {
					src_node_column: "login",
				}),
			],
		);
		expect(effectiveIdentityColumn(conflicting, "Person")).toBeNull();
	});

	test("an unmapped label has no identity column", () => {
		expect(effectiveIdentityColumn(overlay, "Contract")).toBeNull();
	});
});

describe("lockedObjectColumns", () => {
	test("locks both identities and the join columns of same-table edges", () => {
		expect([...lockedObjectColumns(overlay, person)]).toEqual([
			["email", "identity"],
			["id", "identity"],
		]);
		expect(Object.fromEntries(lockedObjectColumns(overlay, employee))).toEqual({
			id: "identity",
			dept_id: "relationship",
			manager_id: "relationship",
		});
	});

	test("relationship locks cover the pair and objects mapped on the table", () => {
		expect(
			Object.fromEntries(lockedRelationshipColumns(overlay, memberOf)),
		).toEqual({
			person_email: "relationship",
			team_id: "relationship",
			membership_id: "identity",
		});
	});
});

const order = objectType("Order", "orders", "order_id");
const customer = objectType("Customer", "orders", "customer_id");
const sharedOrders = overlayOf([order, customer], []);

describe("lockedTableColumns", () => {
	test("objects sharing a table lock each other's identity", () => {
		const expected: Record<string, ColumnLock> = {
			order_id: "identity",
			customer_id: "identity",
		};
		expect(
			Object.fromEntries(lockedTableColumns(sharedOrders, "orders")),
		).toEqual(expected);
		expect(
			Object.fromEntries(lockedObjectColumns(sharedOrders, order)),
		).toEqual(expected);
		expect(
			Object.fromEntries(lockedObjectColumns(sharedOrders, customer)),
		).toEqual(expected);
		expect(
			propertyEditability(
				"customer_id",
				"c1",
				field("customer_id", "string"),
				lockedObjectColumns(sharedOrders, order),
			),
		).toEqual({ locked: "identity" });
		expect(
			propertyEditability(
				"total",
				10,
				field("total", "number"),
				lockedObjectColumns(sharedOrders, order),
			),
		).toMatchObject({ editor: "number" });
	});

	test("a join table locks the endpoints of every relationship stored on it", () => {
		const enrolledIn = relationship(
			"ENROLLED_IN",
			"enrollments",
			["Student", "student_id"],
			["Course", "course_id"],
		);
		const taughtBy = relationship(
			"TAUGHT_BY",
			"enrollments",
			["Course", "course_id"],
			["Teacher", "teacher_id"],
		);
		const school = overlayOf(
			[
				objectType("Student", "students"),
				objectType("Course", "courses"),
				objectType("Teacher", "teachers"),
			],
			[enrolledIn, taughtBy],
		);
		const expected: Record<string, ColumnLock> = {
			student_id: "relationship",
			course_id: "relationship",
			teacher_id: "relationship",
		};
		expect(
			Object.fromEntries(lockedTableColumns(school, "enrollments")),
		).toEqual(expected);
		expect(
			Object.fromEntries(lockedRelationshipColumns(school, enrolledIn)),
		).toEqual(expected);
		expect(
			Object.fromEntries(lockedRelationshipColumns(school, taughtBy)),
		).toEqual(expected);
	});

	test("identity wins when a column also stores a relationship", () => {
		const expected: Record<string, ColumnLock> = {
			id: "identity",
			dept_id: "relationship",
			manager_id: "relationship",
		};
		expect(
			Object.fromEntries(lockedTableColumns(overlay, "employees")),
		).toEqual(expected);
		expect(
			Object.fromEntries(lockedRelationshipColumns(overlay, reportsTo)),
		).toEqual(expected);
		expect(
			Object.fromEntries(lockedRelationshipColumns(overlay, worksIn)),
		).toEqual(expected);
	});
});

describe("foreignKeyRowIdentity", () => {
	test("a relationship stored on its object's own table is a foreign key", () => {
		expect(foreignKeyRowIdentity(overlay, "employees", "id", "dept_id")).toBe(
			"id",
		);
	});

	test("a join table is not a foreign key", () => {
		const joinOnly = overlayOf([person, team], [memberOf]);
		expect(
			foreignKeyRowIdentity(joinOnly, "memberships", "person_email", "team_id"),
		).toBeNull();
	});

	test("a junction table mapped as an object keeps its id out of the join", () => {
		expect(
			foreignKeyRowIdentity(overlay, "memberships", "person_email", "team_id"),
		).toBeNull();
	});
});

describe("resolveObjectIdentity", () => {
	test("identifies an object by the effective column", () => {
		const identity = resolveObjectIdentity(overlay, "Person", {
			id: 1,
			email: "ada@example.com",
		});
		expect(identity).toEqual({
			ok: true,
			mapping: person,
			identityColumn: "email",
			id: "ada@example.com",
		});
	});

	test("a missing identity value is not editable", () => {
		expect(resolveObjectIdentity(overlay, "Person", { id: 1 })).toEqual({
			ok: false,
			reason: "identityMissing",
		});
		expect(
			resolveObjectIdentity(overlay, "Team", { id: { nested: true } }),
		).toEqual({ ok: false, reason: "identityMissing" });
	});

	test("an integer past 2^53 is not editable", () => {
		expect(
			resolveObjectIdentity(overlay, "Team", { id: UNSAFE_INTEGER }),
		).toEqual({ ok: false, reason: "identityUnsafe" });
	});

	test("objects of another ontology are not editable here", () => {
		const crossing = overlayOf(
			[person],
			[
				relationship(
					"SIGNED",
					"contracts",
					["Person", "id"],
					["Contract", "id"],
					{
						dst_ontology: "legal",
					},
				),
			],
		);
		expect(resolveObjectIdentity(crossing, "Contract", { id: "c1" })).toEqual({
			ok: false,
			reason: "crossOntology",
		});
		expect(resolveObjectIdentity(crossing, "Invoice", { id: "i1" })).toEqual({
			ok: false,
			reason: "unknownType",
		});
	});

	test("conflicting overrides are not editable", () => {
		const conflicting = overlayOf(
			[person, team],
			[
				memberOf,
				relationship("LEADS", "teams", ["Person", "lead"], ["Team", "id"], {
					src_node_column: "login",
				}),
			],
		);
		expect(
			resolveObjectIdentity(conflicting, "Person", { email: "a", login: "b" }),
		).toEqual({ ok: false, reason: "identityConflict" });
	});

	test("a sibling on the same table with conflicting overrides blocks the edit", () => {
		const invoice = objectType("Invoice", "invoices", "invoice_id");
		const conflicting = overlayOf(
			[order, customer, invoice],
			[
				relationship(
					"BILLED_TO",
					"invoices",
					["Invoice", "invoice_id"],
					["Customer", "customer_ref"],
					{ dst_node_column: "customer_id" },
				),
				relationship(
					"SHIPPED_TO",
					"shipments",
					["Order", "order_id"],
					["Customer", "customer_email"],
					{ dst_node_column: "email" },
				),
			],
		);
		expect(
			resolveObjectIdentity(sharedOrders, "Order", { order_id: "o1" }),
		).toMatchObject({ ok: true, identityColumn: "order_id", id: "o1" });
		expect(
			resolveObjectIdentity(conflicting, "Order", { order_id: "o1" }),
		).toEqual({ ok: false, reason: "identityConflict" });
		expect(
			resolveObjectIdentity(conflicting, "Invoice", { invoice_id: "i1" }),
		).toMatchObject({ ok: true, identityColumn: "invoice_id", id: "i1" });
	});
});

describe("resolveRelationshipIdentity", () => {
	test("a foreign-key relationship points at the object that stores it", () => {
		expect(
			resolveRelationshipIdentity(
				overlay,
				edge("WORKS_IN", "Employee:7", "Department:3"),
			),
		).toEqual({
			ok: false,
			reason: "foreignKey",
			ownerLabel: "Employee",
			ownerSide: "source",
		});
		expect(
			resolveRelationshipIdentity(
				overlay,
				edge("MANAGES", "Employee:1", "Employee:7"),
			),
		).toEqual({
			ok: false,
			reason: "foreignKey",
			ownerLabel: "Employee",
			ownerSide: "target",
		});
	});

	test("a join-table relationship takes typed endpoint values from loaded nodes", () => {
		const result = resolveRelationshipIdentity(
			overlay,
			edge("MEMBER_OF", "Person:ada@example.com", "Team:42"),
			node("Person:ada@example.com", "Person", {
				id: 1,
				email: "ada@example.com",
			}),
			node("Team:42", "Team", { id: 42 }),
		);
		expect(result).toEqual({
			ok: true,
			mapping: memberOf,
			source: "ada@example.com",
			target: 42,
		});
	});

	test("without loaded nodes the raw id keeps colons past the label prefix", () => {
		expect(
			resolveRelationshipIdentity(
				overlay,
				edge("MEMBER_OF", "Person:urn:person:ada", "Team:42"),
			),
		).toEqual({
			ok: true,
			mapping: memberOf,
			source: "urn:person:ada",
			target: "42",
		});
	});

	test("an unsafe endpoint integer is not editable", () => {
		expect(
			resolveRelationshipIdentity(
				overlay,
				edge("MEMBER_OF", "Person:ada", `Team:${UNSAFE_INTEGER}`),
				undefined,
				node(`Team:${UNSAFE_INTEGER}`, "Team", { id: UNSAFE_INTEGER }),
			),
		).toEqual({ ok: false, reason: "identityUnsafe" });
	});

	test("an unmapped relationship is not editable", () => {
		expect(
			resolveRelationshipIdentity(overlay, edge("OWNS", "Person:a", "Team:b")),
		).toEqual({ ok: false, reason: "unknownType" });
	});
});

describe("objectEditFields", () => {
	test("types columns from the arrow schema", () => {
		const fields = objectEditFields({
			fields: [
				{ name: "id", data_type: "Utf8", nullable: false },
				{ name: "score", data_type: "Int64", nullable: true },
				{ name: "ratio", data_type: "Float64", nullable: true },
				{
					name: "created_at",
					data_type: { Timestamp: ["Millisecond", null] },
					nullable: true,
				},
				{ name: "born_on", data_type: "Date32", nullable: true },
			],
		});
		expect(fields.get("id")).toEqual(
			field("id", "string", { nullable: false }),
		);
		expect(fields.get("score")?.integer).toBe(true);
		expect(fields.get("ratio")?.integer).toBe(false);
		expect(fields.get("created_at")?.temporal).toEqual({
			unit: "millisecond",
			wire: "number",
		});
		expect(fields.get("born_on")?.temporal).toEqual({
			unit: "day",
			wire: "number",
		});
	});
});

describe("propertyEditability", () => {
	const none = new Map();

	test("locked columns win over their type", () => {
		expect(
			propertyEditability(
				"email",
				"a",
				field("email", "string"),
				lockedObjectColumns(overlay, person),
			),
		).toEqual({ locked: "identity" });
	});

	test("locks kinds that have no editor", () => {
		for (const kind of ["geometry", "binary", "vector"] as const) {
			expect(propertyEditability("x", null, field("x", kind), none)).toEqual({
				locked: "kind",
				kind,
			});
		}
	});

	test("locks unknown columns and unsafe integers", () => {
		expect(propertyEditability("x", "a", undefined, none)).toEqual({
			locked: "unknownType",
		});
		expect(
			propertyEditability(
				"x",
				UNSAFE_INTEGER,
				field("x", "number", { integer: true }),
				none,
			),
		).toEqual({ locked: "unsafeInteger" });
	});

	test("picks an editor per kind", () => {
		const integer = field("n", "number", { integer: true });
		expect(propertyEditability("n", 3, integer, none)).toEqual({
			editor: "integer",
			field: integer,
		});
		expect(
			propertyEditability("t", "a", field("t", "string"), none),
		).toMatchObject({ editor: "text" });
		expect(
			propertyEditability("b", true, field("b", "boolean"), none),
		).toMatchObject({ editor: "boolean" });
		expect(
			propertyEditability(
				"d",
				1700000000000,
				field("d", "date", {
					temporal: { unit: "millisecond", wire: "number" },
				}),
				none,
			),
		).toMatchObject({ editor: "temporal" });
	});
});

describe("drafts", () => {
	test("text keeps what was typed", () => {
		expect(parsePropertyDraft("text", { text: "123", isNull: false })).toEqual({
			ok: true,
			value: "123",
		});
		expect(draftFromValue("text", "hello")).toEqual({
			text: "hello",
			isNull: false,
		});
	});

	test("integers reject fractions and values past 2^53", () => {
		expect(
			parsePropertyDraft("integer", { text: "42", isNull: false }),
		).toEqual({
			ok: true,
			value: 42,
		});
		expect(
			parsePropertyDraft("integer", { text: "1.5", isNull: false }),
		).toEqual({ ok: false, error: "integer" });
		expect(
			parsePropertyDraft("integer", {
				text: "9007199254740993",
				isNull: false,
			}),
		).toEqual({ ok: false, error: "unsafeInteger" });
	});

	test("numbers must be finite decimals", () => {
		expect(
			parsePropertyDraft("number", { text: "1.5", isNull: false }),
		).toEqual({ ok: true, value: 1.5 });
		expect(parsePropertyDraft("number", { text: "", isNull: false })).toEqual({
			ok: false,
			error: "number",
		});
		expect(
			parsePropertyDraft("number", { text: "0x10", isNull: false }),
		).toEqual({ ok: false, error: "number" });
	});

	test("booleans accept only true or false", () => {
		expect(
			parsePropertyDraft(
				{ editor: "boolean" },
				{ text: "false", isNull: false },
			),
		).toEqual({ ok: true, value: false });
		expect(
			parsePropertyDraft("boolean", { text: "yes", isNull: false }),
		).toEqual({ ok: false, error: "boolean" });
		expect(draftFromValue("boolean", true)).toEqual({
			text: "true",
			isNull: false,
		});
	});

	test("temporal drafts carry the editor's JSON wire value", () => {
		const draft = draftFromValue("temporal", 1700000000000);
		expect(draft).toEqual({ text: "1700000000000", isNull: false });
		expect(parsePropertyDraft("temporal", draft)).toEqual({
			ok: true,
			value: 1700000000000,
		});
		expect(
			parsePropertyDraft("temporal", { text: "not json", isNull: false }),
		).toEqual({ ok: false, error: "temporal" });
	});

	test("an empty draft is null for every editor", () => {
		expect(draftFromValue("integer", null)).toEqual({ text: "", isNull: true });
		expect(parsePropertyDraft("integer", { text: "", isNull: true })).toEqual({
			ok: true,
			value: null,
		});
	});
});

describe("updates", () => {
	test("an unchanged number is not dirty", () => {
		expect(
			changedProperties(
				{ score: 3, name: "Ada", missing: null },
				{ score: 3, name: "Grace", missing: undefined },
			),
		).toEqual({ name: "Grace" });
	});

	test("an object update names the type by id, api name, then label", () => {
		expect(objectTypeKey(person)).toBe("person-type");
		expect(objectTypeKey({ label: "Team", api_name: "team" })).toBe("team");
		expect(objectTypeKey(team)).toBe("Team");

		const identity = resolveObjectIdentity(overlay, "Person", {
			email: "ada@example.com",
		});
		if (!identity.ok) throw new Error("expected an editable identity");
		expect(
			buildObjectUpdate(
				identity,
				{ email: "ada@example.com", name: "Ada", score: 3 },
				{ name: "Grace", nickname: "G" },
			),
		).toEqual({
			object_type: "person-type",
			id: "ada@example.com",
			updates: { name: "Grace", nickname: "G" },
			expected: { name: "Ada", nickname: null },
		});
	});

	test("a relationship update carries the typed pair", () => {
		const identity = resolveRelationshipIdentity(
			overlay,
			edge("MEMBER_OF", "Person:ada", "Team:42"),
			undefined,
			node("Team:42", "Team", { id: 42 }),
		);
		if (!identity.ok) throw new Error("expected an editable relationship");
		expect(
			buildRelationshipUpdate(identity, { role: "member" }, { role: "lead" }),
		).toEqual({
			relationship_type: "MEMBER_OF",
			source: "ada",
			target: 42,
			updates: { role: "lead" },
			expected: { role: "member" },
		});
	});

	test("a stale error carries the current row", () => {
		const error = new StaleObjectError({ name: "Current" });
		expect(error).toBeInstanceOf(Error);
		expect(error.current).toEqual({ name: "Current" });
	});
});
