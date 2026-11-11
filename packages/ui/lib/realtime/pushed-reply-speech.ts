import type { SpeechTimers, StreamedReply } from "./bot-call-speech";
import type { ReplyStream } from "./reply-streams";

const MAX_FINISHED = 64;
const DEFAULT_STALL_MS = 20_000;

const defaultTimers: SpeechTimers = {
	set: (callback, ms) => setTimeout(callback, ms),
	clear: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
};

/** Feeds replies a host pushes while it writes them into the call's speech streams. */
export function followPushedReplies({
	stream,
	timers = defaultTimers,
	stallMs = DEFAULT_STALL_MS,
}: {
	stream: (
		transcriptId: string,
		botId: string,
		options: { messageId: string },
	) => StreamedReply | null;
	timers?: SpeechTimers;
	stallMs?: number;
}): { apply(reply: ReplyStream): void; dispose(): void } {
	const handles = new Map<string, StreamedReply>();
	const stalls = new Map<string, unknown>();
	const finished = new Set<string>();

	const disarm = (id: string) => {
		const timer = stalls.get(id);
		if (timer === undefined) return;
		timers.clear(timer);
		stalls.delete(id);
	};
	const finish = (id: string) => {
		disarm(id);
		handles.delete(id);
		finished.delete(id);
		finished.add(id);
		if (finished.size > MAX_FINISHED) {
			const oldest = finished.values().next().value;
			if (oldest !== undefined) finished.delete(oldest);
		}
	};

	/** A speech stream for a reply that has none yet; none while this device cannot voice it. */
	const open = (reply: ReplyStream) => {
		const id = reply.messageId;
		const voiced = reply.state !== "dropped" && reply.text.trim() !== "";
		const handle = voiced
			? stream(reply.replyTo, reply.senderId, { messageId: id })
			: null;
		if (handle) handles.set(id, handle);
		// Nothing opens after the end: the stored message is spoken the usual way.
		else if (reply.state === "done" || reply.state === "dropped") finish(id);
		return handle ?? undefined;
	};
	const watch = (id: string, handle: StreamedReply) => {
		disarm(id);
		stalls.set(
			id,
			timers.set(() => {
				stalls.delete(id);
				handle.end();
				finish(id);
			}, stallMs),
		);
	};

	return {
		apply(reply) {
			const id = reply.messageId;
			if (finished.has(id)) return;
			const handle = handles.get(id) ?? open(reply);
			if (!handle) return;
			if (reply.state === "dropped") {
				handle.cancel();
				return finish(id);
			}
			handle.update(reply.text);
			if (reply.state === "done") {
				handle.end();
				return finish(id);
			}
			if (reply.state === "working") handle.flush();
			watch(id, handle);
		},
		dispose() {
			for (const timer of stalls.values()) timers.clear(timer);
			stalls.clear();
			handles.clear();
		},
	};
}
