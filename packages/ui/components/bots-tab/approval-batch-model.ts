import { requiresFreshToolApproval } from "../../lib/frontend-tool-approval-scope";
import type { IBotApprovalItemDecision } from "../../state/backend-state/bot-state";
import type { ApprovalDecision } from "./approval-entry";
import type {
	ConversationApproval,
	ConversationApprovalItem,
} from "./fold-entries";

/** Above this many calls a waiting card shows its groups only, until asked. */
export const FEW_CALLS = 5;
/** Groups a closed list shows. */
export const CLOSED_GROUPS = 4;
/** How long a card waits before the host gives up (`APPROVAL_WAIT_DAYS` on the server). */
export const APPROVAL_WAIT_MS = 7 * 24 * 60 * 60 * 1000;

export const TURN_NOT_WAITING = "BOT_TURN_NOT_WAITING";
export const APPROVAL_EXPIRED = "BOT_APPROVAL_EXPIRED";

/** A summary without the core's "The bot wants to" and its closing full stop. */
function bare(summary: string): string {
	return summary
		.trim()
		.replace(/^the bot wants to\s+/i, "")
		.replace(/[.!]+$/, "");
}

/** The summary as the end of "{{name}} wants to …". */
export function callPhrase(summary: string): string {
	const text = bare(summary);
	return /^[A-Z][a-z]/.test(text)
		? text.charAt(0).toLowerCase() + text.slice(1)
		: text;
}

/** The summary as a line of its own, sentence-cased. */
export function callLine(summary: string): string {
	const text = bare(summary);
	return text.charAt(0).toUpperCase() + text.slice(1);
}

/**
 * How a call's row reads. `start` may lose its end to an ellipsis, `end` always stays whole, so
 * a long row is shortened in the middle. `full` is the whole line, for the details, the tooltip
 * and screen readers.
 */
export interface CallText {
	full: string;
	start: string;
	end: string;
}

const METHOD = /^(GET|HEAD|POST|PUT|PATCH|DELETE|OPTIONS)$/;
const SITE_AND_PATH =
	/^(?:https?:\/\/)?([a-z0-9-]+(?:\.[a-z0-9-]+)+(?::\d+)?)(\/\S*?)?([.,;:]*)$/i;
