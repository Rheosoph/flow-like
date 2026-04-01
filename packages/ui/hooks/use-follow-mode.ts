import { useCallback, useEffect, useRef, useState } from "react";

interface Viewport {
	x: number;
	y: number;
	zoom: number;
}

interface UseFollowModeProps {
	// biome-ignore lint/suspicious/noExplicitAny: Yjs awareness is untyped
	awareness: any | undefined;
	sub?: string;
	setViewport: (viewport: Viewport, options?: { duration?: number }) => void;
	getViewport: () => Viewport;
}

export function useFollowMode({
	awareness,
	sub,
	setViewport,
	getViewport,
}: UseFollowModeProps) {
	const [followingSub, setFollowingSub] = useState<string | undefined>(
		undefined,
	);
	const followingSubRef = useRef<string | undefined>(undefined);
	const lastAppliedViewportRef = useRef<string>("");

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
			setFollowingSub(targetSub);
		},
		[sub],
	);

	const stopFollowing = useCallback(() => {
		setFollowingSub(undefined);
		lastAppliedViewportRef.current = "";
	}, []);

	const toggleFollow = useCallback(
		(targetSub: string) => {
			if (targetSub === sub) return;
			setFollowingSub((prev) => (prev === targetSub ? undefined : targetSub));
			lastAppliedViewportRef.current = "";
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
