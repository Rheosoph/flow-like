import { parseRealtimeReply } from "@flow-like/flow-like-ui/lib/realtime/providers";
import type { RealtimeReply } from "@flow-like/flow-like-ui/lib/realtime/types";
import { listen } from "@tauri-apps/api/event";

/** Emitted by the desktop's bot hosts while they write a live-call reply on this computer. */
export const BOT_HOST_REPLY_EVENT = "bot-host://reply";

/** Live-call replies a bot host on this computer writes; a message that is not a valid reply is dropped. */
export function watchBotHostReplies(
	onReply: (reply: RealtimeReply) => void,
): () => void {
	let closed = false;
	const listening = listen<unknown>(BOT_HOST_REPLY_EVENT, (event) => {
		const reply = parseRealtimeReply(event.payload);
		if (reply && !closed) onReply(reply);
	});
	return () => {
		closed = true;
		void listening.then(
			(unlisten) => unlisten(),
			() => undefined,
		);
	};
}