/** Where the differing part of a tail may start: a path, query or word boundary. */
const BOUNDARY = /[/?&# ]/;
/** The longest `end` a row keeps whole; a longer one loses its middle. */
const END_CHARS = 36;
const END_HEAD_CHARS = 12;

function sharedTokens(lines: readonly (readonly string[])[]): number {
	const [first, ...rest] = lines;
	let shared = 0;
	while (
		shared < first.length &&
		rest.every((tokens) => tokens[shared] === first[shared])
	)
		shared++;
	return shared;
}

function sharedChars(texts: readonly string[]): number {
	const [first, ...rest] = texts;
	let shared = 0;
	while (
		shared < first.length &&
		rest.every((text) => text[shared] === first[shared])
	)
		shared++;
	return shared;
}

function siteOf(token: string | undefined): string | undefined {
	return token ? SITE_AND_PATH.exec(token)?.[1]?.toLowerCase() : undefined;
}

/** The token without the site the group's header already names: `/path?query`. */
function withoutSite(token: string): string {
	const [, , path, punctuation] = SITE_AND_PATH.exec(token) ?? [];
	return `${path || "/"}${punctuation ?? ""}`;
}

/** A tail that now starts with a plain word reads as a line; a site, path or name stays. */
function upperFirst(text: string): string {
	return /^[a-z]+(?:\s|$)/.test(text)
		? text.charAt(0).toUpperCase() + text.slice(1)
		: text;
}

/** Splits a tail where the calls of its group start to differ, at the boundary before it. */
function splitTail(full: string, tail: string, differsAt: number): CallText {
	let cut = Math.min(differsAt, tail.length - 1);
	while (cut > 0 && !BOUNDARY.test(tail[cut])) cut--;
	const end = tail.slice(cut);
	return {
		full,
		start: tail.slice(0, cut),
		end:
			end.length > END_CHARS
				? `${end.slice(0, END_HEAD_CHARS)}…${end.slice(end.length - (END_CHARS - END_HEAD_CHARS - 1))}`
				: end,
	};
}

/**
 * The rows of one group, each by what tells it apart: the words every call of the group starts
 * with are dropped, and so is the site the header names, keeping the method, e.g. "GET
 * /alerts?p=1". A lone call, or calls with nothing in common, keep their whole line.
 */
export function callTexts(
	items: readonly ConversationApprovalItem[],
): (item: ConversationApprovalItem) => CallText {
	const texts = tailsOf(items);
	return (item) => {
		const full = callLine(item.summary || item.tool);
		return texts?.get(item.argsHash) ?? { full, start: full, end: "" };
	};
}

function tailsOf(
	items: readonly ConversationApprovalItem[],
): Map<string, CallText> | undefined {
	if (items.length < 2) return undefined;
	const lines = items.map((item) => callLine(item.summary || item.tool));
	const tokens = lines.map((line) => line.split(" "));
	const shared = sharedTokens(tokens);
	if (shared === 0 || tokens.some((line) => line.length <= shared))
		return undefined;
	const rests = tokens.map((line) => line.slice(shared));
	const sites = rests.map((rest) => siteOf(rest[0]));
	const allSites = sites.every(Boolean);
	const sameSite = allSites && sites.every((site) => site === sites[0]);
	const method = allSites
		? tokens[0].slice(0, shared).find((token) => METHOD.test(token))
		: undefined;
	const tails = rests.map(([first, ...more]) =>
		allSites
			? [method, sameSite ? withoutSite(first) : first, ...more]
					.filter(Boolean)
					.join(" ")
			: upperFirst([first, ...more].join(" ")),
	);
	const differsAt = sharedChars(tails);
	return new Map(
		items.map((item, index) => [
			item.argsHash,
			splitTail(lines[index], tails[index], differsAt),
		]),
	);
}

/** Every call a card asks about; a card from before batches is its one call. */
export function approvalItems(
	approval: ConversationApproval,
): ConversationApprovalItem[] {
	if (approval.items && approval.items.length > 0) return approval.items;
	return [
		{
			argsHash: approval.id,
			tool: approval.tool,
			summary: approval.summary,
			scopes: approval.scopes,
			calls: 1,
			afterWebContent: approval.afterWebContent === true,
			...(approval.secret ? { secret: approval.secret } : {}),
		},
	];
}

export function isBatch(approval: ConversationApproval): boolean {
	return (approval.items?.length ?? 0) > 1;
}

export function callCount(items: readonly ConversationApprovalItem[]): number {
	return items.reduce((sum, item) => sum + item.calls, 0);
}

export type AlwaysBlock = "notRememberable" | "webContent";

/** Why "Always" cannot be offered for this call, if it can't. */
export function alwaysBlock(
	item: ConversationApprovalItem,
): AlwaysBlock | undefined {
	if (item.afterWebContent) return "webContent";
	if (
		item.secret ||
		item.scopes.length === 0 ||
		requiresFreshToolApproval(item.tool)
	)
		return "notRememberable";
	return undefined;
}

/** Calls of one tool on one grant target, in the order the model wrote them. */
export interface CallGroup {
	key: string;
	tool: string;
	scopes: string[];
	flagged: boolean;
	items: ConversationApprovalItem[];
	calls: number;
}

export function groupItems(
	items: readonly ConversationApprovalItem[],
): CallGroup[] {
	const groups = new Map<string, CallGroup>();
	for (const item of items) {
		const key = [item.tool, item.scopes.join(","), item.afterWebContent].join(
			"\u0000",
		);
		const group = groups.get(key) ?? {
			key,
			tool: item.tool,
			scopes: item.scopes,
			flagged: item.afterWebContent,
			items: [],
			calls: 0,
		};
		group.items.push(item);
		group.calls += item.calls;
		groups.set(key, group);
	}
	return [...groups.values()];
}

export type ChoiceOf = (
	item: ConversationApprovalItem,
) => ApprovalDecision | undefined;

/** The one answer every call of the group has, or none when they differ. */
export function groupChoice(
	group: CallGroup,
	choiceOf: ChoiceOf,
): ApprovalDecision | undefined {
	const choices = new Set(group.items.map(choiceOf));
	return choices.size === 1 ? [...choices][0] : undefined;
}

export interface Tally {
	allowed: number;
	denied: number;
}

export function tallyOf(
	items: readonly ConversationApprovalItem[],
	choiceOf: ChoiceOf,
): Tally {
	let allowed = 0;
	let denied = 0;
	for (const item of items) {
		const choice = choiceOf(item);
		if (choice === "deny") denied += item.calls;
		else if (choice) allowed += item.calls;
	}
	return { allowed, denied };
}

const DECISION_ORDER: readonly ApprovalDecision[] = ["once", "deny", "always"];

/** The answer most calls got; ties go to the more cautious one. */
function mostCommon(
	items: readonly ConversationApprovalItem[],
	choiceOf: ChoiceOf,
): ApprovalDecision {
	const counts = new Map<ApprovalDecision, number>();
	for (const item of items) {
		const choice = choiceOf(item);
		if (choice) counts.set(choice, (counts.get(choice) ?? 0) + 1);
	}
	return DECISION_ORDER.reduce((best, next) =>
		(counts.get(next) ?? 0) > (counts.get(best) ?? 0) ? next : best,
	);
}

export interface BatchAnswer {
	decision: ApprovalDecision;
	items: IBotApprovalItemDecision[];
}

/** The answer to send: one top-level decision, and only the calls that differ from it. */
export function batchAnswer(
	items: readonly ConversationApprovalItem[],
	choiceOf: ChoiceOf,
	decision: ApprovalDecision = mostCommon(items, choiceOf),
): BatchAnswer {
	return {
		decision,
		items: items.flatMap((item) => {
			const choice = choiceOf(item) ?? decision;
			return choice === decision
				? []
				: [{ args_hash: item.argsHash, decision: choice }];
		}),
	};
}

/** The answer one call of an answered card got. */
export function answeredChoice(approval: ConversationApproval): ChoiceOf {
	return (item) => approval.itemAnswers?.[item.argsHash] ?? approval.answer;
}

export function isPastWait(postedAt: number | undefined, now: number): boolean {
	return postedAt !== undefined && now - postedAt > APPROVAL_WAIT_MS;
}

/** The code an answer was refused with, from the hub or this computer's store. */
export function refusalCode(error: unknown): string | undefined {
	if (typeof error !== "object" || error === null) return undefined;
	const { code, message } = error as { code?: unknown; message?: unknown };
	if (typeof code === "string") return code;
	return typeof message === "string"
		? /^\[([A-Z0-9_]+)[;\]]/.exec(message)?.[1]
		: undefined;
}
