/**
 * Request-time half of the agent Markdown support: serves the Markdown twin
 * that scripts/agent-markdown/generate.mjs wrote next to every built page.
 *
 * Copied verbatim into dist/server/ by scripts/prepare-workers-sites-deploy.mjs,
 * so this module must stay dependency free and Workers compatible.
 */

const MARKDOWN_TYPES = ["text/markdown", "text/x-markdown"];

function parseAccept(header) {
	const entries = new Map();

	for (const part of header.split(",")) {
		const [rawType, ...params] = part.split(";");
		const type = rawType.trim().toLowerCase();
		if (!type) continue;

		const q = params
			.map((param) => /^\s*q\s*=\s*([\d.]+)\s*$/i.exec(param))
			.find(Boolean);
		const quality = q ? Number.parseFloat(q[1]) : 1;
		entries.set(type, Number.isFinite(quality) ? quality : 1);
	}

	return entries;
}

/**
 * Markdown wins when the client asked for it explicitly and ranks it at least
 * as high as HTML — browsers never list `text/markdown`, so HTML stays default.
 */
export function prefersMarkdown(request) {
	if (request.method !== "GET" && request.method !== "HEAD") return false;

	const accept = request.headers.get("accept");
	if (!accept) return false;

	const entries = parseAccept(accept);
	const markdown = Math.max(
		...MARKDOWN_TYPES.map((type) => entries.get(type) ?? 0),
	);
	if (markdown <= 0) return false;

	const html = Math.max(
		entries.get("text/html") ?? 0,
		entries.get("application/xhtml+xml") ?? 0,
	);
	return markdown >= html;
}

export function isMarkdownPath(pathname) {
	return pathname.endsWith(".md");
}

/**
 * Mirrors how the asset server resolves HTML: `/pricing` is backed by either
 * `pricing/index.html` or `pricing.html`, so both twins are worth trying.
 */
export function markdownCandidates(pathname) {
	if (isMarkdownPath(pathname)) {
		if (pathname.endsWith("/index.md")) return [pathname];
		return [pathname, `${pathname.slice(0, -".md".length)}/index.md`];
	}
	if (pathname.endsWith("/")) return [`${pathname}index.md`];
	if (/\.[a-z0-9]+$/i.test(pathname)) return [];
	return [`${pathname}/index.md`, `${pathname}.md`];
}

function estimateTokens(text) {
	return Math.max(1, Math.ceil(text.length / 4));
}

function buildResponse(request, assetResponse, markdown, canonicalUrl) {
	const headers = new Headers();
	headers.set("content-type", "text/markdown; charset=utf-8");
	headers.set("x-markdown-tokens", String(estimateTokens(markdown)));
	headers.set("vary", "accept");
	headers.set("link", `<${canonicalUrl}>; rel="canonical"`);

	const cacheControl = assetResponse.headers.get("cache-control");
	if (cacheControl) headers.set("cache-control", cacheControl);

	return new Response(request.method === "HEAD" ? null : markdown, {
		status: 200,
		headers,
	});
}

/**
 * @param {Request} request
 * @param {(path: string) => Promise<Response>} fetchAsset
 * @returns {Promise<Response | null>} markdown response, or null to fall through
 */
export async function serveMarkdown(request, fetchAsset) {
	const url = new URL(request.url);
	const explicit = isMarkdownPath(url.pathname);
	if (!explicit && !prefersMarkdown(request)) return null;

	for (const candidate of markdownCandidates(url.pathname)) {
		const response = await fetchAsset(candidate);
		if (!response || response.status !== 200) continue;

		const markdown = await response.text();
		const canonical = new URL(
			explicit ? url.pathname.replace(/(?:\/index)?\.md$/, "/") : url.pathname,
			url.origin,
		).toString();
		return buildResponse(request, response, markdown, canonical);
	}

	return null;
}

/**
 * HTML and Markdown share one URL, so caches must key on the Accept header.
 */
export function withVaryAccept(response) {
	const type = response.headers.get("content-type") ?? "";
	if (!type.includes("text/html")) return response;

	const vary = response.headers.get("vary");
	const values = vary
		? vary.split(",").map((value) => value.trim().toLowerCase())
		: [];
	if (values.includes("accept") || values.includes("*")) return response;

	const next = new Response(response.body, response);
	next.headers.set("vary", [...values, "accept"].filter(Boolean).join(", "));
	return next;
}
