import { INodePermission } from "./schema/flow/board";
import type { INode } from "./schema/flow/node";

/**
 * Pure view-model helpers for the app Packages page: what each linked package
 * may do (from its nodes' sandbox permissions, with the manifest supplying
 * resource tiers and the host allowlist) and how its nodes group by category.
 */

/** Mirror of `PackagePermissions::capability_tags()` order: most sensitive first. */
export const CAPABILITY_ORDER = [
	"net.http",
	"net.ws",
	"net.tcp",
	"net.udp",
	"net.dns",
	"oauth",
	"models",
	"database.read",
	"database.write",
	"storage.user",
	"storage.node",
	"storage.uploads",
	"storage.cache",
	"variables",
	"cache",
	"streaming",
	"a2ui",
] as const;

const STORAGE_TAGS = [
	"storage.user",
	"storage.node",
	"storage.uploads",
	"storage.cache",
] as const;

/** The sandbox gates every directory on one storage capability (`with_capabilities_from`). */
const NODE_PERMISSION_TAGS: Record<string, readonly string[]> = {
	[INodePermission.NetworkHttp]: ["net.http"],
	[INodePermission.NetworkWebsocket]: ["net.ws"],
	[INodePermission.NetworkTcp]: ["net.tcp"],
	[INodePermission.NetworkUdp]: ["net.udp"],
	[INodePermission.NetworkDns]: ["net.dns"],
	[INodePermission.StorageRead]: STORAGE_TAGS,
	[INodePermission.StorageWrite]: STORAGE_TAGS,
	[INodePermission.DatabaseRead]: ["database.read"],
	[INodePermission.DatabaseWrite]: ["database.write"],
	[INodePermission.Variables]: ["variables"],
	[INodePermission.Cache]: ["cache"],
	[INodePermission.Streaming]: ["streaming"],
	[INodePermission.Models]: ["models"],
	[INodePermission.A2ui]: ["a2ui"],
	[INodePermission.OAuth]: ["oauth"],
};

export type ElevatedAccessGroup =
	| "network"
	| "files"
	| "accounts"
	| "models"
	| "data";

export type AccessGroup = ElevatedAccessGroup | "runtime";

/** Groups that let a package reach past its own node: the network, files, accounts, models or data. */
export const ELEVATED_ACCESS_GROUPS: readonly ElevatedAccessGroup[] = [
	"network",
	"files",
	"accounts",
	"models",
	"data",
];

export interface ManifestAccess {
	description?: string;
	/** Raw `MemoryTier` key, e.g. `huge`. */
	memory?: string;
	/** Raw `TimeoutTier` key, e.g. `long_running`. */
	timeout?: string;
	allowedHosts: string[];
	oauthProviders: string[];
	/** Tags from the manifest's own capability flags, for packages without catalog nodes. */
	capabilityTags: string[];
}

export interface PackageAccess {
	tags: string[];
	/** Outbound allowlist; empty while `net.http` is set means any host. */
	hosts: string[];
	memory?: string;
	timeout?: string;
	oauthProviders: string[];
}

