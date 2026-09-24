import {
	type AccessGroup,
	accessGroupOf,
	readManifestAccess,
	sortCapabilityTags,
} from "../../lib/app-package-overview";
import {
	WIDGET_NET_CAPABILITY,
	capabilitySeverity,
} from "../../lib/package-capabilities";
import { packageNeedsLicense } from "../../lib/package-license";
import { readManifestWidgets } from "../../lib/package-widgets";
import { asArray, isRecord } from "../../lib/response-shape";
import type { InstalledPackage, PackageSummary } from "../../lib/schema/wasm";

export type ShelfKind = "packages" | "templates";
export type ShelfAccessFacet = Exclude<AccessGroup, "runtime"> | "widgets";
export type ShelfSort = "relevance" | "name" | "least" | "most";
/** `needed`: a holder must get the package before a project may pin it. */
export type ShelfLicense = "none" | "owned" | "needed";

export const SHELF_ACCESS_FACETS: readonly ShelfAccessFacet[] = [
	"network",
	"files",
	"accounts",
	"models",
	"data",
	"widgets",
];

export interface ShelfPackage {
	id: string;
	name: string;
	version: string;
	description: string;
	keywords: string[];
	authors: string[];
	tags: string[];
	hosts: string[];
	facets: ShelfAccessFacet[];
	widgetCount: number;
	/** Raw `MemoryTier` key; only local manifests carry it. */
	memory?: string;
	/** Registry results only. */
	downloadCount?: number;
	price: number;
	verified: boolean;
	template: boolean;
	license: ShelfLicense;
}

export interface ShelfPin {
	facet: ShelfAccessFacet | "more";
	tags: string[];
	elevated: boolean;
	count?: number;
	host?: string;
}

const TEMPLATE_KEYWORDS = ["template", "example"];

const MEMORY_LIMITS: Record<string, string> = {
	minimal: "16 MB",
	light: "32 MB",
	standard: "64 MB",
	heavy: "128 MB",
	intensive: "256 MB",
	large: "512 MB",
	huge: "1 GB",
	extreme: "2 GB",
	maximum: "4 GB",
};

/** Scaffolds from the package templates keep both of these keywords until edited. */
export function isTemplatePackage(keywords: readonly string[]): boolean {
	const lower = new Set(keywords.map((k) => k.toLowerCase()));
	return TEMPLATE_KEYWORDS.every((k) => lower.has(k));
}

export function memoryLimitLabel(tier: string | undefined): string | undefined {
	return tier ? MEMORY_LIMITS[tier] : undefined;
}

export function packageInitials(name: string): string {
	const words = name.replace(/[()&]/g, " ").split(/\s+/).filter(Boolean);
	if (words.length === 0) return "?";
	if (words.length === 1)
		return words[0].charAt(0).toUpperCase() + words[0].slice(1, 2);
	return (words[0].charAt(0) + words[1].charAt(0)).toUpperCase();
}

function facetOfTag(tag: string): ShelfAccessFacet | undefined {
	if (tag === WIDGET_NET_CAPABILITY) return "network";
	const group = accessGroupOf(tag);
	return group === "runtime" ? undefined : group;
}

function facetsOf(tags: readonly string[], widgetCount: number) {
	const present = new Set<ShelfAccessFacet>();
	for (const tag of tags) {
		const facet = facetOfTag(tag);
		if (facet) present.add(facet);
	}
	if (widgetCount > 0 || tags.includes(WIDGET_NET_CAPABILITY))
		present.add("widgets");
	return SHELF_ACCESS_FACETS.filter((facet) => present.has(facet));
}

function licenseOf(pkg: PackageSummary): ShelfLicense {
	if (!packageNeedsLicense(pkg)) return "none";
	if (pkg.viewerHasAccess === true) return "owned";
	return pkg.viewerHasAccess === false ? "needed" : "none";
}

function readAuthors(value: unknown): string[] {
	return asArray(value as unknown[]).flatMap((author) =>
		isRecord(author) && typeof author.name === "string" && author.name.trim()
			? [author.name.trim()]
			: [],
	);
}

