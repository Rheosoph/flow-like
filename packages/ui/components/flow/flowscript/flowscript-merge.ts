/**
 * Statement-level three-way merge for the FlowScript editor (todo/
 * flowscript-collab.md rule 1): when the board changes behind a DIRTY buffer,
 * the rendered texts are segmented into anchor-keyed units and merged per
 * unit — the user's edited statements are never rewritten, remote-only
 * changes flow in, and a unit changed on both sides becomes an explicit
 * conflict the user resolves per statement.
 *
 * Line-ownership rule: an anchored line owns itself plus every following
 * UNANCHORED line up to (excluding) the next anchored line or end of text.
 * Branch arms, nested handlers and loop bodies carry their own `//@n:`
 * anchors, so only structural residue (closing braces, `} else {`, handler
 * openers like `onStream: {`, blank separators) and user-added unanchored
 * lines attach to the statement above them. Everything before the first
 * anchored line — the `use` block and interface declarations — forms one
 * leading "preamble" unit (variables carry per-variable `//@v:` anchors and
 * segment individually).
 */

import type { Monaco } from "@monaco-editor/react";
import {
	type FlowScriptAnchorIndex,
	parseFlowScriptAnchors,
} from "./flowscript-anchors";
import { FLOWSCRIPT_LANGUAGE_ID } from "./flowscript-language";

/* ── Segmentation ──────────────────────────────────────────────────────── */

/** Map key of the leading preamble unit (anchor ids are ≥ 10 chars, no clash). */
const PREAMBLE_KEY = "";

export interface FlowScriptMergeUnit {
	/** Undefined for the leading preamble (use block + interfaces). */
	anchorId?: string;
	/** 1-based line the unit starts on in its source text. */
	startLine: number;
	/** The unit's lines joined with `\n` (no trailing separator). */
	text: string;
}

export interface FlowScriptSegmentation {
	units: FlowScriptMergeUnit[];
	/** Set when the same anchor id starts two units — merge cannot key safely. */
	duplicateAnchorId?: string;
}

/**
 * Split a render into merge units per the line-ownership rule above.
 * Invariant: `units.map((u) => u.text).join("\n") === text`.
 */
export function segmentFlowScriptUnits(text: string): FlowScriptSegmentation {
	const lines = text.split("\n");
	const index = parseFlowScriptAnchors(text);
	const units: FlowScriptMergeUnit[] = [];
	const seen = new Set<string>();
	let duplicateAnchorId: string | undefined;

	const anchorLines: { line: number; id: string }[] = [];
	for (const anchor of index.anchors) {
		anchorLines.push({ line: anchor.line, id: anchor.id });
		if (seen.has(anchor.id) && duplicateAnchorId === undefined) {
			duplicateAnchorId = anchor.id;
		}
		seen.add(anchor.id);
	}
	anchorLines.sort((a, b) => a.line - b.line);

	const firstAnchorLine = anchorLines[0]?.line ?? lines.length + 1;
	if (firstAnchorLine > 1) {
		units.push({
			startLine: 1,
			text: lines.slice(0, firstAnchorLine - 1).join("\n"),
		});
	}
	for (let i = 0; i < anchorLines.length; i++) {
		const start = anchorLines[i].line;
		const end = (anchorLines[i + 1]?.line ?? lines.length + 1) - 1;
		units.push({
			anchorId: anchorLines[i].id,
			startLine: start,
			text: lines.slice(start - 1, end).join("\n"),
		});
	}
	return { units, duplicateAnchorId };
}

/* ── Three-way merge ───────────────────────────────────────────────────── */

export type FlowScriptMergeConflictKind = "both-changed" | "remote-deleted";

export interface FlowScriptMergeConflict {
	/** Undefined = the preamble unit. */
	anchorId?: string;
	kind: FlowScriptMergeConflictKind;
	/** The user's version of the unit; "" when they deleted it. */
	localBlock: string;
	/** The board's version of the unit; "" when it was deleted remotely. */
	freshBlock: string;
	/** 1-based start line of the unit inside `mergedText`. */
	line: number;
}

export interface FlowScriptMergeStats {
	/** Units updated to the fresh render (remote-only changes). */
	tookFresh: number;
	/** Units kept from the local buffer (local-only edits, incl. deletions). */
	tookLocal: number;
	/** Units that exist only in the fresh render (added remotely). */
	freshAdded: number;
	conflictCount: number;
}

