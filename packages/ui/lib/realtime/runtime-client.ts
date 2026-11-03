import type { RealtimeRuntime } from "./runtime";
import { RealtimePortClient } from "./runtime-port-client";
import type {
	RealtimeClient,
	RealtimeClientOptions,
	RealtimeConnectionSession,
} from "./types";

export { getRealtimeClientForRuntime } from "./runtime-port-client";

/** Keep worker construction outside the modules loaded by the worker itself. */
export class BrowserRealtimeClient extends RealtimePortClient {
	constructor(
		session: RealtimeConnectionSession,
		options: RealtimeClientOptions = {},
		localRuntime?: RealtimeRuntime,
	) {
		super(session, options, localRuntime, () => {
			if (typeof SharedWorker === "undefined" || typeof window === "undefined")
				return undefined;
			return new SharedWorker(
				new URL("./runtime.shared-worker.ts", import.meta.url),
				{ type: "module", name: "flow-like-realtime-executions-v2" },
			);
		});
	}
}

export async function getRealtimeClient(
	session: RealtimeConnectionSession,
	options?: RealtimeClientOptions,
): Promise<RealtimeClient> {
	const client = new BrowserRealtimeClient(session, options);
	await client.connect();
	return client;
}
