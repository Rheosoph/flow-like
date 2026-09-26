import { useCallback, useSyncExternalStore } from "react";
import type { SurfaceComponent } from "./types";

export type SurfaceComponents = Record<string, SurfaceComponent>;

/**
 * Holds a surface's components so every rendered node can subscribe to its own
 * entry instead of re-rendering with the whole tree. The reducers share
 * structure across updates, so an entry's identity changes exactly when that
 * component (or its style) does.
 *
 * `replace` runs during the renderer's render so nodes mounting in that pass
 * read the new record; `commit` runs once the tree has committed with that
 * record and notifies only the ids whose entries changed since the previous
 * commit. Diffing committed-to-committed — never against a record a render
 * React threw away — means no node can be left stale.
 */
export interface ComponentStore {
	get(id: string): SurfaceComponent | undefined;
	replace(next: SurfaceComponents): void;
	commit(next: SurfaceComponents): void;
	subscribe(id: string, listener: () => void): () => void;
}

export function createComponentStore(
	initial: SurfaceComponents,
): ComponentStore {
	let current = initial;
	let committed = initial;
	const listeners = new Map<string, Set<() => void>>();

	return {
		get: (id) => current[id],
		replace(next) {
			current = next;
		},
		commit(next) {
			current = next;
			const previous = committed;
			committed = next;
			if (previous === next) return;
			for (const [id, set] of [...listeners]) {
				if (previous[id] === next[id]) continue;
				for (const listener of [...set]) listener();
			}
		},
		subscribe(id, listener) {
			let set = listeners.get(id);
			if (!set) {
				set = new Set();
				listeners.set(id, set);
			}
			set.add(listener);
			return () => {
				set.delete(listener);
				if (set.size === 0) listeners.delete(id);
			};
		},
	};
}

export function useSurfaceComponent(
	store: ComponentStore,
	id: string,
): SurfaceComponent | undefined {
	const subscribe = useCallback(
		(listener: () => void) => store.subscribe(id, listener),
		[store, id],
	);
	const getSnapshot = useCallback(() => store.get(id), [store, id]);
	return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}
