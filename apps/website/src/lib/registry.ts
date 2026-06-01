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

export interface PackageNodeSummary {
	id: string;
	name: string;
	friendly_name?: string;
	description?: string;
	category?: string;
	icon?: string | null;
	permissions?: string[];
}

export interface PackageVersion {
	version: string;
	wasmHash: string;
	wasmSize: number;
	downloadUrl?: string;
	publishedAt: string;
	releaseNotes?: string;
	yanked: boolean;
}

export interface RegistryEntry {
	id: string;
	manifest: PackageManifest;
	nodes?: PackageNodeSummary[];
	versions: PackageVersion[];
	status: string;
	downloadCount: number;
	createdAt: string;
	updatedAt: string;
	verified: boolean;
	price: number;
	visibility: string;
}

export interface PackageMeta {
	id?: string;
	lang?: string;
	name: string;
	description?: string;
	longDescription?: string;
	releaseNotes?: string;
	tags: string[];
	useCase?: string;
	icon?: string;
	thumbnail?: string;
	previewMedia: string[];
	ageRating?: number;
	website?: string;
	supportUrl?: string;
	docsUrl?: string;
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
		throw new Error(`Package fetch failed: ${res.status} ${res.statusText}`);
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
	return normalizePackageMeta(await res.json());
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
	return cat.replace(/_/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
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

type RawPackageMeta = Partial<PackageMeta> & {
	long_description?: string;
	release_notes?: string;
	use_case?: string;
	preview_media?: string[];
	age_rating?: number;
	support_url?: string;
	docs_url?: string;
};

function normalizePackageMeta(raw: RawPackageMeta): PackageMeta {
	return {
		id: raw.id,
		lang: raw.lang,
		name: raw.name ?? "",
		description: raw.description,
		longDescription: raw.longDescription ?? raw.long_description,
		releaseNotes: raw.releaseNotes ?? raw.release_notes,
		tags: raw.tags ?? [],
		useCase: raw.useCase ?? raw.use_case,
		icon: raw.icon,
		thumbnail: raw.thumbnail,
		previewMedia: raw.previewMedia ?? raw.preview_media ?? [],
		ageRating: raw.ageRating ?? raw.age_rating,
		website: raw.website,
		supportUrl: raw.supportUrl ?? raw.support_url,
		docsUrl: raw.docsUrl ?? raw.docs_url,
	};
}

export function starRating(avg?: number): string {
	if (!avg) return "☆☆☆☆☆";
	const full = Math.round(avg);
	return "★".repeat(full) + "☆".repeat(5 - full);
}

const PLATE_JSON_PREFIX = "plate_json::";

type PlateTextNode = {
	text?: string;
	bold?: boolean;
	italic?: boolean;
	underline?: boolean;
	strikethrough?: boolean;
	code?: boolean;
};

type PlateElementNode = {
	type?: string;
	children?: PlateNode[];
	url?: string;
	listStyleType?: string;
	[key: string]: unknown;
};

type PlateNode = PlateTextNode | PlateElementNode;

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

function escapeHtml(text: string): string {
	return text
		.replace(/&/g, "&amp;")
		.replace(/</g, "&lt;")
		.replace(/>/g, "&gt;")
		.replace(/"/g, "&quot;")
		.replace(/'/g, "&#39;");
}

function safeUrl(url: unknown): string | null {
	if (typeof url !== "string") return null;
	if (
		url.startsWith("/") ||
		url.startsWith("./") ||
		url.startsWith("../") ||
		url.startsWith("#")
	) {
		return url;
	}
	try {
		const parsed = new URL(url);
		if (["http:", "https:", "mailto:"].includes(parsed.protocol)) {
			return parsed.toString();
		}
	} catch {
		return null;
	}
	return null;
}

function renderInlineMarkdown(text: string): string {
	const linkTokens: string[] = [];
	const withLinkTokens = text.replace(
		/\[([^\]]+)\]\(([^)\s]+)\)/g,
		(match, label: string, url: string) => {
			const href = safeUrl(url);
			if (!href) return match;
			const token = `\u0000${linkTokens.length}\u0000`;
			linkTokens.push(
				`<a href="${escapeHtml(href)}" target="_blank" rel="noopener">${renderInlineMarkdown(label)}</a>`,
			);
			return token;
		},
	);

	return escapeHtml(withLinkTokens)
		.replace(/`([^`]+)`/g, "<code>$1</code>")
		.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
		.replace(/__([^_]+)__/g, "<strong>$1</strong>")
		.replace(/\*([^*]+)\*/g, "<em>$1</em>")
		.replace(/_([^_]+)_/g, "<em>$1</em>")
		.replace(
			/\u0000(\d+)\u0000/g,
			(_, index: string) => linkTokens[Number(index)] ?? "",
		);
}

function renderMarkdown(content: string): string {
	const lines = content.replace(/\r\n?/g, "\n").split("\n");
	const html: string[] = [];
	let paragraph: string[] = [];
	let listItems: string[] = [];
	let listType: "ul" | "ol" | null = null;
	let codeFence: string[] | null = null;

	const flushParagraph = () => {
		if (!paragraph.length) return;
		html.push(`<p>${renderInlineMarkdown(paragraph.join(" "))}</p>`);
		paragraph = [];
	};
	const flushList = () => {
		if (!listType || !listItems.length) return;
		html.push(`<${listType}>${listItems.join("")}</${listType}>`);
		listItems = [];
		listType = null;
	};

	for (const line of lines) {
		if (line.trim().startsWith("```") || line.trim().startsWith("~~~")) {
			if (codeFence) {
				html.push(
					`<pre><code>${escapeHtml(codeFence.join("\n"))}</code></pre>`,
				);
				codeFence = null;
			} else {
				flushParagraph();
				flushList();
				codeFence = [];
			}
			continue;
		}

