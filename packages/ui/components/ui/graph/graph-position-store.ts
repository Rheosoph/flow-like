import type Graph from "graphology";
import type { LayoutPosition } from "./graph-layout";

const STORE_PREFIX = "flow-like:graph-scene:";
const STORE_VERSION = 1;
/** Above this a scene is not worth persisting — the layout is cheap to redo and the JSON is not. */
const MAX_STORED_NODES = 3000;

interface StoredGraphScene {
	v: number;
	savedAt: number;
	positions: Record<string, [number, number]>;
	pinned: string[];
}

export interface GraphSceneSnapshot {
	positions: Map<string, LayoutPosition>;
	pinned: Set<string>;
}

/**
 * Positions a reader arranged by hand survive a reload. Local-only and
 * best-effort: a missing or malformed entry means a fresh layout, never an
 * error.
 */
export function loadGraphScene(key: string): GraphSceneSnapshot | null {
	if (typeof window === "undefined") return null;
	try {
		const raw = window.localStorage.getItem(`${STORE_PREFIX}${key}`);
		if (!raw) return null;
		const parsed = JSON.parse(raw) as StoredGraphScene;
		if (parsed?.v !== STORE_VERSION || typeof parsed.positions !== "object") {
			return null;
		}
		const positions = new Map<string, LayoutPosition>();
		for (const [nodeId, coords] of Object.entries(parsed.positions)) {
			if (
				Array.isArray(coords) &&
				Number.isFinite(coords[0]) &&
				Number.isFinite(coords[1])
			) {
				positions.set(nodeId, { x: coords[0], y: coords[1] });
			}
		}
		return {
			positions,
			pinned: new Set(Array.isArray(parsed.pinned) ? parsed.pinned : []),
		};
	} catch {
		return null;
	}
}

export function saveGraphScene(
	key: string,
	graph: Graph,
	pinned: ReadonlySet<string>,
): void {
	if (typeof window === "undefined") return;
	if (graph.order === 0 || graph.order > MAX_STORED_NODES) return;
	try {
		const positions: Record<string, [number, number]> = {};
		graph.forEachNode((nodeId, attrs) => {
			const x = attrs.x;
			const y = attrs.y;
			if (typeof x === "number" && typeof y === "number") {
				positions[nodeId] = [Math.round(x * 10) / 10, Math.round(y * 10) / 10];
			}
		});
		const scene: StoredGraphScene = {
			v: STORE_VERSION,
			savedAt: Date.now(),
			positions,
			pinned: [...pinned],
		};
		window.localStorage.setItem(`${STORE_PREFIX}${key}`, JSON.stringify(scene));
	} catch {
		// Quota or privacy mode — the scene just will not be remembered.
	}
}

export function clearGraphScene(key: string): void {
	if (typeof window === "undefined") return;
	try {
		window.localStorage.removeItem(`${STORE_PREFIX}${key}`);
	} catch {
		// Nothing to recover from.
	}
}
