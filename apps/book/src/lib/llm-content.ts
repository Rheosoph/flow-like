import { CURRENT_BOOK_EDITION } from "./book-edition";
import {
	BOOK_NAME,
	BOOK_ORIGIN,
	type BookPageData,
	bookEntryPath,
	normalizeBookEntryId,
	resolveBookSeo,
} from "./seo";

export interface LlmBookEntry {
	readonly id: string;
	readonly body?: string;
	readonly data: BookPageData;
}

function absoluteUrl(path: string): string {
	return new URL(path, BOOK_ORIGIN).toString();
}

export function bookMarkdownPath(entryId: string): string {
	const normalized = normalizeBookEntryId(entryId);
	return normalized ? `/${normalized}/index.md` : "/index.md";
}

export function bookMarkdownUrl(entryId: string): string {
	return absoluteUrl(bookMarkdownPath(entryId));
}

function oneLine(value: string): string {
	return value.replace(/\s+/g, " ").trim();
}

function isMdxImportStart(line: string): boolean {
	return /^import(?:\s+type)?\s+(?:["'{*]|[A-Za-z_$][\w$]*(?:\s*,|\s+from\s+["']))/.test(
		line,
	);
}

function stripLeadingMdxImports(body: string): string {
	const lines = body.split("\n");
	let cursor = 0;

	while (cursor < lines.length) {
		const line = lines[cursor]?.trim() ?? "";
		if (!line) {
			cursor += 1;
			continue;
		}
		if (!isMdxImportStart(line)) break;

		while (cursor < lines.length) {
			const importLine = lines[cursor] ?? "";
			cursor += 1;
			if (
				importLine.trimEnd().endsWith(";") ||
				/(?:from\s+)?["'][^"']+["']\s*$/.test(importLine)
			) {
				break;
			}
		}
	}

	return lines.slice(cursor).join("\n");
}

interface MarkdownSegment {
	readonly content: string;
	readonly fenced: boolean;
}

function splitMarkdownCodeFences(body: string): MarkdownSegment[] {
	const lines = body.match(/[^\n]*(?:\n|$)/g)?.filter(Boolean) ?? [];
	const segments: MarkdownSegment[] = [];
	let content = "";
	let fenced = false;
	let fenceCharacter = "";
	let fenceLength = 0;

	const flush = () => {
		if (!content) return;
		segments.push({ content, fenced });
		content = "";
	};

	for (const lineWithEnding of lines) {
		const line = lineWithEnding.endsWith("\n")
			? lineWithEnding.slice(0, -1)
			: lineWithEnding;
		const marker = line.match(/^ {0,3}(`{3,}|~{3,})/);

		if (!fenced && marker) {
			flush();
			fenced = true;
			fenceCharacter = marker[1][0] ?? "";
			fenceLength = marker[1].length;
			content = lineWithEnding;
			continue;
		}

		content += lineWithEnding;
		if (
			fenced &&
			marker?.[1][0] === fenceCharacter &&
			marker[1].length >= fenceLength &&
			new RegExp(`^ {0,3}${fenceCharacter}{${fenceLength},}\\s*$`).test(line)
		) {
			flush();
			fenced = false;
			fenceCharacter = "";
			fenceLength = 0;
		}
	}

	flush();
	return segments;
}

function transformMarkdownProse(
	body: string,
	transform: (content: string) => string,
): string {
	return splitMarkdownCodeFences(body)
		.map((segment) =>
			segment.fenced ? segment.content : transform(segment.content),
		)
		.join("");
}

function markdownProse(body: string): string {
	return splitMarkdownCodeFences(body)
		.filter((segment) => !segment.fenced)
		.map((segment) => segment.content)
		.join("");
}

function stringAttribute(attributes: string, name: string): string | undefined {
	const match = attributes.match(
		new RegExp(`(?:^|\\s)${name}\\s*=\\s*(?:"([^"]*)"|'([^']*)')`),
	);
	return match?.[1] ?? match?.[2];
}

function asBlockquote(value: string): string {
	return value
		.trim()
		.split("\n")
		.map((line) => (line ? `> ${line}` : ">"))
		.join("\n");
}

function replaceAsides(body: string): string {
	return body.replace(
		/<Aside\b([^>]*)>([\s\S]*?)<\/Aside>/g,
		(_match, attributes: string, content: string) => {
			const title = stringAttribute(attributes, "title") ?? "Note";
			return asBlockquote(`**${title}**\n\n${content.trim()}`);
		},
	);
}

function replaceWorkflowFigures(body: string, canonical: string): string {
	return body.replace(
		/<WorkflowFigure\b([\s\S]*?)\/>/g,
		(_match, attributes: string) => {
			const alt =
				stringAttribute(attributes, "alt") ?? "Flow-Like workflow figure";
			const caption = stringAttribute(attributes, "caption");
			const lines = [`**Workflow figure:** ${alt}`];
			if (caption) lines.push("", caption);
			lines.push(
				"",
				`[View the figure in the canonical HTML chapter](${canonical}).`,
			);
			return asBlockquote(lines.join("\n"));
		},
	);
}

function replaceInteractiveComponents(body: string, canonical: string): string {
	return body.replace(
		/<IncidentDeskDemo\b[^>]*\/>/g,
		asBlockquote(
			`**Interactive example:** The canonical HTML chapter includes a local Incident Desk demonstration. [Open the interactive version](${canonical}).`,
		),
	);
}

function rewritePublishedBookLinks(
	body: string,
	entries: readonly LlmBookEntry[],
): string {
	const markdownByHtmlPath = new Map(
		entries.map((entry) => [
			bookEntryPath(entry.id),
			bookMarkdownUrl(entry.id),
		]),
	);

	return body.replace(/(\]\()([^\s)]+)(\))/g, (match, open, href, close) => {
		let parsed: URL;
		try {
			parsed = new URL(href, BOOK_ORIGIN);
		} catch {
			return match;
		}
		if (parsed.origin !== BOOK_ORIGIN) return match;

		const markdownUrl = markdownByHtmlPath.get(parsed.pathname);
		if (!markdownUrl) return match;
		return `${open}${markdownUrl}${parsed.hash}${close}`;
	});
}

