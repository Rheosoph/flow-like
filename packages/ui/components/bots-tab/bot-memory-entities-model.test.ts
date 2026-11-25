import { describe, expect, test } from "bun:test";
import type {
	BotEntity,
	BotFact,
} from "../../state/global-chat/bot-memory-facts";
import {
	aliasLabel,
	edgeWidth,
	egoGraph,
	entityAliases,
	entityFacts,
	entityKey,
	groupEntities,
	layoutEgo,
	mergeTargets,
} from "./bot-memory-entities-model";

function entity(id: string, patch: Partial<BotEntity> = {}): BotEntity {
	return {
		id,
		name: id,
		kind: "person",
		owner: false,
		aliases: [],
		fact_count: 0,
		created_at: "2026-11-01T00:00:00Z",
		updated_at: "2026-11-01T00:00:00Z",
		...patch,
	};
}

function fact(id: string, patch: Partial<BotFact> = {}): BotFact {
	return {
		id,
		text: id,
		kind: "profile",
		subject: "user",
		source: "user_stated",
		confidence: 0.9,
		recorded_at: "2026-11-01T00:00:00Z",
		valid_from: null,
		valid_to: null,
		superseded_by: null,
		evidence: [],
		...patch,
	};
}

const entities = [
	entity("e_user", { name: "user", owner: true, aliases: ["user"] }),
	entity("e_mia", { name: "Mia", aliases: ["user.sister", "Mia"] }),
	entity("e_hh", { name: "Hamburg", kind: "place" }),
	entity("e_acme", { name: "Acme", kind: "organization" }),
	entity("e_room_mia", { name: "Mia", conversation_scope: "room1" }),
];

describe("groupEntities", () => {
	test("puts the owner first, then kinds in a fixed order", () => {
		const groups = groupEntities(entities, "", "You");
		expect(groups.map((group) => group.key)).toEqual([
			"owner",
			"person",
			"place",
			"organization",
		]);
	});

	test("searches names, aliases and the owner label", () => {
		expect(
			groupEntities(entities, "sister", "You").flatMap((g) => g.entities),
		).toHaveLength(1);
		expect(
			groupEntities(entities, "you", "You").flatMap((g) => g.entities)[0]?.id,
		).toBe("e_user");
		expect(groupEntities(entities, "zzz", "You")).toEqual([]);
	});
});

describe("entity helpers", () => {
	test("keys differ across scopes", () => {
		expect(entityKey(entities[1])).not.toBe(entityKey(entities[4]));
	});

	test("aliases drop the name itself and read relational keys from the owner", () => {
		expect(entityAliases(entities[1], "You")).toEqual(["You › sister"]);
		expect(aliasLabel("user.sister.husband", "You")).toBe(
			"You › sister › husband",
		);
		expect(aliasLabel("acme gmbh", "You")).toBe("acme gmbh");
		expect(
			groupEntities(entities, "you › sister", "You").flatMap(
				(g) => g.entities,
			)[0]?.id,
		).toBe(entities[1].id);
	});

	test("merge targets stay within the scope", () => {
		expect(mergeTargets(entities, entities[1]).map((e) => e.id)).toEqual([
			"e_user",
			"e_hh",
			"e_acme",
		]);
		expect(mergeTargets(entities, entities[4])).toEqual([]);
	});
});

describe("entityFacts and egoGraph", () => {
	const facts = [
		fact("f1", {
			subject_id: "e_mia",
			object_id: "e_hh",
			relation: "lives in",
		}),
		fact("f2", {
			subject_id: "e_mia",
			object_id: "e_hh",
			relation: "lives in",
		}),
		fact("f3", {
			subject_id: "e_user",
			object_id: "e_mia",
			relation: "sister",
		}),
		fact("f4", { subject_id: "e_mia" }),
		fact("f5", {
			subject_id: "e_room_mia",
			object_id: "e_acme",
			conversation_scope: "room1",
		}),
		fact("f6", { subject_id: "e_room_mia", conversation_scope: "room1" }),
	];

	test("filters facts by id and scope", () => {
		expect(entityFacts(facts, entities[1]).map((f) => f.id)).toEqual([
			"f1",
			"f2",
			"f3",
			"f4",
		]);
		expect(entityFacts(facts, entities[4]).map((f) => f.id)).toEqual([
			"f5",
			"f6",
		]);
	});

	test("counts facts per neighbour and keeps the busiest first", () => {
		const graph = egoGraph(entities[1], entities, facts);
		expect(graph.neighbours.map((n) => n.entity.id)).toEqual([
			"e_hh",
			"e_user",
		]);
		expect(graph.neighbours[0].edge).toEqual({
			count: 2,
			relation: "lives in",
			outgoing: true,
		});
		expect(graph.neighbours[1].edge.outgoing).toBe(false);
		expect(graph.hidden).toBe(0);
	});

	test("caps neighbours and reports the rest", () => {
		const many = Array.from({ length: 5 }, (_, i) => entity(`p${i}`));
		const links = many.map((other, i) =>
			fact(`l${i}`, { subject_id: "e_mia", object_id: other.id }),
		);
		const graph = egoGraph(entities[1], [...entities, ...many], links, 3);
		expect(graph.neighbours).toHaveLength(3);
		expect(graph.hidden).toBe(2);
	});
});

describe("layoutEgo", () => {
	test("starts at the top and spreads evenly", () => {
		const layout = layoutEgo(3);
		expect(layout.nodes[0]).toEqual({ x: layout.center.x, y: 46 });
		expect(layout.nodes[1].y).toBeGreaterThan(layout.center.y);
		expect(layout.nodes).toHaveLength(3);
	});

	test("keeps the space below the centre free for an even count", () => {
		for (const count of [2, 4, 6, 8]) {
			const layout = layoutEgo(count);
			expect(
				layout.nodes.some(
					(node) => node.x === layout.center.x && node.y > layout.center.y,
				),
			).toBe(false);
		}
	});

	test("keeps every node inside the frame", () => {
		for (const node of layoutEgo(8).nodes) {
			expect(node.x).toBeGreaterThan(0);
			expect(node.x).toBeLessThan(360);
			expect(node.y).toBeGreaterThan(0);
			expect(node.y).toBeLessThan(250);
		}
	});

	test("edge width grows with the count and saturates", () => {
		expect(edgeWidth(1)).toBeLessThan(edgeWidth(3));
		expect(edgeWidth(10)).toBe(edgeWidth(6));
	});
});
