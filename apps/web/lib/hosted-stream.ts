import type { IIntercomEvent } from "@flow-like/flow-like-ui/lib/schema/events/intercom-event";

/** Preserve token whitespace and accept frames split anywhere across network chunks. */
export async function consumeHostedStream(
	body: ReadableStream<Uint8Array>,
	onEvents?: (events: IIntercomEvent[]) => void,
	onRunId?: (runId: string) => void,
): Promise<void> {
	const reader = body.getReader();
	const decoder = new TextDecoder();
	let buffer = "";
	let terminal = false;
	let failure: string | undefined;
	const consumeFrame = (frame: string) => {
		const data = frame
			.split(/\r?\n/)
			.filter((line) => line.startsWith("data:"))
			.map((line) => line.slice(5).replace(/^ /, ""))
			.join("\n");
		if (!data || data === "keep-alive" || data === "[DONE]") return;
		const event = JSON.parse(data) as IIntercomEvent;
		if (!event || typeof event.event_type !== "string")
			throw new Error("The server returned an invalid execution event.");
		if (
			event.event_type === "run_initiated" &&
			typeof event.payload?.run_id === "string"
		)
			onRunId?.(event.payload.run_id);
		onEvents?.([event]);
		if (
			event.event_type === "completed" ||
			event.event_type === "execution_complete"
		) {
			terminal = true;
			const status =
				typeof event.payload?.status === "string"
					? event.payload.status.trim().toLowerCase()
					: "completed";
			if (status !== "completed")
				failure = `The workflow finished with status: ${status}.`;
		}
		if (event.event_type === "error") {
			terminal = true;
			failure =
				typeof event.payload === "string"
					? event.payload
					: (event.payload?.message ?? "The workflow failed.");
		}
	};
	try {
		while (true) {
			const { value, done } = await reader.read();
			buffer += decoder.decode(value, { stream: !done });
			const frames = buffer.split(/\r?\n\r?\n/);
			buffer = frames.pop() ?? "";
			for (const frame of frames) consumeFrame(frame);
			if (done) {
				if (buffer.trim()) consumeFrame(buffer);
				break;
			}
			if (terminal) {
				await reader.cancel();
				break;
			}
		}
		if (failure) throw new Error(failure);
		if (!terminal)
			throw new Error(
				"The connection ended before the workflow completed. Please try again.",
			);
	} finally {
		reader.releaseLock();
	}
}