		if (codeFence) {
			codeFence.push(line);
			continue;
		}

		if (!line.trim()) {
			flushParagraph();
			flushList();
			continue;
		}

		const heading = /^(#{1,6})\s+(.+)$/.exec(line);
		if (heading) {
			flushParagraph();
			flushList();
			const level = heading[1].length;
			html.push(
				`<h${level}>${renderInlineMarkdown(heading[2].trim())}</h${level}>`,
			);
			continue;
		}

		const unordered = /^\s*[-*+]\s+(.+)$/.exec(line);
		const ordered = /^\s*\d+[.)]\s+(.+)$/.exec(line);
		if (unordered || ordered) {
			flushParagraph();
			const nextType = unordered ? "ul" : "ol";
			if (listType && listType !== nextType) flushList();
			listType = nextType;
			listItems.push(
				`<li>${renderInlineMarkdown((unordered?.[1] ?? ordered?.[1] ?? "").trim())}</li>`,
			);
			continue;
		}

		paragraph.push(line.trim());
	}

	flushParagraph();
	flushList();
	if (codeFence) {
		html.push(`<pre><code>${escapeHtml(codeFence.join("\n"))}</code></pre>`);
	}
	return html.join("");
}

function isTextNode(node: PlateNode): node is PlateTextNode {
	return typeof (node as PlateTextNode).text === "string";
}

function renderPlateInline(node: PlateNode): string {
	if (isTextNode(node)) {
		let html = escapeHtml(node.text ?? "");
		if (node.code) html = `<code>${html}</code>`;
		if (node.bold) html = `<strong>${html}</strong>`;
		if (node.italic) html = `<em>${html}</em>`;
		if (node.underline) html = `<u>${html}</u>`;
		if (node.strikethrough) html = `<s>${html}</s>`;
		return html;
	}

	const children = renderPlateChildren(node.children);
	if (node.type === "a") {
		const href = safeUrl(node.url);
		if (!href) return children;
		return `<a href="${escapeHtml(href)}" target="_blank" rel="noopener">${children}</a>`;
	}
	return children;
}

function renderPlateChildren(children: PlateNode[] | undefined): string {
	return (children ?? []).map(renderPlateInline).join("");
}

function renderPlateBlock(node: PlateNode): string {
	if (isTextNode(node)) return renderPlateInline(node);

	const type = node.type ?? "p";
	const children = renderPlateChildren(node.children);
	if (!children.trim() && type !== "img") return "";

	if (/^h[1-6]$/.test(type)) return `<${type}>${children}</${type}>`;
	if (type === "blockquote") return `<blockquote>${children}</blockquote>`;
	if (type === "code_block") return `<pre><code>${children}</code></pre>`;
	if (type === "li") return `<li>${children}</li>`;
	if (type === "ul" || type === "ol") {
		const listChildren = (node.children ?? []).map(renderPlateBlock).join("");
		return `<${type}>${listChildren || children}</${type}>`;
	}
	if (type === "img") {
		const src = safeUrl(node.url);
		return src ? `<img src="${escapeHtml(src)}" alt="" loading="lazy" />` : "";
	}
	if (type === "tr" || type === "td" || type === "th") {
		const cellChildren = (node.children ?? []).map(renderPlateBlock).join("");
		return `<${type}>${cellChildren || children}</${type}>`;
	}
	if (type === "table") {
		const tableChildren = (node.children ?? []).map(renderPlateBlock).join("");
		return `<table>${tableChildren || children}</table>`;
	}
	if (node.listStyleType) {
		const listType = node.listStyleType === "decimal" ? "ol" : "ul";
		return `<${listType}><li>${children}</li></${listType}>`;
	}
	return `<p>${children}</p>`;
}

function renderPlateContent(content: string): string {
	try {
		const nodes = JSON.parse(content.slice(PLATE_JSON_PREFIX.length));
		if (!Array.isArray(nodes)) return "";
		return nodes.map((node) => renderPlateBlock(node as PlateNode)).join("");
	} catch {
		return `<p>${escapeHtml(content.slice(PLATE_JSON_PREFIX.length))}</p>`;
	}
}

/** Render trusted store rich text as sanitized server-side HTML. */
export function renderRichContent(content: string): string {
	if (!content.trim()) return "";
	if (content.startsWith(PLATE_JSON_PREFIX)) return renderPlateContent(content);
	return renderMarkdown(content);
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
			text = extractTextFromPlateNodes(Array.isArray(nodes) ? nodes : []);
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
	opts: Intl.DateTimeFormatOptions = {
		year: "numeric",
		month: "short",
		day: "numeric",
	},
): string | null {
	if (!dateStr) return null;
	const d = new Date(dateStr);
	if (Number.isNaN(d.getTime())) return null;
	return d.toLocaleDateString("en-US", opts);
}
