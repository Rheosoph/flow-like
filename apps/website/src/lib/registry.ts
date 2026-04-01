const API_BASE =
	import.meta.env.REGISTRY_API_URL || "https://api.flow-like.com";
const API_URL = `${API_BASE}/api/v1`;
const PAT = import.meta.env.REGISTRY_PAT || "";

function headers(): HeadersInit {
	return {
		Authorization: `Bearer ${PAT}`,
		"Content-Type": "application/json",
	};
}

export function isStoreEnabled(): boolean {
	return import.meta.env.ENABLE_STORE_SEO === "true";
}

// ---------- Package types ----------

export interface MetaSummary {
	lang: string;
	name: string;
	description?: string;
	icon?: string;
	thumbnail?: string;
}

export interface PackageSummary {
	id: string;
	name: string;
	description: string;
	latestVersion: string;
	downloadCount: number;
	status: "active" | "deprecated" | "disabled" | "pending_review";
	keywords: string[];
	verified: boolean;
	price: number;
	visibility: string;
	metadata?: MetaSummary;
}

export interface SearchResults {
	packages: PackageSummary[];
	totalCount: number;
	offset: number;
	limit: number;
}

export interface PackageManifest {
	name: string;
	version: string;
	description: string;
	authors: string[];
	license?: string;
	homepage?: string;
	repository?: string;
	keywords: string[];
	nodes: unknown;
	permissions: unknown;
}

export interface PackageVersion {
	version: string;
	wasm_hash: string;
	wasm_size: number;
	download_url?: string;
	published_at: string;
	release_notes?: string;
	yanked: boolean;
}

export interface RegistryEntry {
	id: string;
	manifest: PackageManifest;
	versions: PackageVersion[];
	status: string;
	download_count: number;
	created_at: string;
	updated_at: string;
	verified: boolean;
	price: number;
	visibility: string;
}

export interface PackageMeta {
	name: string;
	description: string;
	long_description?: string;
	release_notes?: string;
	tags: string[];
	use_case?: string;
	icon?: string;
	thumbnail?: string;
	preview_media: string[];
	age_rating?: number;
	website?: string;
	support_url?: string;
	docs_url?: string;
}

// ---------- App types ----------

export interface AppData {
	id: string;
	price?: number;
	execution_mode: string;
	status: string;
	visibility: string;
	bits: string[];
	boards: string[];
	templates: string[];
	events: string[];
	changelog?: string;
	avg_rating?: number;
	download_count: number;
	interactions_count: number;
	rating_count: number;
	rating_sum: number;
	relevance_score?: number;
	primary_category?: string;
	secondary_category?: string;
	version?: string;
	created_at: string;
	updated_at: string;
}

export interface AppMetadata {
	name: string;
	description: string;
	long_description?: string;
	release_notes?: string;
	tags: string[];
	use_case?: string;
	icon?: string;
	thumbnail?: string;
	preview_media: string[];
	age_rating?: number;
	website?: string;
	support_url?: string;
	docs_url?: string;
}

export type AppSearchResult = [AppData, AppMetadata | null];

// ---------- API calls ----------

export async function searchPackages(
	limit = 100,
	offset = 0,
	language = "en",
): Promise<SearchResults> {
	const params = new URLSearchParams({
		limit: String(limit),
		offset: String(offset),
		sort_by: "downloads",
		sort_desc: "true",
	});
	if (language) params.set("language", language);

	const res = await fetch(`${API_URL}/registry/search?${params}`, {
		headers: headers(),
	});
	if (!res.ok)
		throw new Error(`Package search failed: ${res.status} ${res.statusText}`);
	return res.json();
}

export async function getPackage(id: string): Promise<RegistryEntry | null> {
	const res = await fetch(`${API_URL}/registry/package/${id}`, {
		headers: headers(),
	});
	if (res.status === 404) return null;
	if (!res.ok)
		throw new Error(
			`Package fetch failed: ${res.status} ${res.statusText}`,
		);
	return res.json();
}

export async function getPackageMeta(
	id: string,
	language = "en",
): Promise<PackageMeta | null> {
	const res = await fetch(
		`${API_URL}/registry/package/${id}/meta?language=${language}`,
		{ headers: headers() },
	);
	if (res.status === 404) return null;
	if (!res.ok) return null;
	return res.json();
}

export async function getPackageReadme(id: string): Promise<string | null> {
	const res = await fetch(`${API_URL}/registry/package/${id}/readme`, {
		headers: headers(),
	});
	if (res.status === 404) return null;
	if (!res.ok) return null;
	return res.text();
}

