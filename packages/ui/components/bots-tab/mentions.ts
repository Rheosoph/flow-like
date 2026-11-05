import { MarkdownPlugin, markdownToAstProcessor } from "@platejs/markdown";
import type { Nodes, Root } from "mdast";
import { createSlateEditor } from "platejs";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import type { IConversation } from "../../state/backend-state/inbox-state";
import { isSelfPrincipal } from "./crew-model";
import type { CrewDirectory } from "./use-crew-directory";

export interface Mentionable {
	id: string;
	name: string;
	kind: "user" | "bot";
}

export interface MentionQuery {
	start: number;
	query: string;
}

const TRIGGER = /(^|\s)@([^\s@]*)$/;
const MAX_SUGGESTIONS = 6;

/** The `@word` being typed right before the caret, if any. */
export function mentionQuery(text: string, caret: number): MentionQuery | null {
	const match = TRIGGER.exec(text.slice(0, caret));
	if (!match) return null;
	return {
		start: caret - match[2].length - 1,
		query: match[2],
	};
}

/** Replaces the `@word` at `start..caret` with `@Name ` and returns the new caret. */
export function insertMention(
	text: string,
	range: MentionQuery,
	caret: number,
	name: string,
): { text: string; caret: number } {
	const token = `@${name} `;
	return {
		text: text.slice(0, range.start) + token + text.slice(caret),
		caret: range.start + token.length,
	};
}

export function filterMentionables(
	options: readonly Mentionable[],
	query: string,
): Mentionable[] {
	const needle = query.trim().toLowerCase();
	return options
		.filter((option) => option.name.toLowerCase().includes(needle))
		.sort(
			(a, b) =>
				Number(b.name.toLowerCase().startsWith(needle)) -
				Number(a.name.toLowerCase().startsWith(needle)),
		)
		.slice(0, MAX_SUGGESTIONS);
}

