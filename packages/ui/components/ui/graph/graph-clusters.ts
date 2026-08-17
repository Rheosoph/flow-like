import type {
	GraphOverlay,
	SubgraphNode,
	SubgraphResult,
} from "../../../state/backend-state/graph-state";
import { detectCommunities } from "./graph-communities";
import { hashLayoutSeed } from "./graph-layout";

/** Spine children a node needs before it reads as a parent rather than a peer. */
export const HUB_MIN_MEMBERS = 3;
/** Below this the plain force layout is already readable, so nothing is grouped. */
export const MIN_GROUPED_NODES = 24;
/** Ceiling on groups laid out separately; the smallest by-type ones fold into a tail. */
export const MAX_CLUSTERS = 400;
/** Share of edges that must stay inside a grouping for it to beat a force layout. */
const MIN_WITHIN_GROUP_SHARE = 0.5;
/** Below this a community is noise, and folds back in with its own object type. */
const MIN_COMMUNITY_MEMBERS = 3;
/** How much of a community must share one label before that label names it. */
const DOMINANT_LABEL_SHARE = 0.6;
/** Subtitle for a group whose members have no edges in the loaded sample. */
const NO_CONNECTIONS_SUBTITLE = "no connections in this view";
const FOLDED_CLUSTER_ID = "type:__more__";

export type ClusterKind = "hub" | "type" | "community";

export interface GraphCluster {
	id: string;
	kind: ClusterKind;
	title: string;
	subtitle?: string;
	/** Every node this group places, hub first when `kind` is `"hub"`. */
	memberIds: string[];
	hubId?: string;
	childIds?: string[];
	/** Objects the group stands for — the whole population for a sampled hub. */
	represented: number;
	/** False when `represented` is a lower bound read off a sampling window. */
	exact: boolean;
}

export interface ClusterAssignment {
	clusterId: string;
	isHub: boolean;
	/** Rendered fan-out, set on hubs only. */
	badge?: string;
	represented: number;
}

export interface ClusterModel {
	clusters: GraphCluster[];
	byNode: Map<string, ClusterAssignment>;
	/** Changes whenever the grouping changes, so a rebuild can force a relayout. */
	epoch: string;
}

/**
 * A count taken over a sampling window is a lower bound, so it ships with the
 * bound made visible rather than being rounded into a claim the data cannot back.
 */
export function formatRepresented(count: number, exact: boolean): string {
	const value = Math.max(0, Math.round(count));
	return `${exact ? "" : "≥"}${value.toLocaleString()}`;
}

function captionOf(node: SubgraphNode): string {
	return node.caption ?? node.id;
}

function compareIds(a: string, b: string): number {
	return a < b ? -1 : a > b ? 1 : 0;
}

function sumOutgoing(node: SubgraphNode): number | null {
	if (!node.stats) return null;
	let total = 0;
	for (const entry of node.stats.out_by_label) total += entry.count;
	return total;
}

/**
 * Identifies the grouping by which groups exist, deliberately not by who is in
 * them. Expanding a node adds members to a group that is already on screen, and
 * that must keep its positions; a different sample brings different groups, and
 * that has to be laid out afresh.
 */
function clusterEpoch(clusters: readonly GraphCluster[]): string {
	let hash = clusters.length >>> 0;
	// Sorted, so that re-ranking alone is not a regrouping: expanding a small
	// parent past a larger one reorders the array without changing which groups
	// are on screen, and that must not throw the layout away.
	for (const id of clusters.map((cluster) => cluster.id).sort(compareIds)) {
		hash = Math.imul(hash ^ hashLayoutSeed(id), 16777619);
	}
	return `${clusters.length}-${(hash >>> 0).toString(36)}`;
}

/**
 * Groups a loaded subgraph into what the layout draws as separate constellations.
 *
 * Reads `data` alone and never the built graphology attributes: above the canvas'
 * large-graph threshold props and icons are dropped from the attribute set, so the
 * raw result is the only place the whole picture is still intact.
 */
