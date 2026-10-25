export interface ComponentActionState {
	get(componentId: string): boolean;
	mark(componentId: string, on: boolean): void;
	subscribe(componentId: string, listener: () => void): () => void;
}

/** Pending actions are counted so overlapping runs keep their component busy. */
export function createComponentActionState(): ComponentActionState {
	const counts = new Map<string, number>();
	const listeners = new Map<string, Set<() => void>>();

	return {
		get: (componentId) => counts.has(componentId),
		mark(componentId, on) {
			if (!componentId) return;
			const current = counts.get(componentId) ?? 0;
			const next = on ? current + 1 : Math.max(0, current - 1);
			if (next === 0) counts.delete(componentId);
			else counts.set(componentId, next);
			if (current > 0 === next > 0) return;
			for (const listener of [...(listeners.get(componentId) ?? [])]) {
				listener();
			}
		},
		subscribe(componentId, listener) {
			let set = listeners.get(componentId);
			if (!set) {
				set = new Set();
				listeners.set(componentId, set);
			}
			set.add(listener);
			return () => {
				set.delete(listener);
				if (set.size === 0) listeners.delete(componentId);
			};
		},
	};
}
