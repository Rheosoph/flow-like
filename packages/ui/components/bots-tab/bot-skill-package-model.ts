import type {
	InstalledPackage,
	PackageManifest,
	PackageSummary,
} from "../../lib/schema/wasm";

export interface SkillPackageCandidate {
	id: string;
	name: string;
	description: string;
	version: string;
	installed: boolean;
	/** Capability tags from the registry listing or the manifest, most sensitive first. */
	tags: string[];
	skills: { name: string; description: string }[];
}

/** The bot capability a package capability tag needs; mirrors `uncovered_tags` on the server. */
export function capabilityForTag(tag: string): string | null {
	if (/^net\.(http|ws|tcp|udp|dns)$/.test(tag)) return "internet";
	if (tag === "oauth") return "connectors";
	if (tag === "database.read" || tag === "database.write") return "data_studio";
	const known = [
		"models",
		"variables",
		"cache",
		"streaming",
		"a2ui",
		"storage.user",
		"storage.node",
		"storage.uploads",
		"storage.cache",
		"widget.net",
	];
	return known.includes(tag) ? null : `unknown:${tag}`;
}

/** The capabilities (or unknown tags) the bot lacks to run the package's tools. */
export function uncoveredCapabilities(
	tags: readonly string[],
	held: readonly string[],
): string[] {
	const missing = new Set<string>();
	for (const tag of tags) {
		const needed = capabilityForTag(tag);
		if (needed && !held.includes(needed)) missing.add(needed);
	}
	return [...missing];
}

/** Tags a manifest's permissions imply, for packages the listing gives none for. */
export function tagsOfManifest(manifest: PackageManifest): string[] {
	const permissions = manifest.permissions;
	const tags: string[] = [];
	const network = permissions?.network as unknown as
		| Record<string, unknown>
		| undefined;
	if (network?.httpEnabled || network?.http_enabled) tags.push("net.http");
	if (network?.websocketEnabled || network?.websocket_enabled)
		tags.push("net.ws");
	if (network?.tcpEnabled || network?.tcp_enabled) tags.push("net.tcp");
	if (network?.udpEnabled || network?.udp_enabled) tags.push("net.udp");
	if (network?.dnsEnabled || network?.dns_enabled) tags.push("net.dns");
	const scopes = permissions?.oauthScopes ?? [];
	if (scopes.length > 0) tags.push("oauth");
	if (permissions?.database?.read) tags.push("database.read");
	if (permissions?.database?.write) tags.push("database.write");
	return tags;
}

/** A package that declares `[[skills]]`, or null. */
export function toCandidate(
	manifest: PackageManifest | undefined,
	summary: PackageSummary | undefined,
	installed: boolean,
): SkillPackageCandidate | null {
	const skills = (
		manifest as { skills?: { name: string; description: string }[] } | undefined
	)?.skills;
	if (!manifest || !skills?.length) return null;
	return {
		id: manifest.id,
		name: manifest.name,
		description: manifest.description,
		version: manifest.version,
		installed,
		tags: summary?.capabilities ?? tagsOfManifest(manifest),
		skills: skills.map(({ name, description }) => ({ name, description })),
	};
}

export function fromInstalled(
	pkg: InstalledPackage,
): SkillPackageCandidate | null {
	return toCandidate(pkg.manifest, undefined, true);
}
