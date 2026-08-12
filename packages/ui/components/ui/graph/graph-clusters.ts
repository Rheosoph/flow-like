import type {
	GraphOverlay,
	SubgraphNode,
	SubgraphResult,
} from "../../../state/backend-state/graph-state";
import { hashLayoutSeed } from "./graph-layout";

/** Spine children a node needs before it reads as a parent rather than a peer. */
export const HUB_MIN_MEMBERS = 3;
/** Below this the plain force layout is already readable, so nothing is grouped. */
export const MIN_GROUPED_NODES = 24;
/** Ceiling on groups laid out separately; the smallest by-type ones fold into a tail. */
export const MAX_CLUSTERS = 400;
/** Subtitle for a group whose members have no edges in the loaded sample. */
const NO_CONNECTIONS_SUBTITLE = "no connections in this view";
const FOLDED_CLUSTER_ID = "type:__more__";

export type ClusterKind = "hub" | "type";

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

	const clusters = foldSmallest([...hubClusters, ...typeClusters]);
	if (clusters.length <= 1) return null;

	const byNode = new Map<string, ClusterAssignment>();
	for (const cluster of clusters) {
		const badge = formatRepresented(cluster.represented, cluster.exact);
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

	// Grouping is only worth having when the edges agree with it. On a peer
	// ontology — no parent-child spine, everything linked across labels — these
	// groups would be blobs joined by stage-crossing lines, strictly worse than
	// the force layout at showing where the structure is. Bail and let it run.
	if (data.edges.length > 0) {
		let withinGroup = 0;
		for (const edge of data.edges) {
			const source = byNode.get(edge.source);
			const target = byNode.get(edge.target);
			if (source && target && source.clusterId === target.clusterId) {
				withinGroup += 1;
			}
		}
		if (withinGroup * 2 < data.edges.length) return null;
	}

	return { clusters, byNode, epoch: clusterEpoch(clusters) };
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
