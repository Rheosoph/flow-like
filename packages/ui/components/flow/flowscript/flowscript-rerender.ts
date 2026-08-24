/**
 * Cursor/viewport reseat + reload deferral for live FlowScript re-renders
 * (todo/flowscript-collab.md rule 1): a CLEAN buffer may be re-rendered when
 * the board changes remotely, but the cursor, selection and scroll position
 * must survive the swap (resolved anchor-relative against the new render),
 * and the reload must never land mid-composition, mid-typing or under an
 * open editor widget — it is deferred, not dropped.
 */

import {
	type FlowScriptAnchorIndex,
	anchorAtOrAbove,
} from "./flowscript-anchors";

/* ── Seat capture / resolve (pure) ─────────────────────────────────────── */

/** How far above a line the owning anchor may sit (covers long preambles). */
const SEAT_MAX_ANCHOR_DISTANCE = 5000;

export interface FlowScriptSeatPoint {
	/** Owning anchor; absent for lines with no anchor above (deep preamble). */
	anchorId?: string;
	/** Lines below the owning anchor's line. */
	dLine: number;
	column: number;
	/** Absolute 1-based line in the OLD text (last-resort fallback). */
	line: number;
}

export interface FlowScriptSeat {
	cursor: FlowScriptSeatPoint;
	/** The selection's other end; absent when the selection is empty. */
	selectionStart?: FlowScriptSeatPoint;
	scroll?: {
		anchorId?: string;
		dLine: number;
		line: number;
		/** Pixels the viewport top sits below the top of `line`. */
		offsetPx: number;
	};
}

function capturePoint(
	index: FlowScriptAnchorIndex,
	line: number,
	column: number,
): FlowScriptSeatPoint {
	const anchor = anchorAtOrAbove(index, line, SEAT_MAX_ANCHOR_DISTANCE);
	return {
		anchorId: anchor?.id,
		dLine: anchor ? line - anchor.line : 0,
		column,
		line,
	};
}

export function captureFlowScriptSeat(
	index: FlowScriptAnchorIndex,
	state: {
		position: { lineNumber: number; column: number };
		/** Pass only when a non-empty selection exists. */
		selectionStart?: { lineNumber: number; column: number };
		firstVisibleLine?: number;
		firstVisibleLineOffsetPx?: number;
	},
): FlowScriptSeat {
	const seat: FlowScriptSeat = {
		cursor: capturePoint(
			index,
			state.position.lineNumber,
			state.position.column,
		),
	};
	if (state.selectionStart) {
		seat.selectionStart = capturePoint(
			index,
			state.selectionStart.lineNumber,
			state.selectionStart.column,
		);
	}
	if (typeof state.firstVisibleLine === "number") {
		const point = capturePoint(index, state.firstVisibleLine, 1);
		seat.scroll = {
			anchorId: point.anchorId,
			dLine: point.dLine,
			line: state.firstVisibleLine,
			offsetPx: state.firstVisibleLineOffsetPx ?? 0,
		};
	}
	return seat;
}

const clampLine = (line: number, maxLine: number) =>
	Math.min(Math.max(line, 1), maxLine);

/**
 * Resolve one captured point against the NEW render:
 * 1. the owning anchor's new line + dLine when the anchor survived;
 * 2. else the nearest OLD anchor (by old-line distance, ties prefer above)
 *    that survives, keeping the point's offset to that anchor;
 * 3. else the same absolute line number.
 */
export function resolveFlowScriptSeatPoint(
	point: Pick<FlowScriptSeatPoint, "anchorId" | "dLine" | "line" | "column">,
	oldIndex: FlowScriptAnchorIndex,
	newIndex: FlowScriptAnchorIndex,
	maxLine: number,
): { lineNumber: number; column: number } {
	if (point.anchorId) {
		const anchorLine = newIndex.firstLineById.get(point.anchorId);
		if (anchorLine) {
			return {
				lineNumber: clampLine(anchorLine + point.dLine, maxLine),
				column: point.column,
			};
		}
	}
	const candidates = [...oldIndex.anchors].sort((a, b) => {
		const da = Math.abs(a.line - point.line);
		const db = Math.abs(b.line - point.line);
		if (da !== db) return da - db;
		// Prefer the anchor above the point (smaller line) on equal distance.
		return a.line - b.line;
	});
	for (const candidate of candidates) {
		const anchorLine = newIndex.firstLineById.get(candidate.id);
		if (!anchorLine) continue;
		return {
			lineNumber: clampLine(
				anchorLine + (point.line - candidate.line),
				maxLine,
			),
			column: point.column,
		};
	}
	return { lineNumber: clampLine(point.line, maxLine), column: point.column };
}

export interface ResolvedFlowScriptSeat {
	position: { lineNumber: number; column: number };
	selectionStart?: { lineNumber: number; column: number };
	scroll?: { lineNumber: number; offsetPx: number };
}

