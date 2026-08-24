import { useCallback, useEffect, useRef, useState } from "react";
import {
	FLOWSCRIPT_CURSOR_FIELD,
	type FlowScriptAnchorWireKind,
	sanitizeCursorForWire,
} from "../components/flow/flowscript/flowscript-presence-protocol";

interface Viewport {
	x: number;
	y: number;
	zoom: number;
}

export interface FollowedEditorAnchor {
	id: string;
	kind: FlowScriptAnchorWireKind;
}

interface UseFollowModeProps {
	// biome-ignore lint/suspicious/noExplicitAny: Yjs awareness is untyped
	awareness: any | undefined;
	sub?: string;
	setViewport: (viewport: Viewport, options?: { duration?: number }) => void;
	getViewport: () => Viewport;
	/**
	 * Invoked when the followed peer's latest activity is a FlowScript editor
	 * cursor (their text cursor moved more recently than their canvas pointer):
	 * the caller reveals the anchor in its own panel, or focuses the node on
	 * canvas when no panel is open. Fired once per anchor change.
	 */
	onFollowEditorAnchor?: (anchor: FollowedEditorAnchor) => void;
}

export function useFollowMode({
	awareness,
	sub,
	setViewport,
	getViewport,
	onFollowEditorAnchor,
}: UseFollowModeProps) {
	const [followingSub, setFollowingSub] = useState<string | undefined>(
		undefined,
	);
	const followingSubRef = useRef<string | undefined>(undefined);
	const lastAppliedViewportRef = useRef<string>("");
	const onFollowEditorAnchorRef = useRef(onFollowEditorAnchor);
	onFollowEditorAnchorRef.current = onFollowEditorAnchor;
	// Activity observed per session, LOCAL wall clock only — remote timestamps
	// are never compared across machines.
	const canvasActivityRef = useRef<Map<number, { key: string; at: number }>>(
		new Map(),
	);
	const editorActivityRef = useRef<Map<number, { key: string; at: number }>>(
		new Map(),
	);
	const lastFollowedEditorAnchorRef = useRef<string | undefined>(undefined);

	// Keep ref in sync
	followingSubRef.current = followingSub;

	// Broadcast own viewport via awareness
	useEffect(() => {
		if (!awareness) return;
		const interval = setInterval(() => {
			const vp = getViewport();
			awareness.setLocalStateField("viewport", {
				x: vp.x,
				y: vp.y,
				zoom: vp.zoom,
			});
		}, 200);
		return () => clearInterval(interval);
	}, [awareness, getViewport]);

	// Follow the target peer's viewport
	useEffect(() => {
		if (!awareness || !followingSub) return;

		const handleChange = () => {
			const targetSub = followingSubRef.current;
			if (!targetSub) return;

			const states = awareness.getStates() as Map<
				number,
				Record<string, unknown>
			>;

			// Cross-surface: when the peer's latest activity is a FlowScript editor
			// cursor, follow them into the text instead of chasing a stale viewport.
			const now = Date.now();
			let editorCandidate:
				| { anchor: FollowedEditorAnchor; at: number }
				| undefined;
			for (const [clientId, state] of states) {
				if (clientId === awareness.clientID) continue;
				if (state?.sub !== targetSub) continue;
				const cursor = state?.cursor as { x: number; y: number } | undefined;
				const canvasKey = cursor ? `${cursor.x}:${cursor.y}` : "";
				const prevCanvas = canvasActivityRef.current.get(clientId);
				if (!prevCanvas || prevCanvas.key !== canvasKey)
					canvasActivityRef.current.set(clientId, { key: canvasKey, at: now });
				const editorCursor = sanitizeCursorForWire(
					state?.[FLOWSCRIPT_CURSOR_FIELD],
				);
				const editorKey = editorCursor
					? `${editorCursor.anchor.id}:${editorCursor.dLine}:${editorCursor.column}:${editorCursor.ts}`
					: "";
				const prevEditor = editorActivityRef.current.get(clientId);
				if (!prevEditor || prevEditor.key !== editorKey)
					editorActivityRef.current.set(clientId, { key: editorKey, at: now });
				if (!editorCursor) continue;
				const editorAt = editorActivityRef.current.get(clientId)?.at ?? 0;
				const canvasAt = canvasActivityRef.current.get(clientId)?.at ?? 0;
				if (editorAt < canvasAt) continue;
				if (!editorCandidate || editorAt > editorCandidate.at) {
					editorCandidate = { anchor: editorCursor.anchor, at: editorAt };
				}
			}
			if (editorCandidate) {
				if (lastFollowedEditorAnchorRef.current !== editorCandidate.anchor.id) {
					lastFollowedEditorAnchorRef.current = editorCandidate.anchor.id;
					onFollowEditorAnchorRef.current?.(editorCandidate.anchor);
				}
			} else {
				// Back on canvas: re-arm so returning to the same statement re-fires.
				lastFollowedEditorAnchorRef.current = undefined;
			}

			// Collect viewports from all sessions of this user
			// Pick the one whose viewport differs most from what we last applied
			// (i.e. the session that actually moved)
			let candidateVp: Viewport | undefined;
			for (const [clientId, state] of states) {
				if (clientId === awareness.clientID) continue;
				if (state?.sub !== targetSub) continue;

				const vp = state?.viewport as Viewport | undefined;
				if (!vp) continue;

				const key = `${vp.x}:${vp.y}:${vp.zoom}`;
				if (key !== lastAppliedViewportRef.current) {
					candidateVp = vp;
					break;
				}
				// Keep first match as fallback even if it matches last applied
				if (!candidateVp) candidateVp = vp;
			}

			if (!candidateVp) return;

			const key = `${candidateVp.x}:${candidateVp.y}:${candidateVp.zoom}`;
			if (key === lastAppliedViewportRef.current) return;
			lastAppliedViewportRef.current = key;

			setViewport(
				{ x: candidateVp.x, y: candidateVp.y, zoom: candidateVp.zoom },
				{ duration: 300 },
			);
		};

		awareness.on("change", handleChange);
		// Apply immediately
		handleChange();

		return () => {
			try {
				awareness.off("change", handleChange);
			} catch {}
		};
	}, [awareness, followingSub, setViewport]);

	const startFollowing = useCallback(
		(targetSub: string) => {
			if (targetSub === sub) return;
			lastFollowedEditorAnchorRef.current = undefined;
			setFollowingSub(targetSub);
		},
		[sub],
	);

	const stopFollowing = useCallback(() => {
		setFollowingSub(undefined);
		lastAppliedViewportRef.current = "";
		lastFollowedEditorAnchorRef.current = undefined;
	}, []);

	const toggleFollow = useCallback(
		(targetSub: string) => {
			if (targetSub === sub) return;
			setFollowingSub((prev) => (prev === targetSub ? undefined : targetSub));
			lastAppliedViewportRef.current = "";
			lastFollowedEditorAnchorRef.current = undefined;
		},
		[sub],
	);

	// Stop following on deliberate pan/zoom (debounced wheel, middle-click, Escape)
	const wheelTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
	const wheelCountRef = useRef(0);

	useEffect(() => {
		if (!followingSub) return;

		const stopFollow = () => {
			setFollowingSub(undefined);
			lastAppliedViewportRef.current = "";
			lastFollowedEditorAnchorRef.current = undefined;
			wheelCountRef.current = 0;
		};

		const handleWheel = () => {
			wheelCountRef.current++;
			// Only stop after sustained scrolling (3+ events within 400ms)
			if (wheelCountRef.current >= 3) {
				stopFollow();
				return;
			}
			if (wheelTimerRef.current) clearTimeout(wheelTimerRef.current);
			wheelTimerRef.current = setTimeout(() => {
				wheelCountRef.current = 0;
			}, 400);
		};

		const handleMouseDown = (e: MouseEvent) => {
			if (e.button === 1) stopFollow();
		};

		const handleKeyDown = (e: KeyboardEvent) => {
			if (e.key === "Escape") stopFollow();
		};

		window.addEventListener("wheel", handleWheel, { passive: true });
		window.addEventListener("mousedown", handleMouseDown);
		window.addEventListener("keydown", handleKeyDown);

		return () => {
			window.removeEventListener("wheel", handleWheel);
			window.removeEventListener("mousedown", handleMouseDown);
			window.removeEventListener("keydown", handleKeyDown);
			if (wheelTimerRef.current) clearTimeout(wheelTimerRef.current);
			wheelCountRef.current = 0;
		};
	}, [followingSub]);

	return {
		followingSub,
		startFollowing,
		stopFollowing,
		toggleFollow,
	};
}
