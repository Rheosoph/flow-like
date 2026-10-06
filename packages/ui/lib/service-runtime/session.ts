import { QueryClient } from "@tanstack/react-query";
import type { ExecutionServiceContextValue } from "../../state/execution-service-context-value";
import {
	type TunnelFetchOptions,
	createTunnelFetch,
} from "../device-management/tunnel-fetch";
import { createServiceBackend, readServiceInventory } from "./backend";
import { createRuntimeChannels } from "./channels";
import { createRuntimeResources } from "./resources";
import { startRuntimeHistory } from "./session-history";
import { createServiceRequest } from "./transport";

/** One open app owns its transport, credentials, local state and renderer backend. */
export async function openRuntimeSession(
	options: TunnelFetchOptions,
	token: string | null,
) {
	const controller = new AbortController();
	const signal = controller.signal;
	const fetcher = createTunnelFetch({ ...options, signal });
	const request = createServiceRequest(token, fetcher);
	const abort = () => controller.abort(options.signal?.reason);
	options.signal?.addEventListener("abort", abort, { once: true });
	if (options.signal?.aborted) abort();
	const channels = createRuntimeChannels(fetcher, signal);
	let resources: ReturnType<typeof createRuntimeResources> | undefined;
	const client = new QueryClient({
		defaultOptions: {
			queries: { retry: false, refetchOnWindowFocus: false, gcTime: 0 },
		},
	});
	let clearHistory: (() => Promise<void>) | undefined;
	let closed = false;
	const close = () => {
		if (closed) return;
		closed = true;
		controller.abort();
		options.signal?.removeEventListener("abort", abort);
		channels.close();
		resources?.close();
		client.clear();
		void clearHistory?.().catch((error) =>
			console.error("Could not clear the deployed app session.", error),
		);
	};
	signal.addEventListener("abort", close, { once: true });
	try {
		signal.throwIfAborted();
		const inventory = await readServiceInventory(request, signal);
		signal.throwIfAborted();
		const sessionResources = createRuntimeResources({
			fetch: request,
			appId: inventory.project_id,
			signal,
		});
		resources = sessionResources;
		const appId = `device-runtime:${crypto.randomUUID()}`;
		clearHistory = startRuntimeHistory(appId);
		const { backend, bootstrap } = createServiceBackend(
			inventory,
			request,
			signal,
			{
				visibleAppId: appId,
				mapValue: (value, signal) =>
					sessionResources.mapValue(channels.bind(value), signal),
				resolveResource: (path, signal) =>
					sessionResources.resolve(path, signal),
			},
		);
		const unavailable = async () => {
			throw new Error("Only deployed events can run in this session.");
		};
		const execution: ExecutionServiceContextValue = {
			executeBoard: unavailable,
			executeBoardRemote: unavailable,
			executeBoardDirect: unavailable,
			executeEvent: (...args) => backend.eventState.executeEvent(...args),
			executeEventDirect: (...args) => backend.eventState.executeEvent(...args),
		};
		return {
			appId,
			inventory,
			backend,
			bootstrap,
			execution,
			client,
			signal,
			close,
			resolveResource: sessionResources.resolve,
		};
	} catch (error) {
		close();
		throw error;
	}
}

export type RuntimeSession = Awaited<ReturnType<typeof openRuntimeSession>>;