export function buildClusterModel(
	data: SubgraphResult,
	overlay: GraphOverlay,
): ClusterModel | null {
	if (data.nodes.length < MIN_GROUPED_NODES) return null;

	const nodeIds = new Set(data.nodes.map((node) => node.id));
	const captionById = new Map(
		data.nodes.map((node) => [node.id, captionOf(node)]),
	);
	const spineLabels = new Set(
		overlay.edges.filter((edge) => edge.containment).map((edge) => edge.label),
	);
	// `containment` is an opt-in checkbox most overlays never tick, but the
	// sampler only counts fan-out for the relationship it grouped by — so a label
	// it reports is a parent-child spine here even when the overlay never said so.
	for (const node of data.nodes) {
		for (const entry of node.stats?.out_by_label ?? [])
			spineLabels.add(entry.label);
	}

	const degree = new Map<string, number>();
	for (const node of data.nodes) degree.set(node.id, 0);

	const childrenByParent = new Map<string, Set<string>>();
	for (const edge of data.edges) {
		if (!nodeIds.has(edge.source) || !nodeIds.has(edge.target)) continue;
		degree.set(edge.source, (degree.get(edge.source) ?? 0) + 1);
		degree.set(edge.target, (degree.get(edge.target) ?? 0) + 1);
		if (!spineLabels.has(edge.label) || edge.source === edge.target) continue;
		const children = childrenByParent.get(edge.source);
		if (children) children.add(edge.target);
		else childrenByParent.set(edge.source, new Set([edge.target]));
	}

	/** (degree desc, caption asc, id asc) — independent of the order rows arrived in. */
	const byProminence = (a: string, b: string): number =>
		(degree.get(b) ?? 0) - (degree.get(a) ?? 0) ||
		(captionById.get(a) ?? a).localeCompare(captionById.get(b) ?? b) ||
		compareIds(a, b);

	const hubs = data.nodes
		.filter(
			(node) =>
				node.stats !== undefined ||
				(childrenByParent.get(node.id)?.size ?? 0) >= HUB_MIN_MEMBERS,
		)
		.map((node) => {
			const inView = childrenByParent.get(node.id)?.size ?? 0;
			return {
				node,
				represented: sumOutgoing(node) ?? inView,
				exact: node.stats?.exact ?? true,
			};
		})
		.sort(
			(a, b) =>
				b.represented - a.represented || compareIds(a.node.id, b.node.id),
		);

	const claimed = new Set<string>();
	const hubClusters: GraphCluster[] = [];
	for (const hub of hubs) {
		if (claimed.has(hub.node.id)) continue;
		claimed.add(hub.node.id);

		const childIds = Array.from(childrenByParent.get(hub.node.id) ?? [])
			.filter((childId) => !claimed.has(childId))
			.sort(byProminence);
		for (const childId of childIds) claimed.add(childId);

		hubClusters.push({
			id: `hub:${hub.node.id}`,
			kind: "hub",
			title: captionOf(hub.node),
			subtitle: hub.node.label,
			memberIds: [hub.node.id, ...childIds],
			hubId: hub.node.id,
			childIds,
			represented: hub.represented,
			exact: hub.exact,
		});
	}

	const idsByLabel = new Map<string, string[]>();
	for (const node of data.nodes) {
		if (claimed.has(node.id)) continue;
		const bucket = idsByLabel.get(node.label);
		if (bucket) bucket.push(node.id);
		else idsByLabel.set(node.label, [node.id]);
	}

	const typeClusters = Array.from(idsByLabel.entries())
		.map(([label, ids]) => {
			const memberIds = [...ids].sort(byProminence);
			const connected = memberIds.some((id) => (degree.get(id) ?? 0) > 0);
			return {
				id: `type:${label}`,
				kind: "type" as const,
				title: label,
				subtitle: connected ? undefined : NO_CONNECTIONS_SUBTITLE,
				memberIds,
				represented: memberIds.length,
				exact: true,
			};
		})
		.sort(
			(a, b) =>
				b.memberIds.length - a.memberIds.length || compareIds(a.id, b.id),
		);

	const byType = foldSmallest([...hubClusters, ...typeClusters]);
	const typeModel = byType.length > 1 ? toModel(byType) : null;

	// Grouping is only worth having when the edges agree with it. On a peer
	// ontology — no parent-child spine, everything linked across labels — these
	// groups are blobs joined by stage-crossing lines, strictly worse than the
	// force layout at showing where the structure is.
	if (
		typeModel &&
		withinGroupShare(data, typeModel) >= MIN_WITHIN_GROUP_SHARE
	) {
		return typeModel;
	}

	// So ask the edges instead of the schema. Modularity finds the groups a peer
	// ontology actually has, which is the difference between a hairball and a
	// readable picture on exactly the ontologies the rule above rejects.
	const communities = buildCommunityClusters(data, byProminence, degree);
	if (communities.length > 1) {
		const communityModel = toModel(communities);
		const share = withinGroupShare(data, communityModel);
		if (share >= MIN_WITHIN_GROUP_SHARE) return communityModel;
		// Neither grouping beat the threshold, so take whichever kept more edges
		// at home rather than dropping to an undifferentiated force layout.
		if (!typeModel || share > withinGroupShare(data, typeModel)) {
			return communityModel;
		}
	}

	return typeModel && withinGroupShare(data, typeModel) > 0 ? typeModel : null;
}

function toModel(clusters: GraphCluster[]): ClusterModel {
	const byNode = new Map<string, ClusterAssignment>();
	for (const cluster of clusters) {
		// Only a real parent gets a badge. A community's centre is its label
		// anchor, not something the other members belong to, and a fan-out count
		// beside it would claim a relationship the data never stated.
		const badge =
			cluster.kind === "hub"
				? formatRepresented(cluster.represented, cluster.exact)
				: undefined;
		for (const nodeId of cluster.memberIds) {
			byNode.set(
				nodeId,
				nodeId === cluster.hubId
					? {
							clusterId: cluster.id,
							isHub: true,
							badge,
							represented: cluster.represented,
						}
					: { clusterId: cluster.id, isHub: false, represented: 0 },
			);
		}
	}
	return { clusters, byNode, epoch: clusterEpoch(clusters) };
}