export interface FlowScriptMergeSuccess {
	ok: true;
	/**
	 * Fresh-ordered merged render. Conflicted units carry the LOCAL block
	 * (the user's work is never rewritten by the merge itself); a unit the
	 * user deleted while the board changed it carries the fresh block so the
	 * conflict has a visible line to resolve on.
	 */
	mergedText: string;
	conflicts: FlowScriptMergeConflict[];
	stats: FlowScriptMergeStats;
	/** Anchored units whose fresh render differs from baseline (changed, added or deleted remotely). */
	remoteTouchedAnchorIds: string[];
}

export interface FlowScriptMergeFailure {
	ok: false;
	reason: string;
}

export type FlowScriptMergeResult =
	| FlowScriptMergeSuccess
	| FlowScriptMergeFailure;

interface MergedEntry {
	key: string;
	text: string;
	conflict?: Omit<FlowScriptMergeConflict, "line">;
}

function unitMap(
	segmentation: FlowScriptSegmentation,
): Map<string, FlowScriptMergeUnit> {
	const map = new Map<string, FlowScriptMergeUnit>();
	for (const unit of segmentation.units) {
		map.set(unit.anchorId ?? PREAMBLE_KEY, unit);
	}
	return map;
}

/**
 * Merge `local` (the user's dirty buffer, based on `baseline`) with `fresh`
 * (the board's new render) unit by unit:
 * - local unit unchanged vs baseline → take fresh (incl. remote deletion);
 * - local changed, fresh unchanged → take local (incl. local deletion);
 * - both changed identically → converged, take fresh;
 * - both changed differently → conflict `both-changed`;
 * - deleted remotely but locally edited → conflict `remote-deleted`;
 * - new units in fresh → included; new anchored units only in local (paste)
 *   → kept after their local predecessor;
 * - unanchored lines the user added ride the unit above them.
 *
 * Fails (never merges) when any input anchors the same id twice — unit
 * keying would be ambiguous; the caller falls back to the explicit guard.
 */