function entryById(
	entries: readonly LlmBookEntry[],
): Map<string, LlmBookEntry> {
	return new Map(
		entries.map((entry) => [normalizeBookEntryId(entry.id), entry]),
	);
}

function chapterLine(entry: LlmBookEntry, label?: string): string {
	const title = entry.data.title.replace(/^\d+\.\s*/, "");
	const linkLabel = label ? `${label}: ${title}` : title;
	return `- [${linkLabel}](${bookMarkdownUrl(entry.id)}): ${oneLine(entry.data.description ?? "Read this FlowBook page.")}`;
}

function homeBody(entries: readonly LlmBookEntry[]): string {
	const byId = entryById(entries);
	const introduction = byId.get(CURRENT_BOOK_EDITION.introduction.entryId);
	const lines = [
		"FlowScript is Flow-Like's typed textual language for authoring Flows. It represents the same executable workflow as the visual Board while keeping execution inside Flow-Like's governed node model.",
		"",
		"The open edition is source-backed and distinguishes implemented behavior from design intent and release-specific claims. Release-check callouts are important limitations, not boilerplate.",
		"",
		"## Start reading",
		"",
	];
	if (introduction) lines.push(chapterLine(introduction, "Introduction"));
	lines.push(
		`- [Complete contents](${bookMarkdownUrl("contents")}): See all published and planned chapters, with publication status clearly marked.`,
		"",
		"## Published parts",
		"",
	);

	for (const part of CURRENT_BOOK_EDITION.parts) {
		const partEntry = byId.get(part.id);
		if (!partEntry) continue;
		lines.push(
			`- [${part.label}: ${part.title}](${bookMarkdownUrl(part.id)}): ${oneLine(part.description)}`,
		);
	}

	lines.push(
		"",
		"## Available formats",
		"",
		`- [Canonical web edition](${BOOK_ORIGIN}/): Human-readable HTML with interactive examples and workflow figures.`,
		`- [FlowBook PDF](${absoluteUrl("/flowbook.pdf")}): Downloadable open-edition book.`,
		`- [LLM content index](${absoluteUrl("/llms.txt")}): Curated machine-readable navigation to every published Markdown page.`,
	);

	return lines.join("\n");
}

function partChapterList(
	partId: string,
	entries: readonly LlmBookEntry[],
): string {
	const part = CURRENT_BOOK_EDITION.parts.find(
		(candidate) => candidate.id === partId,
	);
	if (!part) return "";
	const byId = entryById(entries);
	const lines = ["## Chapters in this part", ""];

	for (const chapter of part.chapters) {
		const entry = byId.get(chapter.entryId);
		if (entry) lines.push(chapterLine(entry, `Chapter ${chapter.number}`));
	}

	return lines.join("\n");
}

