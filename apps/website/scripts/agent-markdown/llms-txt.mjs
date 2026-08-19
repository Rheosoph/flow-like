/**
 * Renders /llms.txt (llmstxt.org) from a site's page inventory.
 *
 * Mirror of apps/docs/scripts/agent-markdown/llms-txt.mjs — keep both copies
 * in sync. The module only knows the file format; which pages appear, in what
 * order and under which heading is the caller's decision.
 */

/** Labels and notes are inline Markdown, so they have to stay on one line. */
function inline(value) {
	return String(value ?? "")
		.replace(/\s+/g, " ")
		.trim();
}

function escapeLabel(value) {
	return inline(value).replace(/([\\[\]])/g, "\\$1");
}

/**
 * The Markdown twin generate.mjs wrote next to the page. `/pricing/` resolves
 * through `/pricing.md`, the form the content negotiation middleware accepts.
 */
export function markdownUrl(pageUrl) {
	const url = new URL(pageUrl);
	url.hash = "";
	url.search = "";
	const path = url.pathname.replace(/\/+$/, "");
	url.pathname = path ? `${path}.md` : "/index.md";
	return url.toString();
}

/** @param {{title: string, url?: string, note?: string, depth?: number}} entry */
function renderEntry(entry) {
	const indent = "  ".repeat(entry.depth ?? 0);
	const label = escapeLabel(entry.title);
	const note = entry.note ? `: ${inline(entry.note)}` : "";
	const link = entry.url ? `[${label}](${entry.url})` : `**${label}**`;
	return `${indent}- ${link}${note}`;
}

/**
 * @param {object} page
 * @param {string} page.title       H1, the site name
 * @param {string} [page.summary]   blockquote directly under the H1
 * @param {string[]} [page.details] paragraphs between summary and sections
 * @param {Array<{heading: string, body?: string, entries?: object[]}>} page.sections
 */
export function renderLlmsTxt({ title, summary, details = [], sections }) {
	const lines = [`# ${inline(title)}`, ""];

	if (summary) lines.push(`> ${inline(summary)}`, "");
	for (const paragraph of details) lines.push(inline(paragraph), "");

	for (const section of sections) {
		const entries = (section.entries ?? []).filter(Boolean);
		if (!entries.length && !section.body) continue;

		lines.push(`## ${inline(section.heading)}`, "");
		if (section.body) lines.push(inline(section.body), "");
		for (const entry of entries) lines.push(renderEntry(entry));
		lines.push("");
	}

	return `${lines.join("\n").trimEnd()}\n`;
}
