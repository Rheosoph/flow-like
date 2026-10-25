/**
 * Live drafts of generated A2UI trees. Components are extracted while the model is still writing:
 * from `<ui_draft>` frames (the backend scans streamed `emit_ui` arguments) and from a JSON block in
 * streamed assistant text (the Bits UI specialist answers with a ```json fence instead of a tool
 * call). The scanner mirrors `UiDraftToolCallTracker` in `flow/copilot/stream.rs`: a cursor-based,
 * string/escape-aware brace scanner that parses each element of the top-level `components` array
 * exactly once, when its closing brace arrives. Drafts are display-only — the completed run's
 * components stay authoritative.
 */

import type { UiDraftFrame } from "./copilot-stream-parser";

export const MAX_UI_DRAFT_COMPONENTS = 300;
export const MAX_UI_DRAFT_TEXT_CHARS = 2 * 1024 * 1024;
export const UI_DRAFT_PUBLISH_INTERVAL_MS = 150;
const MAX_UI_DRAFT_SOURCES = 8;

export interface UiDraftScan {
	components: unknown[];
	rootComponentId?: string;
	canvasSettings?: Record<string, unknown>;
}

export interface UiDraftSnapshot {
	readonly components: readonly unknown[];
	readonly rootComponentId?: string;
	readonly canvasSettings?: Record<string, unknown>;
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
	Boolean(value) && typeof value === "object" && !Array.isArray(value);

const parseSlice = (text: string, start: number, end: number) => {
	try {
		return JSON.parse(text.slice(start, end + 1)) as unknown;
	} catch {
		return undefined;
	}
};

/** Flat (`type`) or nested (`component`) authored component; anything else is not a UI tree. */
const isAuthoredComponent = (value: unknown) =>
	isRecord(value) &&
	typeof value.id === "string" &&
	(typeof value.type === "string" || isRecord(value.component));

const isJsonWhitespace = (char: string) =>
	char === " " || char === "\n" || char === "\r" || char === "\t";

const hasDraftContent = (scan: UiDraftScan) =>
	scan.components.length > 0 ||
	scan.rootComponentId !== undefined ||
	scan.canvasSettings !== undefined;

/** Merge `scan` into `draft`, keeping at most `MAX_UI_DRAFT_COMPONENTS` components. */
const appendScan = (draft: UiDraftScan, scan: UiDraftScan) => {
	const room = Math.max(0, MAX_UI_DRAFT_COMPONENTS - draft.components.length);
	draft.components.push(...scan.components.slice(0, room));
	if (scan.rootComponentId !== undefined)
		draft.rootComponentId = scan.rootComponentId;
	if (scan.canvasSettings) draft.canvasSettings = scan.canvasSettings;
};

interface ComponentScanner {
	/** Scan the growing `text` from the internal cursor. */
	scan(text: string): UiDraftScan;
	/** True once the root value closed, or the text at the start is not a JSON object/array. */
	done(): boolean;
	/** Offset just past the last scanned character. */
	end(): number;
}

/**
 * Scans one JSON value starting at `start`: an object holding `components` (plus
 * `rootComponentId`/`canvasSettings`), or a bare array of components.
 */
function createComponentScanner(start: number): ComponentScanner {
	let cursor = start;
	let depth = 0;
	let root: "pending" | "object" | "array" | "done" = "pending";
	let inString = false;
	let escaped = false;
	let stringStart = 0;
	let expectingKey = false;
	let currentKey: string | undefined;
	let componentsDepth: number | undefined;
	let elementStart: number | undefined;
	let canvasSettingsStart: number | undefined;

	const closeString = (text: string, end: number, scan: UiDraftScan) => {
		if (depth !== 1 || root !== "object") return;
		const value = parseSlice(text, stringStart, end);
		if (typeof value !== "string") return;
		if (expectingKey) currentKey = value;
		else if (currentKey === "rootComponentId") scan.rootComponentId = value;
	};

	const stepInString = (
		char: string,
		text: string,
		index: number,
		scan: UiDraftScan,
	) => {
		if (escaped) escaped = false;
		else if (char === "\\") escaped = true;
		else if (char === '"') {
			inString = false;
			closeString(text, index, scan);
		}
	};

	const openRoot = (char: string) => {
		if (char === "{") {
			root = "object";
			depth = 1;
			expectingKey = true;
		} else if (char === "[") {
			root = "array";
			depth = 1;
			componentsDepth = 1;
		} else if (!isJsonWhitespace(char)) {
			root = "done";
		}
	};

	const openTopLevelValue = (char: string, index: number) => {
		if (currentKey === "components" && char === "[") componentsDepth = 2;
		else if (currentKey === "canvasSettings" && char === "{")
			canvasSettingsStart = index;
	};

	const openContainer = (char: string, index: number) => {
		if (depth === 1 && root === "object") openTopLevelValue(char, index);
		else if (char === "{" && componentsDepth === depth) elementStart = index;
		depth += 1;
	};

	const takeComponent = (text: string, end: number, scan: UiDraftScan) => {
		if (elementStart === undefined) return;
		const component = parseSlice(text, elementStart, end);
		elementStart = undefined;
		if (isRecord(component)) scan.components.push(component);
	};

	const closeTopLevelValue = (
		char: string,
		text: string,
		end: number,
		scan: UiDraftScan,
	) => {
		if (char === "]") componentsDepth = undefined;
		if (canvasSettingsStart === undefined) return;
		const settings = parseSlice(text, canvasSettingsStart, end);
		canvasSettingsStart = undefined;
		if (isRecord(settings)) scan.canvasSettings = settings;
	};

	const closeContainer = (
		char: string,
		text: string,
		index: number,
		scan: UiDraftScan,
	) => {
		depth -= 1;
		if (depth <= 0) root = "done";
		else if (char === "}" && componentsDepth === depth)
			takeComponent(text, index, scan);
		else if (depth === 1 && root === "object")
			closeTopLevelValue(char, text, index, scan);
	};

	const separate = (char: string) => {
		if (depth !== 1) return;
		expectingKey = char === ",";
		if (expectingKey) currentKey = undefined;
	};

	const stepStructure = (
		char: string,
		text: string,
		index: number,
		scan: UiDraftScan,
	) => {
		if (char === '"') {
			inString = true;
			stringStart = index;
		} else if (char === "{" || char === "[") openContainer(char, index);
		else if (char === "}" || char === "]")
			closeContainer(char, text, index, scan);
		else if (char === ":" || char === ",") separate(char);
	};

	const step = (text: string, index: number, scan: UiDraftScan) => {
		const char = text[index];
		if (inString) stepInString(char, text, index, scan);
		else if (root === "pending") openRoot(char);
		else stepStructure(char, text, index, scan);
	};

	return {
		done: () => root === "done",
		end: () => cursor,
		scan(text) {
			const scan: UiDraftScan = { components: [] };
			while (cursor < text.length && root !== "done") {
				cursor += 1;
				step(text, cursor - 1, scan);
			}
			return scan;
		},
	};
}

type CandidateSearch =
	| { kind: "found"; index: number }
	| { kind: "wait"; from: number }
	| { kind: "none" };

type CandidateStep = CandidateSearch | { kind: "skip"; to: number };

const firstNonWhitespace = (text: string, from: number) => {
	let index = from;
	while (index < text.length && isJsonWhitespace(text[index])) index += 1;
	return index;
};

/** Earliest `{` or fence opener at or after `from`, or -1. */
const nextMarker = (text: string, from: number) => {
	const brace = text.indexOf("{", from);
	const fence = text.indexOf("```", from);
	if (brace < 0 || fence < 0) return Math.max(brace, fence);
	return Math.min(brace, fence);
};

/**
 * A ```json (or bare ```) fence yields the value right after its opener line. Code in another
 * language (examples, FlowScript) never holds the component tree, so it is skipped whole.
 */
const fenceCandidate = (text: string, index: number): CandidateStep => {
	const lineEnd = text.indexOf("\n", index + 3);
	if (lineEnd < 0) return { kind: "wait", from: index };
	const language = text
		.slice(index + 3, lineEnd)
		.trim()
		.toLowerCase();
	if (language !== "json" && language !== "") {
		const fenceEnd = text.indexOf("```", lineEnd + 1);
		return fenceEnd < 0
			? { kind: "wait", from: index }
			: { kind: "skip", to: fenceEnd + 3 };
	}
	const valueStart = firstNonWhitespace(text, lineEnd + 1);
	if (valueStart >= text.length) return { kind: "wait", from: index };
	const opensValue = text[valueStart] === "{" || text[valueStart] === "[";
	return opensValue
		? { kind: "found", index: valueStart }
		: { kind: "skip", to: lineEnd + 1 };
};

/** A bare object counts only when it opens with a key (`{"`), not prose braces. */
const objectCandidate = (text: string, index: number): CandidateStep => {
	const next = firstNonWhitespace(text, index + 1);
	if (next >= text.length) return { kind: "wait", from: index };
	return text[next] === '"'
		? { kind: "found", index }
		: { kind: "skip", to: index + 1 };
};

/**
 * Next place a component tree may start. `wait` means the tail is still ambiguous (an unfinished
 * fence line or a lone `{`), so the search resumes there once more text arrives.
 */
const nextCandidate = (text: string, from: number): CandidateSearch => {
	let index = from;
	while (index < text.length) {
		const marker = nextMarker(text, index);
		if (marker < 0) break;
		const step = text.startsWith("```", marker)
			? fenceCandidate(text, marker)
			: objectCandidate(text, marker);
		if (step.kind !== "skip") return step;
		index = step.to;
	}
	return { kind: "none" };
};

export interface UiDraftTextExtractor {
	/** Feed the next assistant-text delta; returns what completed since the previous push. */
	push(text: string): UiDraftScan;
}

/**
 * Finds the component tree in streamed assistant text. The first JSON value that yields authored
 * components wins; other values (inline examples, command lists) are skipped.
 */
export function createUiDraftTextExtractor(): UiDraftTextExtractor {
	let buffer = "";
	let searchFrom = 0;
	let scanner: ComponentScanner | undefined;
	let emitted = 0;
	let finished = false;

	const collect = (scan: UiDraftScan, result: UiDraftScan) => {
		const components = scan.components
			.filter(isAuthoredComponent)
			.slice(0, MAX_UI_DRAFT_COMPONENTS - emitted);
		emitted += components.length;
		appendScan(result, { ...scan, components });
		if (emitted >= MAX_UI_DRAFT_COMPONENTS) finished = true;
	};

	const startScanner = () => {
		const candidate = nextCandidate(buffer, searchFrom);
		if (candidate.kind === "found")
			return createComponentScanner(candidate.index);
		// After `none`, a trailing "`" or "``" may still grow into a fence opener.
		searchFrom =
			candidate.kind === "wait"
				? candidate.from
				: Math.max(searchFrom, buffer.length - 2);
		return undefined;
	};

	/** Scan as far as the buffer allows; true while another candidate may follow. */
	const advance = (result: UiDraftScan) => {
		scanner ??= startScanner();
		if (!scanner) return false;
		collect(scanner.scan(buffer), result);
		if (!scanner.done()) return false;
		searchFrom = scanner.end();
		scanner = undefined;
		if (emitted > 0) finished = true;
		return true;
	};

	return {
		push(text) {
			const result: UiDraftScan = { components: [] };
			if (finished || !text) return result;
			if (buffer.length + text.length > MAX_UI_DRAFT_TEXT_CHARS)
				finished = true;
			else buffer += text;
			let more = true;
			while (more && !finished) more = advance(result);
			if (finished) buffer = "";
			return result;
		},
	};
}

export interface UiDraftCollector {
	/** Merge a `<ui_draft>` frame; true when the visible draft changed. */
	pushFrame(frame: UiDraftFrame): boolean;
	/** Scan streamed assistant text; true when the visible draft changed. */
	pushText(text: string): boolean;
	/** The most recently updated draft (latest tool call or the text block), if any. */
	snapshot(): UiDraftSnapshot | null;
}

/**
 * Accumulates drafts per source: each `emit_ui` call (a retry starts a fresh draft) and the
 * assistant text. The most recently updated source is the visible draft.
 */
export function createUiDraftCollector(): UiDraftCollector {
	const sources = new Map<string, UiDraftScan>();
	const text = createUiDraftTextExtractor();
	let latest: string | undefined;

	const remember = (key: string, draft: UiDraftScan) => {
		sources.delete(key);
		sources.set(key, draft);
		const oldest = sources.keys().next().value;
		if (sources.size > MAX_UI_DRAFT_SOURCES && oldest !== undefined)
			sources.delete(oldest);
		latest = key;
	};

	const merge = (key: string, scan: UiDraftScan) => {
		const incoming = { ...scan, components: scan.components.filter(isRecord) };
		if (!hasDraftContent(incoming)) return false;
		const draft = sources.get(key) ?? { components: [] };
		appendScan(draft, incoming);
		remember(key, draft);
		return true;
	};

	return {
		pushFrame: (frame) => merge(`tool:${frame.toolCallId}`, frame),
		pushText: (delta) => merge("text", text.push(delta)),
		snapshot: () => (latest ? (sources.get(latest) ?? null) : null),
	};
}

export interface ThrottledUiDraft {
	pushFrame(frame: UiDraftFrame): void;
	pushText(text: string): void;
	/** Drop a pending publish and ignore everything after; call when the run ends. */
	cancel(): void;
}

/** Collects drafts and hands the visible one to `publish` at most once per interval. */
export function createThrottledUiDraft(
	publish: (snapshot: UiDraftSnapshot) => void,
	intervalMs = UI_DRAFT_PUBLISH_INTERVAL_MS,
): ThrottledUiDraft {
	const collector = createUiDraftCollector();
	let timer: ReturnType<typeof setTimeout> | undefined;
	let cancelled = false;
	const flush = () => {
		timer = undefined;
		const snapshot = collector.snapshot();
		if (snapshot && !cancelled) publish(snapshot);
	};
	const schedule = (changed: boolean) => {
		if (!changed || timer || cancelled) return;
		timer = setTimeout(flush, intervalMs);
	};
	return {
		pushFrame: (frame) => schedule(collector.pushFrame(frame)),
		pushText: (delta) => schedule(collector.pushText(delta)),
		cancel: () => {
			cancelled = true;
			if (timer) clearTimeout(timer);
			timer = undefined;
		},
	};
}
