import { rawPushChannel } from "../channel/transport";
import { persistExecutionChat } from "../execution/chat-persistence";
import { ExecutionRuntime } from "../execution/runtime";
import type {
	ExecutionAuthorization,
	ExecutionCommand,
} from "../execution/types";
import { RealtimeRuntime } from "./runtime";
import { getRealtimeClientForRuntime } from "./runtime-port-client";
import type { RealtimeCommand } from "./types";

const runtime = new RealtimeRuntime();
const executions = new ExecutionRuntime({
	push: rawPushChannel,
	persist: persistExecutionChat,
	realtime: (session, options) =>
		getRealtimeClientForRuntime(runtime, session, options),
});
(
	globalThis as unknown as { onconnect: (event: MessageEvent) => void }
).onconnect = (event) => {
	const port = event.ports[0];
	if (!port) return;
	let executionPort: boolean | undefined;
	port.onmessage = (
		message: MessageEvent<
			RealtimeCommand | ExecutionCommand | ExecutionAuthorization
		>,
	) => {
		if (executionPort === undefined) {
			if (message.data.type !== "connect") return;
			executionPort = "scope" in message.data;
		}
		if (executionPort)
			void executions.receive(
				port,
				message.data as ExecutionCommand | ExecutionAuthorization,
			);
		else runtime.receive(port, message.data as RealtimeCommand);
	};
	port.start();
};
