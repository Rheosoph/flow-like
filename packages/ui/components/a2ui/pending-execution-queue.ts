import { type RefObject, useEffect, useMemo, useRef } from "react";

export const UNAVAILABLE_OUTSIDE_RUNNING_PAGE =
	"This workflow action is unavailable outside the running page.";

/** Per caller. A widget's startup sends a handful of events; more than this is a widget bug. */
export const MAX_HELD_ACTIONS = 16;

interface HeldAction<Args extends unknown[]> {
	args: Args;
	requireSuccess: boolean;
	resolve: () => void;
	reject: (error: Error) => void;
}

export interface PendingExecutionQueue<Args extends unknown[]> {
	isPending: () => boolean;
	/** Settles with the replayed call, or as not run when the caller unmounts first. */
	hold: (args: Args, requireSuccess: boolean) => Promise<void>;
}

function release<Args extends unknown[]>(held: HeldAction<Args>) {
	if (held.requireSuccess)
		held.reject(new Error(UNAVAILABLE_OUTSIDE_RUNNING_PAGE));
	else held.resolve();
}

/**
 * Holds actions started while a Page renders a restored bootstrap whose execution authority the
 * network has not confirmed yet. The page is inert then, but a micro widget iframe keeps running
 * and sends its startup event regardless; dropping it left the widget loading forever. Held calls
 * replay in order through the latest executor once the authority is confirmed. They are released
 * unrun when the caller unmounts first: a Page that remounts for a new revision mounts a new
 * widget, which sends its event again.
 */
export function usePendingExecutionQueue<Args extends unknown[]>(
	pending: boolean,
	executorRef: RefObject<((...args: Args) => Promise<void>) | null>,
): PendingExecutionQueue<Args> {
	const pendingRef = useRef(pending);
	pendingRef.current = pending;
	const heldRef = useRef<HeldAction<Args>[]>([]);
	const mountedRef = useRef(false);

	useEffect(() => {
		if (pending) return;
		const held = heldRef.current.splice(0);
		const execute = executorRef.current;
		for (const action of held) {
			if (!execute) {
				release(action);
				continue;
			}
			execute(...action.args).then(action.resolve, action.reject);
		}
	}, [pending, executorRef]);

	// StrictMode and Fast Refresh replay mount effects; only a caller still unmounted after that
	// replay is gone.
	useEffect(() => {
		mountedRef.current = true;
		return () => {
			mountedRef.current = false;
			setTimeout(() => {
				if (mountedRef.current) return;
				for (const action of heldRef.current.splice(0)) release(action);
			}, 0);
		};
	}, []);

	return useMemo(
		() => ({
			isPending: () => pendingRef.current,
			hold: (args, requireSuccess) =>
				new Promise<void>((resolve, reject) => {
					const action = { args, requireSuccess, resolve, reject };
					if (heldRef.current.length >= MAX_HELD_ACTIONS) {
						console.warn(
							`[A2UI] More than ${MAX_HELD_ACTIONS} actions arrived before the Page's execution authority was confirmed; dropping this one`,
						);
						release(action);
						return;
					}
					heldRef.current.push(action);
				}),
		}),
		[],
	);
}