/** Fraction of edges whose two ends landed in the same group. */
function withinGroupShare(data: SubgraphResult, model: ClusterModel): number {
	if (data.edges.length === 0) return 1;
	let withinGroup = 0;
	for (const edge of data.edges) {
		const source = model.byNode.get(edge.source);
		const target = model.byNode.get(edge.target);
		if (source && target && source.clusterId === target.clusterId) {
			withinGroup += 1;
		}
	}
	return withinGroup / data.edges.length;
}

/** The label most of a community shares, when most of it shares one. */
function dominantLabel(
	memberIds: readonly string[],
	labelById: ReadonlyMap<string, string>,
): string | null {
	const counts = new Map<string, number>();
	for (const id of memberIds) {
		const label = labelById.get(id);
		if (label) counts.set(label, (counts.get(label) ?? 0) + 1);
	}
	let best: string | null = null;
	let bestCount = 0;
	for (const [label, count] of [...counts].sort(
		(a, b) => b[1] - a[1] || compareIds(a[0], b[0]),
	)) {
		if (count > bestCount) {
			best = label;
			bestCount = count;
		}
	}
	return bestCount >= memberIds.length * DOMINANT_LABEL_SHARE ? best : null;
}

/**
 * Groups the sample by modularity, then hands the leftovers back to the by-type
 * grouping.
 *
 * A community of one or two is not a finding, it is a node that happened to sit
 * apart — drawn as its own disc it costs a frame and says nothing. Those rejoin
 * their object type, so the stage reads as "here are the clusters, and here is
 * everything else, filed."
 */
function buildCommunityClusters(
	data: SubgraphResult,
	byProminence: (a: string, b: string) => number,
	degree: ReadonlyMap<string, number>,
): GraphCluster[] {
	const labelById = new Map(data.nodes.map((node) => [node.id, node.label]));
	const captionById = new Map(
		data.nodes.map((node) => [node.id, captionOf(node)]),
	);
	const { members } = detectCommunities(
		data.nodes.map((node) => node.id),
		data.edges,
	);

	const clusters: GraphCluster[] = [];
	const loose: string[] = [];
	for (const bucket of members) {
		if (bucket.length < MIN_COMMUNITY_MEMBERS) {
			loose.push(...bucket);
			continue;
		}
		const ordered = [...bucket].sort(byProminence);
		const anchor = ordered[0];
		const label = dominantLabel(ordered, labelById);
		clusters.push({
			id: `community:${anchor}`,
			kind: "community",
			// Named for the object at its centre, like a hub group is — the members
			// are mixed by construction, so the object type rarely names anything.
			title: captionById.get(anchor) ?? anchor,
			subtitle: label ?? `${ordered.length.toLocaleString()} linked objects`,
			memberIds: ordered,
			// The most connected member centres the group and carries its caption.
			hubId: anchor,
			childIds: ordered.slice(1),
			represented: ordered.length,
			exact: true,
		});
	}

	const looseByLabel = new Map<string, string[]>();
	for (const id of loose) {
		const label = labelById.get(id) ?? "unknown";
		const bucket = looseByLabel.get(label);
		if (bucket) bucket.push(id);
		else looseByLabel.set(label, [id]);
	}
	for (const [label, ids] of looseByLabel) {
		const memberIds = [...ids].sort(byProminence);
		const connected = memberIds.some((id) => (degree.get(id) ?? 0) > 0);
		clusters.push({
			id: `type:${label}`,
			kind: "type",
			title: label,
			subtitle: connected ? undefined : NO_CONNECTIONS_SUBTITLE,
			memberIds,
			represented: memberIds.length,
			exact: true,
		});
	}

	return foldSmallest(
		clusters.sort(
			(a, b) =>
				b.memberIds.length - a.memberIds.length || compareIds(a.id, b.id),
		),
	);
}

/**
 * Beyond `MAX_CLUSTERS` the tail is the smallest groups, whose separate discs
 * would cost a frame without telling the reader anything, so they share one.
 * They keep every member — a folded group is drawn plainly, never dropped.
 */
function foldSmallest(clusters: GraphCluster[]): GraphCluster[] {
	if (clusters.length <= MAX_CLUSTERS) return clusters;

	const kept = clusters.slice(0, MAX_CLUSTERS - 1);
	const folded = clusters.slice(MAX_CLUSTERS - 1);
	const memberIds = folded.flatMap((cluster) => cluster.memberIds);

	kept.push({
		id: FOLDED_CLUSTER_ID,
		kind: "type",
		title: "Other objects",
		subtitle: `${folded.length} smaller groups`,
		memberIds,
		// Zero, not the member count: packing puts the highest `represented` at
		// the centre of the stage, and a bag of leftovers is the last thing that
		// should claim the spot reserved for what to look at first. Only a hub's
		// badge renders `represented`, and this group has no hub.
		represented: 0,
		exact: true,
	});
	return kept;
}
