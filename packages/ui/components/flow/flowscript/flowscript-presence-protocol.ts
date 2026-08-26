/**
 * FlowScript presence wire protocol.
 *
 * HARD RULE (todo/flowscript-collab.md §Hard rules #2): presence rides the
 * existing E2E-encrypted WebRTC awareness exchange and carries ONLY ephemeral
 * positional metadata — anchor ids, small numeric offsets, timestamps. It must
 * NEVER carry code text, board content, pin values, or anything reconstructable
 * into content. Enforcement is structural: every string that can appear on the
 * wire either matches {@link WIRE_ANCHOR_ID_PATTERN} or is one of the closed
 * {@link FLOWSCRIPT_ANCHOR_WIRE_KINDS}; every number is finite and clamped into
 * a small bound; unknown keys are dropped by the sanitizers and rejected by
 * {@link wireSafetyViolations}. `flowscript-presence-protocol.test.ts` walks
 * sanitized payloads to prove no field can carry unbounded text.
 *
 * The same sanitizers run on BOTH sides: before publishing a local payload and
 * when reading a peer's state (peers are untrusted).
 */

/** Awareness field carrying a peer's FlowScript editor cursor/selection. */
export const FLOWSCRIPT_CURSOR_FIELD = "flowscriptCursor";
/** Awareness field carrying a peer's soft edit claims (dirty-touched anchors). */
export const FLOWSCRIPT_CLAIMS_FIELD = "flowscriptClaims";
/** Awareness field carrying the node ids of a peer's shared "edit selection" scope. */
export const FLOWSCRIPT_SCOPE_FIELD = "flowscriptScope";

/** Board entity ids (cuid2-style). Anything else is rejected, never truncated. */
export const WIRE_ANCHOR_ID_PATTERN = /^[A-Za-z0-9_-]{10,32}$/;

export const FLOWSCRIPT_ANCHOR_WIRE_KINDS = [
	"node",
	"variable",
	"layer",
] as const;
export type FlowScriptAnchorWireKind =
	(typeof FLOWSCRIPT_ANCHOR_WIRE_KINDS)[number];

/** Max lines a cursor may sit below its owning anchor line. */
export const MAX_WIRE_DLINE = 500;
/** Max 1-based column carried on the wire. */
export const MAX_WIRE_COLUMN = 1000;
/** Max anchors one peer may claim at once. */
export const MAX_CLAIM_ANCHORS = 64;
/** Max node ids a shared scope may carry on the wire. */
export const MAX_SCOPE_NODES = 64;
/** Longest string permitted anywhere in a payload (anchor ids are ≤ 32). */
export const MAX_WIRE_STRING_LENGTH = 32;

export interface FlowScriptWireAnchor {
	/** Board entity id the position is relative to. */
	id: string;
	kind: FlowScriptAnchorWireKind;
}

export interface FlowScriptCursorSelection {
	/** Anchor of the selection's other end; omitted when it shares the cursor's anchor. */
	endAnchorId?: string;
	endDLine: number;
	endColumn: number;
}

export interface FlowScriptCursorPayload {
	anchor: FlowScriptWireAnchor;
	/** Lines below the anchor line (≥ 0, ≤ MAX_WIRE_DLINE). */
	dLine: number;
	/** 1-based column (≥ 1, ≤ MAX_WIRE_COLUMN). */
	column: number;
	/** Present only while a non-empty range is selected. */
	sel?: FlowScriptCursorSelection;
	/** Publisher wall-clock ms — freshness only, never compared across clocks. */
	ts: number;
}

export interface FlowScriptClaimsPayload {
	/** Anchors whose statements differ from the peer's baseline (≤ MAX_CLAIM_ANCHORS). */
	anchorIds: string[];
	ts: number;
}

