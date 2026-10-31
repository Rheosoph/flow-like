import type { ElementSource } from "../../components/a2ui/element-materializer";
import type { A2UIServerMessage } from "../../components/a2ui/types";
import type { IIntercomEvent } from "../schema/events/intercom-event";
import { type ExecutionTask, isExecutionFinished } from "./types";

interface ExecutionSurface {
	appId: string;
	pageId: string;
	source: () => ElementSource | null;
	onMessage?: (message: A2UIServerMessage) => void;
}

interface ExecutionSurfaceTarget {
	appId: string;
	pageId?: string;
	widgetInstanceId?: string;
}

const surfaces = new Set<ExecutionSurface>();
const listeners = new Set<() => void>();
const delivered = new Map<string, Set<string>>();
const replayableOutput = new Set([
	"beginRendering",
	"surfaceUpdate",
	"setCanvasSettings",
	"dataModelUpdate",
	"deleteSurface",
	"showScreen",
	"upsertElement",
	"createElement",
	"removeElement",
	"pruneDetached",
	"setGlobalState",
	"setPageState",
	"clearPageState",
	"clearFileInput",
]);

function surfaceKey(target: ExecutionSurfaceTarget): string {
	return JSON.stringify([
		target.appId,
		target.pageId,
		target.widgetInstanceId ?? "",
	]);
}

function matches(source: ElementSource, widgetInstanceId?: string) {
	return (source.widgetScope?.instanceId ?? "") === (widgetInstanceId ?? "");
}

export function registerExecutionSurface(
	surface: ExecutionSurface,
): () => void {
	surfaces.add(surface);
	for (const listener of listeners) listener();
	return () => {
		surfaces.delete(surface);
	};
}

export function subscribeExecutionSurfaces(listener: () => void): () => void {
	listeners.add(listener);
	return () => {
		listeners.delete(listener);
	};
}

/** Never answer a page read using an unrelated page that happens to be open. */
export function executionElementSource(
	appId: string,
	pageId?: string,
	widgetInstanceId?: string,
): ElementSource | null {
	if (!pageId) return null;
	for (const surface of surfaces) {
		if (surface.appId !== appId || surface.pageId !== pageId) continue;
		const source = surface.source();
		if (!source) continue;
		if (!matches(source, widgetInstanceId)) continue;
		return source;
	}
	return null;
}

/**
 * Both the original callback and surviving tabs use this path for state and element output.
 * A logical surface remembers delivered IDs across remounts, so replay cannot overwrite edits
 * made after that output. A surface that missed the event can still receive it while its run is active.
 */
export function deliverExecutionSurfaceEvent(
	target: ExecutionSurfaceTarget,
	event: IIntercomEvent,
	fallback?: (message: A2UIServerMessage) => void,
): boolean {
	if (
		!target.pageId ||
		event.event_type !== "a2ui" ||
		!replayableOutput.has(event.payload?.type)
	)
		return false;
	const key = surfaceKey(target);
	const seen = delivered.get(key) ?? new Set<string>();
	if (event.event_id && seen.has(event.event_id)) return true;
	let sink = fallback;
	for (const surface of surfaces) {
		if (
			surface.appId !== target.appId ||
			surface.pageId !== target.pageId ||
			!surface.onMessage
		)
			continue;
		const source = surface.source();
		if (!source || !matches(source, target.widgetInstanceId)) continue;
		sink = surface.onMessage;
		break;
	}
	if (!sink) return false;
	// Mark before dispatch because a state update may synchronously publish another snapshot.
	if (event.event_id) seen.add(event.event_id);
	delivered.set(key, seen);
	try {
		sink(event.payload as A2UIServerMessage);
	} catch (error) {
		seen.delete(event.event_id);
		throw error;
	}
	return true;
}

/** Only active runs may catch a newly mounted page up; terminal history never rewrites its state. */
export function deliverExecutionSurfaceEvents(task: ExecutionTask): void {
	if (isExecutionFinished(task)) return;
	for (const event of task.events) deliverExecutionSurfaceEvent(task, event);
}
