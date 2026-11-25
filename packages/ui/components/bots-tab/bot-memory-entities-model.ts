import type {
	BotEntity,
	BotFact,
} from "../../state/global-chat/bot-memory-facts";

export const ENTITY_KINDS = [
	"person",
	"place",
	"organization",
	"project",
	"thing",
] as const;

export const MAX_GRAPH_NEIGHBOURS = 8;

export interface EntityGroup {
	/** `owner` for the person the bot works for, otherwise the entity kind. */
	key: string;
	entities: BotEntity[];
}

export function entityKey(
	entity: Pick<BotEntity, "id" | "conversation_scope">,
) {
	return `${entity.conversation_scope ?? ""}\u0000${entity.id}`;
}

function scopeOf(value: { conversation_scope?: string | null }) {
	return value.conversation_scope ?? "";
}

export function entityLabel(entity: BotEntity, ownerLabel: string) {
	return entity.owner ? ownerLabel : entity.name;
}

/** A relational alias key as a path from the owner: `user.sister` → "You › sister". */
export function aliasLabel(alias: string, ownerLabel: string) {
	if (!alias.startsWith("user.")) return alias;
	return [ownerLabel, ...alias.slice("user.".length).split(".")].join(" › ");
}

/** The other names worth showing, readable: no internal participant keys, no repeat of the name. */
export function entityAliases(entity: BotEntity, ownerLabel: string) {
	const name = entity.name.toLocaleLowerCase();
	return entity.aliases
		.filter(
			(alias) =>
				alias.trim() &&
				!alias.startsWith("speaker:") &&
				alias.toLocaleLowerCase() !== name,
		)
		.map((alias) => aliasLabel(alias, ownerLabel));
}

function matches(entity: BotEntity, ownerLabel: string, query: string) {
	if (!query) return true;
	return [
		entityLabel(entity, ownerLabel),
		entity.name,
		...entity.aliases,
		...entityAliases(entity, ownerLabel),
	]
		.join("\n")
		.toLocaleLowerCase()
		.includes(query);
}

/** The owner first on its own, then the kinds in a fixed order; the backend's order holds inside a group. */
export function groupEntities(
	entities: readonly BotEntity[],
	query: string,
	ownerLabel: string,
): EntityGroup[] {
	const needle = query.trim().toLocaleLowerCase();
	const groups = new Map<string, BotEntity[]>();
	for (const entity of entities) {
		if (!matches(entity, ownerLabel, needle)) continue;
		const key = entity.owner ? "owner" : entity.kind;
		groups.set(key, [...(groups.get(key) ?? []), entity]);
	}
	const rank = (key: string) => {
		if (key === "owner") return -1;
		const index = ENTITY_KINDS.indexOf(key as (typeof ENTITY_KINDS)[number]);
		return index === -1 ? ENTITY_KINDS.length : index;
	};
	return [...groups]
		.sort(([a], [b]) => rank(a) - rank(b) || a.localeCompare(b))
		.map(([key, members]) => ({ key, entities: members }));
}

export function touchesEntity(fact: BotFact, entity: BotEntity) {
	return (
		scopeOf(fact) === scopeOf(entity) &&
		(fact.subject_id === entity.id || fact.object_id === entity.id)
	);
}

export function entityFacts(facts: readonly BotFact[], entity: BotEntity) {
	return facts.filter((fact) => touchesEntity(fact, entity));
}

/** Entities of the same scope a selected one can be merged into. */
export function mergeTargets(
	entities: readonly BotEntity[],
	entity: BotEntity,
) {
	return entities.filter(
		(other) => other.id !== entity.id && scopeOf(other) === scopeOf(entity),
	);
}

export interface GraphEdge {
	count: number;
	/** The most used relation between the two, when the facts name one. */
	relation: string | null;
	/** Whether the centre is the subject of that relation. */
	outgoing: boolean;
}

export interface EgoNeighbour {
	entity: BotEntity;
	edge: GraphEdge;
}

export interface EgoGraph {
	neighbours: EgoNeighbour[];
	/** Connected entities beyond the drawn ones. */
	hidden: number;
}

export function egoGraph(
	center: BotEntity,
	entities: readonly BotEntity[],
	facts: readonly BotFact[],
	limit = MAX_GRAPH_NEIGHBOURS,
): EgoGraph {
	const byId = new Map(
		entities
			.filter((entity) => scopeOf(entity) === scopeOf(center))
			.map((entity) => [entity.id, entity]),
	);
	const edges = new Map<
		string,
		{ count: number; relations: Map<string, number>; outgoing: boolean }
	>();
	for (const fact of entityFacts(facts, center)) {
		if (!fact.subject_id || !fact.object_id) continue;
		if (fact.subject_id === fact.object_id) continue;
		const outgoing = fact.subject_id === center.id;
		const other = outgoing ? fact.object_id : fact.subject_id;
		if (!byId.has(other)) continue;
		const edge = edges.get(other) ?? {
			count: 0,
			relations: new Map(),
			outgoing,
		};
		edge.count += 1;
		const relation = fact.relation?.trim();
		if (relation) {
			edge.relations.set(relation, (edge.relations.get(relation) ?? 0) + 1);
		}
		edges.set(other, edge);
	}
	const ranked = [...edges]
		.map(([id, edge]): EgoNeighbour => {
			const [relation] = [...edge.relations].sort(
				([, a], [, b]) => b - a,
			)[0] ?? [null];
			return {
				entity: byId.get(id) as BotEntity,
				edge: { count: edge.count, relation, outgoing: edge.outgoing },
			};
		})
		.sort(
			(a, b) =>
				b.edge.count - a.edge.count ||
				a.entity.name.localeCompare(b.entity.name),
		);
	return {
		neighbours: ranked.slice(0, limit),
		hidden: Math.max(0, ranked.length - limit),
	};
}

export interface GraphLayoutNode {
	x: number;
	y: number;
}

export interface GraphLayout {
	width: number;
	height: number;
	center: GraphLayoutNode;
	nodes: GraphLayoutNode[];
}

/**
 * Neighbours on an ellipse, clockwise from the top. With an even count the ring is turned by half a
 * step so none sits straight below the centre, where its name is drawn.
 */
export function layoutEgo(
	count: number,
	width = 360,
	height = 250,
	margin = { x: 62, y: 46 },
): GraphLayout {
	const center = { x: width / 2, y: height / 2 };
	const rx = width / 2 - margin.x;
	const ry = height / 2 - margin.y;
	const turn = count % 2 === 0 ? Math.PI / Math.max(count, 1) : 0;
	const nodes = Array.from({ length: count }, (_, index) => {
		const angle =
			-Math.PI / 2 + turn + (2 * Math.PI * index) / Math.max(count, 1);
		return {
			x: round(center.x + rx * Math.cos(angle)),
			y: round(center.y + ry * Math.sin(angle)),
		};
	});
	return { width, height, center, nodes };
}

function round(value: number) {
	return Math.round(value * 100) / 100;
}

export function edgeWidth(count: number) {
	return 1 + Math.min(Math.max(count, 1), 6) * 0.8;
}