export interface FlowScriptScopePayload {
	/** Node ids the peer's scoped session is editing (≤ MAX_SCOPE_NODES). */
	nodeIds: string[];
	ts: number;
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function wireAnchorId(value: unknown): string | undefined {
	return typeof value === "string" && WIRE_ANCHOR_ID_PATTERN.test(value)
		? value
		: undefined;
}

function wireKind(value: unknown): FlowScriptAnchorWireKind | undefined {
	return FLOWSCRIPT_ANCHOR_WIRE_KINDS.includes(
		value as FlowScriptAnchorWireKind,
	)
		? (value as FlowScriptAnchorWireKind)
		: undefined;
}

function boundedInt(
	value: unknown,
	min: number,
	max: number,
): number | undefined {
	if (typeof value !== "number" || !Number.isFinite(value)) return undefined;
	return Math.min(Math.max(Math.round(value), min), max);
}

function wireTs(value: unknown): number {
	const ts = boundedInt(value, 0, Number.MAX_SAFE_INTEGER);
	return ts ?? 0;
}

/**
 * Validate/clamp a cursor payload for the wire. Returns a freshly-built object
 * containing only known fields, or `undefined` when the anchor is unusable.
 * Accepts `unknown` so remote peer state goes through the same gate.
 */
export function sanitizeCursorForWire(
	value: unknown,
): FlowScriptCursorPayload | undefined {
	if (!isRecord(value)) return undefined;
	const anchorRaw = value.anchor;
	if (!isRecord(anchorRaw)) return undefined;
	const id = wireAnchorId(anchorRaw.id);
	const kind = wireKind(anchorRaw.kind);
	if (!id || !kind) return undefined;
	const dLine = boundedInt(value.dLine, 0, MAX_WIRE_DLINE);
	const column = boundedInt(value.column, 1, MAX_WIRE_COLUMN);
	if (typeof dLine === "undefined" || typeof column === "undefined")
		return undefined;

	const payload: FlowScriptCursorPayload = {
		anchor: { id, kind },
		dLine,
		column,
		ts: wireTs(value.ts),
	};

	const selRaw = value.sel;
	if (isRecord(selRaw)) {
		const endDLine = boundedInt(selRaw.endDLine, 0, MAX_WIRE_DLINE);
		const endColumn = boundedInt(selRaw.endColumn, 1, MAX_WIRE_COLUMN);
		if (typeof endDLine !== "undefined" && typeof endColumn !== "undefined") {
			const sel: FlowScriptCursorSelection = { endDLine, endColumn };
			const endAnchorId = wireAnchorId(selRaw.endAnchorId);
			if (endAnchorId && endAnchorId !== id) sel.endAnchorId = endAnchorId;
			// An id-shaped field that fails validation drops the whole selection —
			// never a silently different range.
			if (
				typeof selRaw.endAnchorId === "undefined" ||
				typeof endAnchorId !== "undefined"
			) {
				payload.sel = sel;
			}
		}
	}
	return payload;
}

/** Non-id entries dropped, duplicates removed, the set capped at `max`. */
function boundedIdList(value: unknown, max: number): string[] | undefined {
	if (!Array.isArray(value)) return undefined;
	const ids: string[] = [];
	for (const entry of value) {
		const id = wireAnchorId(entry);
		if (!id || ids.includes(id)) continue;
		ids.push(id);
		if (ids.length >= max) break;
	}
	return ids.length > 0 ? ids : undefined;
}

/**
 * Validate/bound a claims payload for the wire: non-id entries are dropped,
 * duplicates removed, the set capped at {@link MAX_CLAIM_ANCHORS}.
 */
export function sanitizeClaimsForWire(
	value: unknown,
): FlowScriptClaimsPayload | undefined {
	if (!isRecord(value)) return undefined;
	const anchorIds = boundedIdList(value.anchorIds, MAX_CLAIM_ANCHORS);
	if (!anchorIds) return undefined;
	return { anchorIds, ts: wireTs(value.ts) };
}

/**
 * Validate/bound a shared-scope payload for the wire: same closed shape as
 * claims — node ids only, deduped, capped at {@link MAX_SCOPE_NODES}.
 */
export function sanitizeScopeForWire(
	value: unknown,
): FlowScriptScopePayload | undefined {
	if (!isRecord(value)) return undefined;
	const nodeIds = boundedIdList(value.nodeIds, MAX_SCOPE_NODES);
	if (!nodeIds) return undefined;
	return { nodeIds, ts: wireTs(value.ts) };
}

/** Single entry point: sanitize any FlowScript presence field for the wire. */
export function sanitizeForWire(
	field: typeof FLOWSCRIPT_CURSOR_FIELD,
	value: unknown,
): FlowScriptCursorPayload | undefined;
export function sanitizeForWire(
	field: typeof FLOWSCRIPT_CLAIMS_FIELD,
	value: unknown,
): FlowScriptClaimsPayload | undefined;
export function sanitizeForWire(
	field: typeof FLOWSCRIPT_SCOPE_FIELD,
	value: unknown,
): FlowScriptScopePayload | undefined;
export function sanitizeForWire(
	field: string,
	value: unknown,
):
	| FlowScriptCursorPayload
	| FlowScriptClaimsPayload
	| FlowScriptScopePayload
	| undefined {
	if (field === FLOWSCRIPT_CURSOR_FIELD) return sanitizeCursorForWire(value);
	if (field === FLOWSCRIPT_CLAIMS_FIELD) return sanitizeClaimsForWire(value);
	if (field === FLOWSCRIPT_SCOPE_FIELD) return sanitizeScopeForWire(value);
	return undefined;
}

const WIRE_KEY_ALLOWLIST = new Set([
	"anchor",
	"id",
	"kind",
	"dLine",
	"column",
	"sel",
	"endAnchorId",
	"endDLine",
	"endColumn",
	"ts",
	"anchorIds",
	"nodeIds",
]);

const MAX_WIRE_DEPTH = 4;
const MAX_WIRE_ARRAY_LENGTH = Math.max(MAX_CLAIM_ANCHORS, MAX_SCOPE_NODES);

/**
 * Structural safety walk over a wire payload (rule 2 enforcement, used by the
 * protocol test): flags any field that could carry unbounded or free text —
 * strings that are neither a bounded anchor id nor a closed enum value, long
 * strings, unknown keys, oversized arrays, non-finite or negative numbers,
 * or non-JSON values. An empty result means the payload is metadata-only.
 */
export function wireSafetyViolations(value: unknown, path = "$"): string[] {
	return walkWireValue(value, path, 0);
}

function walkWireValue(value: unknown, path: string, depth: number): string[] {
	if (depth > MAX_WIRE_DEPTH) return [`${path}: exceeds max depth`];
	if (value === null || typeof value === "undefined") return [];
	if (typeof value === "number") {
		if (!Number.isFinite(value) || value < 0)
			return [`${path}: number out of bounds (${value})`];
		return [];
	}
	if (typeof value === "boolean") return [];
	if (typeof value === "string") {
		if (value.length > MAX_WIRE_STRING_LENGTH)
			return [`${path}: string longer than ${MAX_WIRE_STRING_LENGTH}`];
		if (
			!WIRE_ANCHOR_ID_PATTERN.test(value) &&
			!FLOWSCRIPT_ANCHOR_WIRE_KINDS.includes(value as FlowScriptAnchorWireKind)
		)
			return [`${path}: string is neither an anchor id nor an enum value`];
		return [];
	}
	if (Array.isArray(value)) {
		const violations: string[] = [];
		if (value.length > MAX_WIRE_ARRAY_LENGTH)
			violations.push(`${path}: array longer than ${MAX_WIRE_ARRAY_LENGTH}`);
		value.forEach((entry, i) => {
			violations.push(...walkWireValue(entry, `${path}[${i}]`, depth + 1));
		});
		return violations;
	}
	if (isRecord(value)) {
		const violations: string[] = [];
		for (const [key, entry] of Object.entries(value)) {
			if (!WIRE_KEY_ALLOWLIST.has(key)) {
				violations.push(`${path}.${key}: key not in wire allowlist`);
				continue;
			}
			violations.push(...walkWireValue(entry, `${path}.${key}`, depth + 1));
		}
		return violations;
	}
	return [`${path}: unsupported value type (${typeof value})`];
}
