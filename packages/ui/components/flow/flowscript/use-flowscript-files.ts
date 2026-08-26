"use client";

import { useCallback, useMemo, useRef, useState } from "react";

/**
 * One virtual FlowScript file as the panel left it: the draft, the board render it was diffed
 * against, that render's apply anchors and the editor's scroll/cursor state. `viewState` is
 * Monaco's opaque `editor.saveViewState()` payload — never inspected here.
 */
export interface FlowScriptFileBuffer {
	text: string;
	baseline: string;
	scopeAnchors?: string[];
	viewState?: unknown;
}

/** A buffer whose draft no longer matches the render it came from. */
export function isFlowScriptFileDirty(buffer: FlowScriptFileBuffer): boolean {
	return buffer.text !== buffer.baseline;
}

/** The files holding unapplied edits — what the tab strip puts a dot on. */
export function dirtyFlowScriptFileIds(
	buffers: ReadonlyMap<string, FlowScriptFileBuffer>,
): ReadonlySet<string> {
	const dirty = new Set<string>();
	for (const [fileId, buffer] of buffers) {
		if (isFlowScriptFileDirty(buffer)) dirty.add(fileId);
	}
	return dirty;
}

/**
 * The stash behind the FlowScript file tabs. Switching files hands the outgoing buffer to
 * `stash` and asks for the incoming one with `peek`; a file that was never stashed (or was
 * dropped) has no buffer and is re-rendered from the board instead. A restored buffer is kept,
 * because the panel overwrites it on the next switch away.
 */
export interface FlowScriptFileStore {
	/** The buffer of `fileId`, or `undefined` when the file must be fetched from the board. */
	peek(fileId: string): FlowScriptFileBuffer | undefined;
	/** Keeps the outgoing file's draft, baseline, anchors and editor seat. */
	stash(fileId: string, buffer: FlowScriptFileBuffer): void;
	/** Forgets one file (its next open re-renders from the board). */
	drop(fileId: string): void;
	/** Forgets every file — board switch, version switch, panel teardown. */
	clear(): void;
	/** Files holding unapplied edits. The current file's own dirty flag lives in the panel. */
	dirtyFileIds: ReadonlySet<string>;
}

const EMPTY_DIRTY: ReadonlySet<string> = new Set();

function sameIds(a: ReadonlySet<string>, b: ReadonlySet<string>): boolean {
	if (a.size !== b.size) return false;
	for (const id of a) if (!b.has(id)) return false;
	return true;
}

/**
 * The store without React, so the stash/restore contract is testable on its own. `onDirtyChange`
 * fires only when the dirty set actually changes — stashing a clean buffer over a clean buffer
 * must not re-render the board.
 */
export function createFlowScriptFileStore(
	onDirtyChange?: (dirtyFileIds: ReadonlySet<string>) => void,
): FlowScriptFileStore {
	const buffers = new Map<string, FlowScriptFileBuffer>();
	let dirtyFileIds: ReadonlySet<string> = EMPTY_DIRTY;

	const sync = () => {
		const next = dirtyFlowScriptFileIds(buffers);
		if (sameIds(next, dirtyFileIds)) return;
		dirtyFileIds = next;
		onDirtyChange?.(next);
	};

	return {
		peek: (fileId) => buffers.get(fileId),
		stash: (fileId, buffer) => {
			buffers.set(fileId, buffer);
			sync();
		},
		drop: (fileId) => {
			if (buffers.delete(fileId)) sync();
		},
		clear: () => {
			if (buffers.size === 0) return;
			buffers.clear();
			sync();
		},
		get dirtyFileIds() {
			return dirtyFileIds;
		},
	};
}

/**
 * Per-file FlowScript buffers, owned by the board: the panel mounts twice (desktop panel and
 * mobile sheet) and both must see the same drafts. Only the dirty set drives React — the buffers
 * live in a ref, since a keystroke must never re-render the board.
 */
export function useFlowScriptFiles(): FlowScriptFileStore {
	const [dirtyFileIds, setDirtyFileIds] =
		useState<ReadonlySet<string>>(EMPTY_DIRTY);
	const storeRef = useRef<FlowScriptFileStore | null>(null);
	storeRef.current ??= createFlowScriptFileStore(setDirtyFileIds);

	const peek = useCallback(
		(fileId: string) => storeRef.current?.peek(fileId),
		[],
	);
	const stash = useCallback(
		(fileId: string, buffer: FlowScriptFileBuffer) =>
			storeRef.current?.stash(fileId, buffer),
		[],
	);
	const drop = useCallback(
		(fileId: string) => storeRef.current?.drop(fileId),
		[],
	);
	const clear = useCallback(() => storeRef.current?.clear(), []);

	return useMemo(
		() => ({ peek, stash, drop, clear, dirtyFileIds }),
		[peek, stash, drop, clear, dirtyFileIds],
	);
}