export async function searchApps(
	limit = 100,
	offset = 0,
	language = "en",
): Promise<AppSearchResult[]> {
	const params = new URLSearchParams({
		limit: String(limit),
		offset: String(offset),
		sort: "MostPopular",
	});
	if (language) params.set("language", language);

	const res = await fetch(`${API_URL}/apps/search?${params}`, {
		headers: headers(),
	});
	if (!res.ok)
		throw new Error(`App search failed: ${res.status} ${res.statusText}`);
	return res.json();
}

export async function getAppMeta(
	appId: string,
	language = "en",
): Promise<AppMetadata | null> {
	const res = await fetch(
		`${API_URL}/apps/${appId}/meta?language=${language}`,
		{ headers: headers() },
	);
	if (res.status === 404) return null;
	if (!res.ok) return null;
	return res.json();
}

// ---------- Helpers ----------

export function formatCategory(cat?: string): string {
	if (!cat) return "Other";

	const normalizedCategory = cat
		.replace(/_/g, " ")
		.replace(/([A-Z]+)([A-Z][a-z])/g, "$1 $2")
		.replace(/([a-z0-9])([A-Z])/g, "$1 $2")
		.replace(/\s+/g, " ")
		.trim();

	if (!normalizedCategory) return "Other";

	return normalizedCategory
		.split(" ")
		.map((word) => word.charAt(0).toUpperCase() + word.slice(1).toLowerCase())
		.join(" ");
}

export function formatPrice(price?: number): string {
	if (!price || price === 0) return "Free";
	return `$${(price / 100).toFixed(2)}`;
}

export function storeDeepLink(
	type: "app" | "package",
	id: string,
): { web: string; desktop: string } {
	const webBase = "https://app.flow-like.com";
	if (type === "app") {
		return {
			web: `${webBase}/store?id=${id}`,
			desktop: `flow-like://store?id=${id}`,
		};
	}
	return {
		web: `${webBase}/store/packages?id=${id}`,
		desktop: `flow-like://store/packages?id=${id}`,
	};
}

export function starRating(avg?: number): string {
	if (!avg) return "☆☆☆☆☆";
	const full = Math.round(avg);
	return "★".repeat(full) + "☆".repeat(5 - full);
}

const PLATE_JSON_PREFIX = "plate_json::";

function extractTextFromPlateNodes(nodes: unknown[]): string {
	const parts: string[] = [];
	for (const node of nodes) {
		if (!node || typeof node !== "object") continue;
		const n = node as Record<string, unknown>;
		if (typeof n.text === "string") {
			parts.push(n.text);
		}
		if (Array.isArray(n.children)) {
			parts.push(extractTextFromPlateNodes(n.children));
		}
	}
	return parts.join(" ");
}

function stripMarkdown(md: string): string {
	return md
		.replace(/!\[.*?\]\(.*?\)/g, "")
		.replace(/\[([^\]]*)\]\(.*?\)/g, "$1")
		.replace(/^#{1,6}\s+/gm, "")
		.replace(/(\*{1,3}|_{1,3})(.*?)\1/g, "$2")
		.replace(/~~(.*?)~~/g, "$1")
		.replace(/`{1,3}[^`]*`{1,3}/g, "")
		.replace(/^[\s>*+-]+/gm, "")
		.replace(/\n{2,}/g, " ")
		.replace(/\s+/g, " ")
		.trim();
}

/** Extract plain text from plate_json:: or markdown content (for SEO). */
export function extractPlainText(content: string, maxLength = 300): string {
	if (!content) return "";
	let text: string;
	if (content.startsWith(PLATE_JSON_PREFIX)) {
		try {
			const nodes = JSON.parse(content.slice(PLATE_JSON_PREFIX.length));
			text = extractTextFromPlateNodes(
				Array.isArray(nodes) ? nodes : [],
			);
		} catch {
			text = content.slice(PLATE_JSON_PREFIX.length);
		}
	} else {
		text = stripMarkdown(content);
	}
	text = text.replace(/\s+/g, " ").trim();
	if (text.length > maxLength) {
		return `${text.slice(0, maxLength - 1)}…`;
	}
	return text;
}

/** Check if content uses plate_json:: or markdown format. */
export function isPlateContent(content: string): boolean {
	return content.startsWith(PLATE_JSON_PREFIX);
}

/** Always treat as markdown for the TextEditor – plate_json:: is auto-detected. */
export function contentIsMarkdown(content: string): boolean {
	return !content.startsWith(PLATE_JSON_PREFIX);
}

/** Safely format a date string – returns null if invalid. */
export function formatDate(
	dateStr?: string | null,
	opts: Intl.DateTimeFormatOptions = { year: "numeric", month: "short", day: "numeric" },
): string | null {
	if (!dateStr) return null;
	const d = new Date(dateStr);
	if (Number.isNaN(d.getTime())) return null;
	return d.toLocaleDateString("en-US", opts);
}