export interface NodeCategoryGroup {
	category: string;
	label: string;
	nodes: INode[];
	packageIds: string[];
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** The Rust wire is snake_case; the TS schema mirrors it in camelCase. */
function field(
	record: Record<string, unknown> | undefined,
	snake: string,
	camel: string,
): unknown {
	if (!record) return undefined;
	return record[snake] ?? record[camel];
}

function readRecord(
	record: Record<string, unknown> | undefined,
	snake: string,
	camel = snake,
): Record<string, unknown> | undefined {
	const value = field(record, snake, camel);
	return isRecord(value) ? value : undefined;
}

function readFlag(
	record: Record<string, unknown> | undefined,
	snake: string,
	camel = snake,
): boolean {
	return field(record, snake, camel) === true;
}

function readText(value: unknown): string | undefined {
	return typeof value === "string" && value.trim().length > 0
		? value
		: undefined;
}

function readStrings(value: unknown): string[] {
	return Array.isArray(value)
		? value.filter((item): item is string => typeof item === "string")
		: [];
}

const CAPABILITY_RANK = new Map<string, number>(
	CAPABILITY_ORDER.map((tag, index) => [tag, index]),
);

export function sortCapabilityTags(tags: Iterable<string>): string[] {
	return [...new Set(tags)].sort((a, b) => {
		const rankA = CAPABILITY_RANK.get(a) ?? CAPABILITY_ORDER.length;
		const rankB = CAPABILITY_RANK.get(b) ?? CAPABILITY_ORDER.length;
		return rankA === rankB ? a.localeCompare(b) : rankA - rankB;
	});
}

export function capabilityTagsFromNodePermissions(
	permissions: Iterable<string>,
): string[] {
	const tags: string[] = [];
	for (const permission of permissions) {
		tags.push(...(NODE_PERMISSION_TAGS[permission] ?? []));
	}
	return sortCapabilityTags(tags);
}

/** Tolerant read of the access-relevant parts of a package manifest. */
export function readManifestAccess(
	manifest: unknown,
): ManifestAccess | undefined {
	if (!isRecord(manifest)) return undefined;
	const permissions = readRecord(manifest, "permissions");
	const network = readRecord(permissions, "network");
	const filesystem = readRecord(permissions, "filesystem");
	const database = readRecord(permissions, "database");
	const oauthScopes = field(permissions, "oauth_scopes", "oauthScopes");
	const oauthProviders = Array.isArray(oauthScopes)
		? [
				...new Set(
					oauthScopes.flatMap((scope) =>
						isRecord(scope) && readText(scope.provider)
							? [scope.provider as string]
							: [],
					),
				),
			]
		: [];

	const flags: Array<[boolean, string]> = [
		[readFlag(network, "http_enabled", "httpEnabled"), "net.http"],
		[readFlag(network, "websocket_enabled", "websocketEnabled"), "net.ws"],
		[readFlag(network, "tcp_enabled", "tcpEnabled"), "net.tcp"],
		[readFlag(network, "udp_enabled", "udpEnabled"), "net.udp"],
		[readFlag(network, "dns_enabled", "dnsEnabled"), "net.dns"],
		[oauthProviders.length > 0, "oauth"],
		[readFlag(permissions, "models"), "models"],
		[readFlag(database, "read"), "database.read"],
		[readFlag(database, "write"), "database.write"],
		[readFlag(filesystem, "user_storage", "userStorage"), "storage.user"],
		[readFlag(filesystem, "node_storage", "nodeStorage"), "storage.node"],
		[readFlag(filesystem, "upload_dir", "uploadDir"), "storage.uploads"],
		[readFlag(filesystem, "cache_dir", "cacheDir"), "storage.cache"],
		[readFlag(permissions, "variables"), "variables"],
		[readFlag(permissions, "cache"), "cache"],
		[readFlag(permissions, "streaming"), "streaming"],
		[readFlag(permissions, "a2ui"), "a2ui"],
	];

	return {
		description: readText(manifest.description),
		memory: readText(permissions?.memory),
		timeout: readText(permissions?.timeout),
		allowedHosts: readStrings(field(network, "allowed_hosts", "allowedHosts")),
		oauthProviders,
		capabilityTags: flags.flatMap(([enabled, tag]) => (enabled ? [tag] : [])),
	};
}

/**
 * Node permissions are what the sandbox enforces, so they decide the tags
 * whenever the catalog lists the package's nodes; the manifest supplies the
 * tiers, the host allowlist and OAuth scopes either way.
 */
export function packageAccess(
	nodes: readonly INode[],
	manifest: ManifestAccess | undefined,
): PackageAccess {
	const nodeTags = nodes.length
		? capabilityTagsFromNodePermissions(
				nodes.flatMap((node) => node.wasm?.permissions ?? []),
			)
		: (manifest?.capabilityTags ?? []);
	const oauthTags = manifest?.oauthProviders.length ? ["oauth"] : [];
	return {
		tags: sortCapabilityTags([...nodeTags, ...oauthTags]),
		hosts: manifest?.allowedHosts ?? [],
		memory: manifest?.memory,
		timeout: manifest?.timeout,
		oauthProviders: manifest?.oauthProviders ?? [],
	};
}

export function accessGroupOf(tag: string): AccessGroup {
	if (tag.startsWith("net.") || tag === "widget.net") return "network";
	if (tag.startsWith("storage.")) return "files";
	if (tag.startsWith("database.")) return "data";
	if (tag === "oauth") return "accounts";
	if (tag === "models") return "models";
	return "runtime";
}

export function groupAccessTags(
	tags: readonly string[],
): Record<AccessGroup, string[]> {
	const groups: Record<AccessGroup, string[]> = {
		network: [],
		files: [],
		accounts: [],
		models: [],
		data: [],
		runtime: [],
	};
	for (const tag of tags) groups[accessGroupOf(tag)].push(tag);
	return groups;
}

export function hasElevatedAccess(tags: readonly string[]): boolean {
	return tags.some((tag) => accessGroupOf(tag) !== "runtime");
}

export function nodeDisplayName(node: INode): string {
	return node.friendly_name || node.name;
}

export function formatCategoryLabel(category: string): string {
	return category
		.split("/")
		.map((part) => part.trim())
		.filter(Boolean)
		.join(" / ");
}

export function groupPackageNodesByCategory(
	nodesByPackage: ReadonlyMap<string, readonly INode[]>,
	uncategorized: string,
): NodeCategoryGroup[] {
	const groups = new Map<string, NodeCategoryGroup>();
	for (const [packageId, nodes] of nodesByPackage) {
		for (const node of nodes) {
			const category = node.category?.trim() || uncategorized;
			let group = groups.get(category);
			if (!group) {
				group = {
					category,
					label: formatCategoryLabel(category) || category,
					nodes: [],
					packageIds: [],
				};
				groups.set(category, group);
			}
			group.nodes.push(node);
			if (!group.packageIds.includes(packageId)) {
				group.packageIds.push(packageId);
			}
		}
	}
	for (const group of groups.values()) {
		group.nodes.sort((a, b) =>
			nodeDisplayName(a).localeCompare(nodeDisplayName(b)),
		);
	}
	return [...groups.values()].sort((a, b) => a.label.localeCompare(b.label));
}

/** Case-insensitive match on node name, description, category or package name. */
export function filterNodeGroups(
	groups: readonly NodeCategoryGroup[],
	query: string,
	packageNames: ReadonlyMap<string, string>,
): NodeCategoryGroup[] {
	const needle = query.trim().toLowerCase();
	if (!needle) return [...groups];
	return groups.flatMap((group) => {
		const groupHit =
			group.label.toLowerCase().includes(needle) ||
			group.packageIds.some((id) =>
				(packageNames.get(id) ?? id).toLowerCase().includes(needle),
			);
		const nodes = groupHit
			? group.nodes
			: group.nodes.filter((node) =>
					[nodeDisplayName(node), node.description ?? ""].some((text) =>
						text.toLowerCase().includes(needle),
					),
				);
		return nodes.length ? [{ ...group, nodes }] : [];
	});
}