export function resolveFlowScriptSeat(
	seat: FlowScriptSeat,
	oldIndex: FlowScriptAnchorIndex,
	newIndex: FlowScriptAnchorIndex,
	maxLine: number,
): ResolvedFlowScriptSeat {
	const resolved: ResolvedFlowScriptSeat = {
		position: resolveFlowScriptSeatPoint(
			seat.cursor,
			oldIndex,
			newIndex,
			maxLine,
		),
	};
	if (seat.selectionStart) {
		resolved.selectionStart = resolveFlowScriptSeatPoint(
			seat.selectionStart,
			oldIndex,
			newIndex,
			maxLine,
		);
	}
	if (seat.scroll) {
		const scrollPoint = resolveFlowScriptSeatPoint(
			{ ...seat.scroll, column: 1 },
			oldIndex,
			newIndex,
			maxLine,
		);
		resolved.scroll = {
			lineNumber: scrollPoint.lineNumber,
			offsetPx: seat.scroll.offsetPx,
		};
	}
	return resolved;
}

/* ── Reload deferral (pure predicate + timer runner) ───────────────────── */

/** A key/content input within this window marks the user as actively typing. */
export const FLOWSCRIPT_TYPING_QUIESCENCE_MS = 2000;
/** How often a deferred reload re-checks for quiescence. */
export const FLOWSCRIPT_RELOAD_CHECK_INTERVAL_MS = 500;

export interface FlowScriptReloadGuardInput {
	now: number;
	editorFocused: boolean;
	/** Timestamp of the last keystroke/content input; undefined = never. */
	lastInputAt?: number;
	/** IME composition in progress. */
	composing: boolean;
	/** A suggest/find/rename widget is visibly open. */
	widgetOpen: boolean;
	quiescenceMs?: number;
}

/** True while an automatic re-render must wait (defer, never drop). */
export function shouldDeferFlowScriptReload(
	input: FlowScriptReloadGuardInput,
): boolean {
	if (input.composing) return true;
	if (input.widgetOpen) return true;
	const quiescence = input.quiescenceMs ?? FLOWSCRIPT_TYPING_QUIESCENCE_MS;
	return (
		input.editorFocused &&
		typeof input.lastInputAt === "number" &&
		input.now - input.lastInputAt < quiescence
	);
}

export interface FlowScriptDeferredReloadRunner {
	/** Run now when unblocked, otherwise remember and retry on the timer. */
	request: () => void;
	/** Opportunistic flush (e.g. on editor blur). */
	poke: () => void;
	pending: () => boolean;
	dispose: () => void;
}

/**
 * Timer-driven deferral: a blocked request stays pending and re-checks every
 * `checkIntervalMs` until the guard clears; `poke()` flushes early (blur).
 * Multiple requests while pending coalesce into one run.
 */
export function createDeferredReloadRunner(options: {
	run: () => void;
	isBlocked: () => boolean;
	checkIntervalMs?: number;
	schedule?: (cb: () => void, ms: number) => unknown;
	cancel?: (handle: unknown) => void;
}): FlowScriptDeferredReloadRunner {
	const interval =
		options.checkIntervalMs ?? FLOWSCRIPT_RELOAD_CHECK_INTERVAL_MS;
	const schedule =
		options.schedule ??
		((cb: () => void, ms: number) => setTimeout(cb, ms) as unknown);
	const cancel =
		options.cancel ??
		((handle: unknown) =>
			clearTimeout(handle as ReturnType<typeof setTimeout>));

	let pending = false;
	let timer: unknown | null = null;
	let disposed = false;

	const clearTimer = () => {
		if (timer !== null) {
			cancel(timer);
			timer = null;
		}
	};

	const runNow = () => {
		pending = false;
		clearTimer();
		options.run();
	};

	const tick = () => {
		timer = null;
		if (disposed || !pending) return;
		if (options.isBlocked()) {
			timer = schedule(tick, interval);
			return;
		}
		runNow();
	};

	return {
		request: () => {
			if (disposed) return;
			if (!options.isBlocked()) {
				runNow();
				return;
			}
			pending = true;
			if (timer === null) timer = schedule(tick, interval);
		},
		poke: () => {
			if (disposed || !pending) return;
			if (!options.isBlocked()) runNow();
		},
		pending: () => pending,
		dispose: () => {
			disposed = true;
			pending = false;
			clearTimer();
		},
	};
}

/* ── Editor widget probe (DOM) ─────────────────────────────────────────── */

/**
 * Widgets whose open state we can observe from the editor DOM. Suggest, find
 * and parameter-hint widgets flag themselves with `.visible`; the rename box
 * has no state class, so visibility is checked geometrically. NOT guarded
 * (no accessible signal): the context menu and hover widgets — hover is
 * transient and holds no user state, so a reload under it is acceptable.
 */
const EDITOR_WIDGET_SELECTORS = [
	".suggest-widget.visible",
	".find-widget.visible",
	".parameter-hints-widget.visible",
	".rename-box",
] as const;

function isElementVisible(element: Element): boolean {
	const el = element as HTMLElement;
	return Boolean(
		el.offsetWidth || el.offsetHeight || el.getClientRects().length,
	);
}

export function isMonacoWidgetOpen(
	root: Pick<Element, "querySelectorAll"> | null | undefined,
): boolean {
	if (!root) return false;
	for (const selector of EDITOR_WIDGET_SELECTORS) {
		const elements = root.querySelectorAll(selector);
		for (let i = 0; i < elements.length; i++) {
			if (isElementVisible(elements[i])) return true;
		}
	}
	return false;
}
