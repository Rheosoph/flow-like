"use client";

import { useEffect, useState } from "react";
import type { FlowScriptAnchorIndex } from "./flowscript-anchors";
import {
	type FlowScriptPresenceSnapshot,
	resolveWireCursor,
} from "./flowscript-presence";
import {
	FLOWSCRIPT_CLAIMS_FIELD,
	FLOWSCRIPT_CURSOR_FIELD,
	FLOWSCRIPT_SCOPE_FIELD,
	FLOWSCRIPT_VIEW_FIELD,
	sanitizeCursorForWire,
} from "./flowscript-presence-protocol";

const REFRESH_MS = 500;

interface AwarenessLike {
	clientID: number;
	getStates: () => Map<number, Record<string, unknown>>;
	getLocalState: () => Record<string, unknown> | null;
}

interface SessionRow {
	clientId: number;
	self: boolean;
	sub: string;
	canvasCursor: boolean;
	selection: number;
	activeNode?: string;
	cursor?: string;
	resolved?: string;
	claims: number;
	scope: number;
	view?: string;
}

function short(id: unknown): string {
	return typeof id === "string" ? id.slice(-6) : "—";
}

function describeSession(
	clientId: number,
	state: Record<string, unknown> | undefined,
	selfClientId: number,
	anchorIndex: FlowScriptAnchorIndex,
): SessionRow {
	const cursor = sanitizeCursorForWire(state?.[FLOWSCRIPT_CURSOR_FIELD]);
	const resolved = cursor ? resolveWireCursor(anchorIndex, cursor) : undefined;
	const selection = state?.selection as { nodes?: unknown[] } | undefined;
	const claims = state?.[FLOWSCRIPT_CLAIMS_FIELD] as
		| { anchorIds?: unknown[] }
		| undefined;
	const scope = state?.[FLOWSCRIPT_SCOPE_FIELD] as
		| { nodeIds?: unknown[] }
		| undefined;
	const view = state?.[FLOWSCRIPT_VIEW_FIELD] as { file?: unknown } | undefined;
	return {
		clientId,
		self: clientId === selfClientId,
		sub: short(state?.sub),
		canvasCursor: Boolean(state?.cursor),
		selection: Array.isArray(selection?.nodes) ? selection.nodes.length : 0,
		activeNode: short(state?.activeNodeId),
		cursor: cursor
			? `${cursor.anchor.kind}:${short(cursor.anchor.id)} +${cursor.dLine}:${cursor.column}${cursor.sel ? ` sel(${cursor.sel.anchorIds?.length ?? 0})` : ""}`
			: undefined,
		resolved: cursor
			? resolved
				? `L${resolved.lineNumber}:${resolved.column}`
				: "anchor not in this buffer"
			: undefined,
		claims: Array.isArray(claims?.anchorIds) ? claims.anchorIds.length : 0,
		scope: Array.isArray(scope?.nodeIds) ? scope.nodeIds.length : 0,
		view: typeof view?.file === "string" ? short(view.file) : undefined,
	};
}

/**
 * Developer-mode readout of the presence pipeline as THIS client sees it:
 * every awareness session (raw fields, sanitized editor cursor, where it
 * resolves in the local buffer), the local session's published fields, and
 * what the decorations effect had to work with. Polls — awareness churns at
 * 20 Hz and this is a debugging aid, not a rendering path.
 */
export function FlowScriptPresenceDebug({
	awareness,
	snapshot,
	anchorIndex,
	enabled,
	hasTextFocus,
}: Readonly<{
	// biome-ignore lint/suspicious/noExplicitAny: Yjs awareness is untyped
	awareness: any | undefined;
	snapshot: FlowScriptPresenceSnapshot;
	anchorIndex: FlowScriptAnchorIndex;
	enabled: boolean;
	hasTextFocus: () => boolean;
}>) {
	const [rows, setRows] = useState<SessionRow[]>([]);
	const [focused, setFocused] = useState(false);
	const [tick, setTick] = useState(0);

	useEffect(() => {
		const read = () => {
			const aw = awareness as AwarenessLike | undefined;
			if (!aw) {
				setRows([]);
				return;
			}
			const next: SessionRow[] = [];
			for (const [clientId, state] of aw.getStates()) {
				next.push(describeSession(clientId, state, aw.clientID, anchorIndex));
			}
			next.sort(
				(a, b) => Number(b.self) - Number(a.self) || a.clientId - b.clientId,
			);
			setRows(next);
			setFocused(hasTextFocus());
			setTick((value) => value + 1);
		};
		read();
		const id = setInterval(read, REFRESH_MS);
		return () => clearInterval(id);
	}, [awareness, anchorIndex, hasTextFocus]);

	return (
		<div className="pointer-events-auto absolute bottom-2 right-3 z-20 max-h-64 w-[26rem] overflow-auto rounded-md border border-border bg-background/95 p-2 font-mono text-[10px] leading-tight text-foreground shadow-md backdrop-blur">
			<div className="mb-1 flex flex-wrap gap-x-3 gap-y-0.5 text-muted-foreground">
				<span>awareness: {awareness ? "yes" : "NO"}</span>
				<span>enabled: {enabled ? "yes" : "NO"}</span>
				<span>focus: {focused ? "yes" : "no"}</span>
				<span>anchors: {anchorIndex.anchors.length}</span>
				<span>
					store: {snapshot.cursors.length}c/{snapshot.claims.length}cl/
					{snapshot.canvasSelections.length}sel
				</span>
				<span>#{tick}</span>
			</div>
			{rows.length === 0 && (
				<div className="text-muted-foreground">no awareness sessions</div>
			)}
			{rows.map((row) => (
				<div
					key={row.clientId}
					className={row.self ? "text-primary" : undefined}
				>
					<span className="font-semibold">
						{row.self ? "me" : "peer"} {row.clientId}
					</span>{" "}
					sub={row.sub} canvas={row.canvasCursor ? "●" : "○"} sel=
					{row.selection}
					{row.activeNode !== "—" ? ` active=${row.activeNode}` : ""} claims=
					{row.claims} scope={row.scope}
					{row.view ? ` file=${row.view}` : ""}
					<div className="pl-3 text-muted-foreground">
						code cursor: {row.cursor ?? "none"}
						{row.resolved ? ` → ${row.resolved}` : ""}
					</div>
				</div>
			))}
		</div>
	);
}
