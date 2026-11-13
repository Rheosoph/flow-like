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
	/** The last draft of each reply that needs nothing more; a later draft is its rewrite. */
	const finished = new Map<string, number>();

	const disarm = (id: string) => {
		const timer = stalls.get(id);
		if (timer === undefined) return;
		timers.clear(timer);
		stalls.delete(id);
	};
	const finish = ({ messageId: id, draft }: ReplyStream) => {
		disarm(id);
		handles.delete(id);
		finished.delete(id);
		finished.set(id, draft);
		if (finished.size > MAX_FINISHED) {
			const oldest = finished.keys().next().value;
			if (oldest !== undefined) finished.delete(oldest);
		}
	};
	const over = (reply: ReplyStream) =>
		(finished.get(reply.messageId) ?? -1) >= reply.draft;

	/** A speech stream for a reply that has none yet; none while this device cannot voice it. */
	const open = (reply: ReplyStream) => {
		const id = reply.messageId;
		const voiced = reply.state !== "dropped" && reply.text.trim() !== "";
		const handle = voiced
			? stream(reply.replyTo, reply.senderId, { messageId: id })
			: null;
		if (handle) handles.set(id, handle);
		// Nothing opens after the end: the stored message is spoken the usual way.
		else if (reply.state === "done" || reply.state === "dropped") finish(reply);
		return handle ?? undefined;
	};
	const watch = (reply: ReplyStream, handle: StreamedReply) => {
		const id = reply.messageId;
		disarm(id);
		stalls.set(
			id,
			timers.set(() => {
				stalls.delete(id);
				handle.end();
				finish(reply);
			}, stallMs),
		);
	};

	return {
		/** Takes the replies of the store in order; it ends a draft before it hands over the rewrite. */
		apply(reply) {
			if (over(reply)) return;
			const handle = handles.get(reply.messageId) ?? open(reply);
			if (!handle) return;
			if (reply.state === "dropped") {
				handle.cancel();
				return finish(reply);
			}
			handle.update(reply.text);
			if (reply.state === "done") {
				handle.end();
				return finish(reply);
			}
			if (reply.state === "working") handle.flush();
			watch(reply, handle);
		},
		dispose() {
			for (const timer of stalls.values()) timers.clear(timer);
			stalls.clear();
			handles.clear();
		},
	};
}