export function mergeFlowScript({
	baseline,
	local,
	fresh,
}: {
	baseline: string;
	local: string;
	fresh: string;
}): FlowScriptMergeResult {
	const segmented = {
		baseline: segmentFlowScriptUnits(baseline),
		local: segmentFlowScriptUnits(local),
		fresh: segmentFlowScriptUnits(fresh),
	};
	for (const [name, segmentation] of Object.entries(segmented)) {
		if (segmentation.duplicateAnchorId) {
			return {
				ok: false,
				reason: `duplicate anchor ${segmentation.duplicateAnchorId} in ${name} render`,
			};
		}
	}

	const baseByKey = unitMap(segmented.baseline);
	const localByKey = unitMap(segmented.local);
	const freshKeys = new Set(
		segmented.fresh.units.map((unit) => unit.anchorId ?? PREAMBLE_KEY),
	);

	const stats: FlowScriptMergeStats = {
		tookFresh: 0,
		tookLocal: 0,
		freshAdded: 0,
		conflictCount: 0,
	};
	const remoteTouched = new Set<string>();
	const merged: MergedEntry[] = [];

	for (const freshUnit of segmented.fresh.units) {
		const key = freshUnit.anchorId ?? PREAMBLE_KEY;
		const base = baseByKey.get(key);
		const localUnit = localByKey.get(key);

		if (!base) {
			if (key !== PREAMBLE_KEY) remoteTouched.add(key);
			if (!localUnit) {
				merged.push({ key, text: freshUnit.text });
				stats.freshAdded++;
			} else if (localUnit.text === freshUnit.text) {
				merged.push({ key, text: freshUnit.text });
			} else {
				// Added on both sides with different content.
				merged.push({
					key,
					text: localUnit.text,
					conflict: {
						anchorId: freshUnit.anchorId,
						kind: "both-changed",
						localBlock: localUnit.text,
						freshBlock: freshUnit.text,
					},
				});
			}
			continue;
		}

		const localChanged = !localUnit || localUnit.text !== base.text;
		const freshChanged = freshUnit.text !== base.text;
		if (freshChanged && key !== PREAMBLE_KEY) remoteTouched.add(key);

		if (!localChanged) {
			merged.push({ key, text: freshUnit.text });
			if (freshChanged) stats.tookFresh++;
		} else if (!freshChanged) {
			if (localUnit) merged.push({ key, text: localUnit.text });
			stats.tookLocal++;
		} else if (localUnit && localUnit.text === freshUnit.text) {
			merged.push({ key, text: freshUnit.text });
		} else {
			// A locally deleted unit has no local block to show — keep the fresh
			// block so the conflict is visible and "keep mine" can delete it.
			merged.push({
				key,
				text: localUnit ? localUnit.text : freshUnit.text,
				conflict: {
					anchorId: freshUnit.anchorId,
					kind: "both-changed",
					localBlock: localUnit?.text ?? "",
					freshBlock: freshUnit.text,
				},
			});
		}
	}

	// Local units absent from fresh: remote deletions of locally edited units
	// (conflict) and locally pasted anchored units (kept). Inserted after their
	// nearest local predecessor that made it into the merged output.
	const insertAfterLocalPredecessor = (
		localIndex: number,
		entry: MergedEntry,
	) => {
		let insertAt = 0;
		for (let p = localIndex - 1; p >= 0; p--) {
			const predecessorKey = segmented.local.units[p].anchorId ?? PREAMBLE_KEY;
			const at = merged.findIndex((m) => m.key === predecessorKey);
			if (at >= 0) {
				insertAt = at + 1;
				break;
			}
		}
		merged.splice(insertAt, 0, entry);
	};

	for (let i = 0; i < segmented.local.units.length; i++) {
		const localUnit = segmented.local.units[i];
		const key = localUnit.anchorId ?? PREAMBLE_KEY;
		if (freshKeys.has(key)) continue;
		const base = baseByKey.get(key);
		if (base) {
			if (key !== PREAMBLE_KEY) remoteTouched.add(key);
			if (localUnit.text === base.text) continue; // accept remote deletion
			insertAfterLocalPredecessor(i, {
				key,
				text: localUnit.text,
				conflict: {
					anchorId: localUnit.anchorId,
					kind: "remote-deleted",
					localBlock: localUnit.text,
					freshBlock: "",
				},
			});
		} else {
			insertAfterLocalPredecessor(i, { key, text: localUnit.text });
			stats.tookLocal++;
		}
	}

	const conflicts: FlowScriptMergeConflict[] = [];
	let line = 1;
	const parts: string[] = [];
	for (const entry of merged) {
		if (entry.conflict) conflicts.push({ ...entry.conflict, line });
		parts.push(entry.text);
		line += entry.text.split("\n").length;
	}
	stats.conflictCount = conflicts.length;

	return {
		ok: true,
		mergedText: parts.join("\n"),
		conflicts,
		stats,
		remoteTouchedAnchorIds: [...remoteTouched],
	};
}

/* ── Conflict resolution ───────────────────────────────────────────────── */

export type FlowScriptConflictResolution = "mine" | "theirs";

/**
 * Apply one side of a conflict to `text`: the unit keyed by the conflict's
 * anchor (the preamble when `anchorId` is undefined) is replaced with the
 * chosen block; an empty block removes the unit. When the unit is no longer
 * present (the user deleted it by hand), a non-empty block is appended.
 */
export function resolveFlowScriptConflict(
	text: string,
	conflict: Pick<
		FlowScriptMergeConflict,
		"anchorId" | "localBlock" | "freshBlock"
	>,
	resolution: FlowScriptConflictResolution,
): string {
	const replacement =
		resolution === "mine" ? conflict.localBlock : conflict.freshBlock;
	const { units } = segmentFlowScriptUnits(text);
	const key = conflict.anchorId ?? PREAMBLE_KEY;
	const at = units.findIndex((unit) => (unit.anchorId ?? PREAMBLE_KEY) === key);
	if (at < 0) {
		if (replacement === "") return text;
		return text.length === 0 ? replacement : `${text}\n${replacement}`;
	}
	if (units[at].text === replacement) return text;
	const parts = units.map((unit) => unit.text);
	if (replacement === "") parts.splice(at, 1);
	else parts[at] = replacement;
	return parts.join("\n");
}

/* ── Remote-touched overlap (apply preview) ────────────────────────────── */

/** Locally edited anchors that were ALSO changed remotely since the local baseline. */
export function intersectRemoteTouched(
	remoteTouchedAnchorIds: ReadonlySet<string>,
	localEditedAnchorIds: readonly string[],
): string[] {
	if (remoteTouchedAnchorIds.size === 0) return [];
	return localEditedAnchorIds.filter((id) => remoteTouchedAnchorIds.has(id));
}