function readingNavigation(
	entryId: string,
	entries: readonly LlmBookEntry[],
): string {
	const normalized = normalizeBookEntryId(entryId);
	const orderedIds: readonly string[] = [
		CURRENT_BOOK_EDITION.introduction.entryId,
		...CURRENT_BOOK_EDITION.parts.flatMap((part) =>
			part.chapters.map((chapter) => chapter.entryId),
		),
	];
	const index = orderedIds.indexOf(normalized);
	if (index < 0) return "";

	const byId = entryById(entries);
	const lines = ["## Reading navigation", ""];
	const previous =
		index > 0 ? byId.get(orderedIds[index - 1] ?? "") : undefined;
	const next =
		index < orderedIds.length - 1
			? byId.get(orderedIds[index + 1] ?? "")
			: undefined;
	if (previous) lines.push(chapterLine(previous, "Previous"));
	if (next) lines.push(chapterLine(next, "Next"));
	lines.push(
		`- [Complete contents](${bookMarkdownUrl("contents")}): Return to the full FlowBook reading plan.`,
	);
	return lines.join("\n");
}

function documentType(entryId: string): string {
	const normalized = normalizeBookEntryId(entryId);
	if (!normalized) return "Book home";
	if (normalized === "contents") return "Contents and publication plan";
	if (CURRENT_BOOK_EDITION.parts.some((part) => part.id === normalized)) {
		return "Part overview";
	}
	if (normalized === CURRENT_BOOK_EDITION.introduction.entryId) {
		return "Introduction";
	}
	return "Book chapter";
}

function metadataBlock(entry: LlmBookEntry): string {
	const seo = resolveBookSeo(entry.id, entry.data);
	const canonical = absoluteUrl(seo.path);
	const lines = [
		`# ${entry.data.title}`,
		"",
		asBlockquote(seo.description),
		"",
		`- **Document type:** ${documentType(entry.id)}`,
		`- **Canonical HTML:** [${canonical}](${canonical})`,
		`- **Markdown alternate:** [${bookMarkdownUrl(entry.id)}](${bookMarkdownUrl(entry.id)})`,
		`- **Book:** ${BOOK_NAME}, ${CURRENT_BOOK_EDITION.subtitle}`,
		`- **Edition:** ${CURRENT_BOOK_EDITION.editionLabel}`,
		`- **Publisher:** ${CURRENT_BOOK_EDITION.publisher}`,
		`- **Language:** ${CURRENT_BOOK_EDITION.language}`,
		`- **Topics:** ${seo.topics.join(", ")}`,
	];

	if (seo.location?.kind === "chapter") {
		lines.push(`- **Chapter:** ${seo.location.number}`);
	}
	if (seo.location?.part) {
		lines.push(
			`- **Part:** ${seo.location.part.label}: ${seo.location.part.title}`,
		);
	}

	lines.push(
		`- **LLM index:** [${absoluteUrl("/llms.txt")}](${absoluteUrl("/llms.txt")})`,
	);
	return lines.join("\n");
}

export function renderBookEntryMarkdown(
	entry: LlmBookEntry,
	entries: readonly LlmBookEntry[],
	options: { readonly includeNavigation?: boolean } = {},
): string {
	const normalized = normalizeBookEntryId(entry.id);
	const canonical = absoluteUrl(bookEntryPath(entry.id));
	let body = normalized
		? stripLeadingMdxImports(entry.body ?? "")
		: homeBody(entries);

	body = transformMarkdownProse(body, (prose) => {
		let transformed = normalized
			? rewritePublishedBookLinks(prose, entries)
			: prose;
		transformed = replaceAsides(transformed);
		transformed = replaceWorkflowFigures(transformed, canonical);
		transformed = replaceInteractiveComponents(transformed, canonical);
		transformed = transformed.replace(/<BookHome\b[^>]*\/>/g, "");
		return transformed.replace(/<BookPartOverview\b[^>]*\/>/g, "");
	});
	body = transformMarkdownProse(body, (prose) =>
		prose.replace(/\n{3,}/g, "\n\n"),
	).trim();

	if (/^<[A-Z][A-Za-z0-9]*(?:\s|\/?>)/m.test(markdownProse(body))) {
		throw new Error(
			`Unsupported MDX component remains in Markdown output for ${entry.id}`,
		);
	}

	const additions = [];
	if (CURRENT_BOOK_EDITION.parts.some((part) => part.id === normalized)) {
		additions.push(partChapterList(normalized, entries));
	}
	if (options.includeNavigation !== false) {
		const navigation = readingNavigation(normalized, entries);
		if (navigation) additions.push(navigation);
	}

	return [metadataBlock(entry), body, ...additions]
		.filter(Boolean)
		.join("\n\n---\n\n")
		.concat("\n");
}

function llmsEntry(
	entry: LlmBookEntry | undefined,
	label?: string,
): string | undefined {
	if (!entry) return undefined;
	return `- [${label ?? entry.data.title}](${bookMarkdownUrl(entry.id)}): ${oneLine(entry.data.description ?? "Read this FlowBook page.")}`;
}