function escapeRegExp(value: string): string {
	return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/**
 * The principals whose `@Name` stands in the text as a whole word, picked from the menu or typed
 * out, so "@Scout" names Scout and "@Scouting" doesn't.
 */
export function mentionedIn(
	text: string,
	options: readonly Mentionable[],
): string[] {
	const { matches, linked } = findMentions(text, [], options);
	const ids = new Set([...linked, ...matches.map((match) => match.target.id)]);
	return [
		...new Set(options.filter((option) => ids.has(option.id)).map((o) => o.id)),
	];
}

/**
 * The bots whose name is spoken as a whole word, in any case ("hey momo" names Momo, "momos"
 * doesn't). People are never matched: a spoken name in a call is just conversation.
 */
export function spokenMentions(
	text: string,
	options: readonly Mentionable[],
): string[] {
	return options
		.filter(
			(option) =>
				option.kind === "bot" &&
				option.name.trim() &&
				new RegExp(
					`(?<![\\p{L}\\p{N}_])${escapeRegExp(option.name.trim())}(?![\\p{L}\\p{N}_])`,
					"iu",
				).test(text),
		)
		.map((option) => option.id);
}

/** Who a room's composer suggests: its active members other than you, people and bots. */
export function mentionablesOf(
	conversation: IConversation,
	directory: CrewDirectory,
	me?: string | null,
): Mentionable[] {
	if (conversation.kind !== "group") return [];
	return conversation.members
		.filter((member) => !isSelfPrincipal(member.principal_id, me))
		.map((member) => ({
			id: member.principal_id,
			name: directory(member.principal_id).name,
			kind: member.kind,
		}));
}

export interface MentionTarget {
	id: string;
	name: string;
}

interface SourceRange {
	start: number;
	end: number;
}

interface MentionMatch extends SourceRange {
	target: MentionTarget;
}

let mentionParser: ReturnType<typeof createSlateEditor> | undefined;

function parseMentionMarkdown(text: string): Root {
	mentionParser ??= createSlateEditor({
		plugins: [MarkdownPlugin],
		nodeId: false,
	});
	return markdownToAstProcessor(mentionParser, text, {
		remarkPlugins: [
			remarkGfm,
			function () {
				return remarkMath.call(this, { singleDollarTextMath: false });
			},
		],
	});
}

/** Source positions preserve formatting while keeping code and links out of name matching. */
function mentionSource(tree: Root) {
	const prose: (SourceRange & { value: string })[] = [];
	const protectedRanges: SourceRange[] = [];
	const linked = new Set<string>();
	const protectedTypes = new Set([
		"code",
		"inlineCode",
		"html",
		"link",
		"linkReference",
		"image",
		"imageReference",
		"definition",
		"math",
		"inlineMath",
	]);
	const visit = (node: Nodes) => {
		const start = node.position?.start.offset;
		const end = node.position?.end.offset;
		if (node.type === "link" && node.url.startsWith("mention://")) {
			try {
				linked.add(decodeURIComponent(node.url.slice("mention://".length)));
			} catch {
				// A malformed link remains as written and cannot identify a principal.
			}
		}
		if (protectedTypes.has(node.type)) {
			if (start !== undefined && end !== undefined)
				protectedRanges.push({ start, end });
			return;
		}
		if (node.type === "text" && start !== undefined && end !== undefined)
			prose.push({ start, end, value: node.value });
		if ("children" in node) node.children.forEach(visit);
	};
	visit(tree);
	return { prose, protectedRanges, linked };
}

function findMentions(
	text: string,
	explicit: readonly MentionTarget[],
	candidates: readonly MentionTarget[],
) {
	const matches: MentionMatch[] = [];
	const linked = new Set<string>();
	if (
		explicit.length === 0 &&
		!text.includes("@") &&
		!text.includes("mention://")
	)
		return { matches, linked, blockStart: false };
	const tree = parseMentionMarkdown(text);
	const source = mentionSource(tree);
	const names = new Map<string, Map<string, MentionTarget>>();
	const explicitNames = new Map<string, Map<string, MentionTarget>>();
	for (const [targets, map] of [
		[[...candidates, ...explicit], names],
		[explicit, explicitNames],
	] as const) {
		for (const target of targets) {
			const name = target.name.trim();
			if (!target.id || !name || /[\r\n]/.test(name)) continue;
			const key = name.toLowerCase();
			let named = map.get(key);
			if (!named) {
				named = new Map();
				map.set(key, named);
			}
			named.set(target.id, { ...target, name });
		}
	}
	const orderedNames = [...names.keys()].sort((a, b) => b.length - a.length);
	if (orderedNames.length > 0) {
		const pattern = new RegExp(
			`@(${orderedNames.map(escapeRegExp).join("|")})`,
			"giu",
		);
		for (const match of text.matchAll(pattern)) {
			const start = match.index;
			const end = start + match[0].length;
			const firstIndex = source.prose.findIndex(
				(range) => range.start <= start && start < range.end,
			);
			const first = source.prose[firstIndex];
			if (
				!first ||
				(start > first.start &&
					/[\p{L}\p{N}\p{M}_@/\\]$/u.test(text.slice(first.start, start)))
			)
				continue;
			const before = source.prose[firstIndex - 1];
			if (
				start === first.start &&
				before &&
				/^[*_~]+$/.test(text.slice(before.end, start)) &&
				/[\p{L}\p{N}\p{M}_@/\\]$/u.test(before.value)
			)
				continue;
			const lastIndex = source.prose.findIndex(
				(range) => range.start < end && end <= range.end,
			);
			const last = source.prose[lastIndex];
			if (end !== last?.end && /^[\p{L}\p{N}\p{M}_/-]/u.test(text.slice(end)))
				continue;
			const after = source.prose[lastIndex + 1];
			if (
				end === last?.end &&
				after &&
				/^[*_~]+$/.test(text.slice(end, after.start)) &&
				/^[\p{L}\p{N}\p{M}_/-]/u.test(after.value)
			)
				continue;
			if (
				source.protectedRanges.some(
					(range) => range.start < end && range.end > start,
				)
			)
				continue;
			const name = match[1].toLowerCase();
			const targets = explicitNames.get(name) ?? names.get(name);
			// A name shared by several members does not identify which one was addressed.
			if (targets?.size !== 1) continue;
			const target = targets.values().next().value;
			if (target) matches.push({ start, end, target });
		}
	}
	return {
		matches,
		linked: source.linked,
		blockStart: tree.children[0]?.type !== "paragraph",
	};
}

function mentionLink({ id, name }: MentionTarget): string {
	const label = name.replace(/[\\`*_[\]<>&~{}$]/g, "\\$&");
	const encoded = encodeURIComponent(id).replace(
		/[()]/g,
		(char) => `%${char.charCodeAt(0).toString(16).toUpperCase()}`,
	);
	return `[@${label}](mention://${encoded})`;
}

/**
 * Turns `@Name` into a link the chat renderer shows as a highlighted name; a mentioned principal
 * the text doesn't spell out leads the first line instead. Known members without explicit
 * metadata are linked only where the text names them unambiguously.
 */
export function inlineMentions(
	text: string,
	targets: readonly MentionTarget[],
	candidates: readonly MentionTarget[] = [],
): string {
	if (targets.length === 0 && candidates.length === 0) return text;
	const { matches, linked, blockStart } = findMentions(
		text,
		targets,
		candidates,
	);
	let body = "";
	let offset = 0;
	for (const match of matches) {
		body += text.slice(offset, match.start) + mentionLink(match.target);
		offset = match.end;
		linked.add(match.target.id);
	}
	body += text.slice(offset);
	const missing: string[] = [];
	for (const target of targets) {
		if (
			!target.id ||
			!target.name.trim() ||
			/[\r\n]/.test(target.name) ||
			linked.has(target.id)
		)
			continue;
		missing.push(mentionLink({ ...target, name: target.name.trim() }));
		linked.add(target.id);
	}
	if (missing.length === 0) return body;
	const prefix = missing.join(" ");
	return body ? `${prefix}${blockStart ? "\n\n" : " "}${body}` : prefix;
}

/** Direct calls address their sole bot; a group call requires a spoken bot name. */
export function callMentions(
	text: string,
	options: readonly Mentionable[],
	kind: IConversation["kind"],
): string[] {
	const bots = options.filter((option) => option.kind === "bot");
	if (kind === "direct" && bots.length === 1 && text.trim())
		return [bots[0].id];
	return spokenMentions(text, options);
}
