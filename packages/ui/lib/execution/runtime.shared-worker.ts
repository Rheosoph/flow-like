import { rawPushChannel } from "../channel/transport";
import { persistExecutionChat } from "./chat-persistence";
import { ExecutionRuntime } from "./runtime";
import type { ExecutionAuthorization, ExecutionCommand } from "./types";

const runtime = new ExecutionRuntime({
	push: rawPushChannel,
	persist: persistExecutionChat,
});

(
	globalThis as unknown as { onconnect: (event: MessageEvent) => void }
).onconnect = (event) => {
	const port = event.ports[0];
	if (!port) return;
	port.onmessage = (
		message: MessageEvent<ExecutionCommand | ExecutionAuthorization>,
	) => {
		void runtime.receive(port, message.data);
	};
	port.start();
};