export function shelfPackageFromSummary(pkg: PackageSummary): ShelfPackage {
	const keywords = asArray(pkg.keywords);
	const tags = sortCapabilityTags(asArray(pkg.capabilities));
	return {
		id: pkg.id,
		name: pkg.name,
		version: pkg.latestVersion,
		description: pkg.description ?? "",
		keywords,
		authors: [],
		tags,
		hosts: [],
		facets: facetsOf(tags, 0),
		widgetCount: 0,
		downloadCount: pkg.downloadCount,
		price: pkg.price ?? 0,
		verified: pkg.verified === true,
		template: isTemplatePackage(keywords),
		license: licenseOf(pkg),
	};
}

export function shelfPackageFromInstalled(pkg: InstalledPackage): ShelfPackage {
	const manifest: Partial<InstalledPackage["manifest"]> = isRecord(pkg.manifest)
		? pkg.manifest
		: {};
	const access = readManifestAccess(manifest);
	const keywords = asArray(manifest.keywords).filter(
		(k): k is string => typeof k === "string",
	);
	const widgetCount = readManifestWidgets(manifest).length;
	const tags = sortCapabilityTags([
		...(access?.capabilityTags ?? []),
		...(access?.oauthProviders.length ? ["oauth"] : []),
	]);
	return {
		id: manifest.id ?? pkg.id,
		name: manifest.name ?? pkg.id,
		version: manifest.version ?? pkg.version,
		description: manifest.description ?? "",
		keywords,
		authors: readAuthors(manifest.authors),
		tags,
		hosts: access?.allowedHosts ?? [],
		facets: facetsOf(tags, widgetCount),
		widgetCount,
		memory: access?.memory,
		price: 0,
		verified: false,
		template: isTemplatePackage(keywords),
		license: "none",
	};
}

/** Distinct elevated facets dominate; the raw tag count breaks ties. */
export function accessWeight(pkg: ShelfPackage): number {
	const elevated = pkg.facets.filter((facet) => facet !== "widgets").length;
	return elevated * 100 + pkg.tags.length;
}

const byName = (a: ShelfPackage, b: ShelfPackage) =>
	a.name.localeCompare(b.name, undefined, { sensitivity: "base" });

export function sortShelf(
	pkgs: readonly ShelfPackage[],
	sort: ShelfSort,
): ShelfPackage[] {
	if (sort === "relevance") return [...pkgs];
	if (sort === "name") return [...pkgs].sort(byName);
	const direction = sort === "least" ? 1 : -1;
	return [...pkgs].sort(
		(a, b) => direction * (accessWeight(a) - accessWeight(b)) || byName(a, b),
	);
}

export function hasFacet(pkg: ShelfPackage, facet: ShelfAccessFacet): boolean {
	return pkg.facets.includes(facet);
}

export function hasTopic(pkg: ShelfPackage, topic: string): boolean {
	return pkg.keywords.some((k) => k.toLowerCase() === topic);
}

/** Keywords shared by at least `min` packages, most common first; template markers excluded. */
export function shelfTopics(
	pkgs: readonly ShelfPackage[],
	{ limit = 8, min = 2 }: { limit?: number; min?: number } = {},
): string[] {
	const counts = new Map<string, number>();
	for (const pkg of pkgs) {
		for (const keyword of new Set(pkg.keywords.map((k) => k.toLowerCase()))) {
			if (TEMPLATE_KEYWORDS.includes(keyword)) continue;
			counts.set(keyword, (counts.get(keyword) ?? 0) + 1);
		}
	}
	return [...counts.entries()]
		.filter(([, count]) => count >= min)
		.sort(([a, ca], [b, cb]) => cb - ca || a.localeCompare(b))
		.slice(0, limit)
		.map(([topic]) => topic);
}

/** What the card cover draws as output pins: access first, widgets last, overflow folded. */
export function shelfPins(pkg: ShelfPackage, max = 3): ShelfPin[] {
	const pins: ShelfPin[] = pkg.facets.map((facet) => {
		if (facet === "widgets")
			return { facet, tags: [], elevated: false, count: pkg.widgetCount };
		const tags = pkg.tags.filter((tag) => facetOfTag(tag) === facet);
		return {
			facet,
			tags,
			elevated: tags.some((tag) => capabilitySeverity(tag) === "elevated"),
			host:
				facet === "network" && pkg.hosts.length === 1
					? pkg.hosts[0]
					: undefined,
		};
	});
	if (pins.length <= max) return pins;
	const shown = pins.slice(0, max - 1);
	return [
		...shown,
		{
			facet: "more",
			tags: pins.slice(max - 1).flatMap((pin) => pin.tags),
			elevated: false,
			count: pins.length - shown.length,
		},
	];
}