export function renderLlmsTxt(entries: readonly LlmBookEntry[]): string {
	const byId = entryById(entries);
	const home = byId.get("");
	const publishedChapterCount = CURRENT_BOOK_EDITION.parts.reduce(
		(total, part) => total + part.chapters.length,
		0,
	);
	const lastPublishedChapter = CURRENT_BOOK_EDITION.parts.reduce(
		(highest, part) =>
			Math.max(highest, ...part.chapters.map((chapter) => chapter.number)),
		0,
	);
	const lines = [
		"# FlowBook",
		"",
		"> FlowBook is the free, source-backed guide to Flow-Like FlowScript: a typed authoring language that represents one executable workflow as both code and a visual node graph.",
		"",
		`This index covers the ${CURRENT_BOOK_EDITION.editionLabel}. Use the Markdown links for clean retrieval and cite the corresponding canonical HTML URL when directing a reader to the book.`,
		"",
		"Important interpretation guidance:",
		"",
		"- Treat release-check callouts and current-status caveats as authoritative limitations.",
		`- Chapters 1-${lastPublishedChapter} and the introduction are published open drafts; Chapters ${lastPublishedChapter + 1}-26, the epilogue, and appendices are planned, not published.`,
		"- FlowScript is an authoring language. The current Flow-Like runtime executes the persisted Board graph.",
		"- Do not infer capabilities, compatibility, performance, or security guarantees beyond the evidence stated in a page.",
		"",
		"## Start here",
		"",
	];

	for (const line of [
		llmsEntry(home, "FlowBook overview"),
		llmsEntry(
			byId.get(CURRENT_BOOK_EDITION.introduction.entryId),
			"Introduction",
		),
		llmsEntry(byId.get("contents"), "Complete contents"),
	]) {
		if (line) lines.push(line);
	}

	for (const part of CURRENT_BOOK_EDITION.parts) {
		lines.push("", `## ${part.label}: ${part.title}`, "");
		const partOverview = byId.get(part.id);
		if (partOverview) {
			lines.push(llmsEntry(partOverview, `${part.label} overview`) ?? "");
		}
		for (const chapter of part.chapters) {
			const entry = byId.get(chapter.entryId);
			if (entry) lines.push(chapterLine(entry, `Chapter ${chapter.number}`));
		}
	}

	lines.push(
		"",
		"## Optional",
		"",
		`- [Complete FlowBook Markdown](${absoluteUrl("/llms-full.txt")}): Single-file context containing the introduction and all ${publishedChapterCount} published chapters; prefer the individual page links above when only one topic is needed.`,
		`- [Canonical FlowBook website](${BOOK_ORIGIN}/): Human-readable edition with interactive examples, search, and workflow figures.`,
		`- [FlowBook PDF](${absoluteUrl("/flowbook.pdf")}): Downloadable open-edition book.`,
		"- [Flow-Like project](https://flow-like.com): Product and platform context.",
		"- [Flow-Like source repository](https://github.com/Rheosoph/flow-like): Source code and implementation evidence referenced by the book.",
	);

	return lines.join("\n").concat("\n");
}

export function renderLlmsFullTxt(entries: readonly LlmBookEntry[]): string {
	const byId = entryById(entries);
	const readingOrder = [
		CURRENT_BOOK_EDITION.introduction.entryId,
		...CURRENT_BOOK_EDITION.parts.flatMap((part) =>
			part.chapters.map((chapter) => chapter.entryId),
		),
	];
	const pages = readingOrder.flatMap((entryId) => {
		const entry = byId.get(entryId);
		return entry
			? [renderBookEntryMarkdown(entry, entries, { includeNavigation: false })]
			: [];
	});
	const preamble = [
		"# FlowBook: Complete Markdown Edition",
		"",
		`> A single-file rendering of the FlowBook introduction and all ${readingOrder.length - 1} chapters published in the open 2026 edition.`,
		"",
		`- **Canonical book:** [${BOOK_ORIGIN}/](${BOOK_ORIGIN}/)`,
		`- **Selective LLM index:** [${absoluteUrl("/llms.txt")}](${absoluteUrl("/llms.txt")})`,
		`- **Edition:** ${CURRENT_BOOK_EDITION.editionLabel}`,
		`- **Publisher:** ${CURRENT_BOOK_EDITION.publisher}`,
		`- **Language:** ${CURRENT_BOOK_EDITION.language}`,
		`- **Published reading units:** ${pages.length}`,
		"",
		"Use the per-page canonical HTML URLs in each document's metadata when citing or directing readers. Release-check callouts and current-status caveats remain authoritative limitations.",
	].join("\n");

	return [preamble, ...pages].join("\n\n---\n\n").concat("\n");
}