/* ── Conflict CodeLens (Monaco adapter) ────────────────────────────────── */

export interface FlowScriptConflictLens {
	/** 1-based line the lens pair renders above (the unit's anchor line). */
	line: number;
	/** Index into the caller's current conflict list. */
	conflictIndex: number;
}

/**
 * Pure lens derivation: one lens pair per unresolved conflict, positioned at
 * the unit's CURRENT anchor line so edits above cannot detach the lens. A
 * conflict whose anchor vanished from the buffer renders no lens (the banner
 * still resolves it).
 */
export function deriveFlowScriptConflictLenses(
	conflicts: readonly Pick<FlowScriptMergeConflict, "anchorId">[],
	anchorIndex: FlowScriptAnchorIndex,
): FlowScriptConflictLens[] {
	const lenses: FlowScriptConflictLens[] = [];
	for (let i = 0; i < conflicts.length; i++) {
		const anchorId = conflicts[i].anchorId;
		const line = anchorId ? anchorIndex.firstLineById.get(anchorId) : 1;
		if (!line) continue;
		lenses.push({ line, conflictIndex: i });
	}
	return lenses;
}

export interface FlowScriptConflictLensLabels {
	keepMine: string;
	takeTheirs: string;
}

interface ConflictLensCommandArg {
	flowscriptConflictLens: true;
	conflictIndex: number;
	resolution: FlowScriptConflictResolution;
}

function isConflictLensCommandArg(
	value: unknown,
): value is ConflictLensCommandArg {
	return (
		typeof value === "object" &&
		value !== null &&
		(value as ConflictLensCommandArg).flowscriptConflictLens === true &&
		typeof (value as ConflictLensCommandArg).conflictIndex === "number"
	);
}

interface ConflictLensEditorLike {
	addCommand(
		keybinding: number,
		handler: (...args: unknown[]) => void,
	): string | null;
	getModel(): unknown;
}

export interface FlowScriptConflictLensHandle {
	refresh: () => void;
	dispose: () => void;
}

/**
 * Registers the per-conflict "Keep mine / Take theirs" CodeLens pair, scoped
 * to this panel's own model (same pattern as the run lens).
 */
export function registerFlowScriptConflictLens(
	monaco: Monaco,
	options: {
		editor: ConflictLensEditorLike;
		getConflicts: () => readonly FlowScriptMergeConflict[];
		getLabels: () => FlowScriptConflictLensLabels;
		onResolve: (
			conflictIndex: number,
			resolution: FlowScriptConflictResolution,
		) => void;
	},
): FlowScriptConflictLensHandle {
	const emitter = new monaco.Emitter<void>();
	const commandId = options.editor.addCommand(0, (...args: unknown[]) => {
		const payload = args.find(isConflictLensCommandArg);
		if (payload) options.onResolve(payload.conflictIndex, payload.resolution);
	});
	const provider = monaco.languages.registerCodeLensProvider(
		FLOWSCRIPT_LANGUAGE_ID,
		{
			// biome-ignore lint/suspicious/noExplicitAny: Emitter<void> vs IEvent<provider> — Monaco only uses it as a change signal
			onDidChange: emitter.event as any,
			provideCodeLenses: (model) => {
				if ((model as unknown) !== options.editor.getModel()) {
					return { lenses: [], dispose: () => {} };
				}
				const conflicts = options.getConflicts();
				if (conflicts.length === 0) {
					return { lenses: [], dispose: () => {} };
				}
				const labels = options.getLabels();
				const anchorIndex = parseFlowScriptAnchors(model.getValue());
				const lenses = deriveFlowScriptConflictLenses(
					conflicts,
					anchorIndex,
				).flatMap((lens) =>
					(["mine", "theirs"] as const).map((resolution) => ({
						range: new monaco.Range(lens.line, 1, lens.line, 1),
						command: {
							id: commandId ?? "",
							title:
								resolution === "mine" ? labels.keepMine : labels.takeTheirs,
							arguments: [
								{
									flowscriptConflictLens: true,
									conflictIndex: lens.conflictIndex,
									resolution,
								} satisfies ConflictLensCommandArg,
							],
						},
					})),
				);
				return { lenses, dispose: () => {} };
			},
		},
	);
	return {
		refresh: () => emitter.fire(undefined),
		dispose: () => {
			provider.dispose();
			emitter.dispose();
		},
	};
}
