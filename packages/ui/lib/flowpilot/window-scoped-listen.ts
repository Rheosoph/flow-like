import type { EventCallback, UnlistenFn } from "@tauri-apps/api/event";

/**
 * Listen to a FlowPilot bridge event addressed to this window or broadcast to all of them.
 *
 * The desktop host sends a run's tool requests to the window that started the run. A plain
 * `listen` also receives requests targeted at other windows, so every open window would execute
 * them, and duplicate nested specialists started under one request id would cancel each other.
 */
export async function listenInCurrentWindow<T>(
	event: string,
	handler: EventCallback<T>,
): Promise<UnlistenFn> {
	const [{ listen }, { getCurrentWebview }] = await Promise.all([
		import("@tauri-apps/api/event"),
		import("@tauri-apps/api/webview"),
	]);
	return listen<T>(event, handler, { target: getCurrentWebview().label });
}
