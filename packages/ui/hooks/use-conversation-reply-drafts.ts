"use client";

import { useEffect, useState } from "react";
import {
	type ReplyStream,
	watchReplyStreams,
} from "../lib/realtime/reply-streams";

const NO_DRAFTS: readonly ReplyStream[] = [];
/** A finished draft waits this long for its stored message before it is dropped. */
export const DONE_DRAFT_MS = 20_000;

/**
 * The replies a host is writing into one conversation right now, oldest first. A dropped draft
 * leaves at once. A finished one stays until its stored message arrives (the server finishes the
 * stream before it stores the reply), so the bubble never blinks out between the two; the caller
 * swaps it by message id.
 */
export function useConversationReplyDrafts(
	conversationId: string | undefined,
): readonly ReplyStream[] {
	const [drafts, setDrafts] = useState<ReadonlyMap<string, ReplyStream>>(
		new Map(),
	);

	useEffect(() => {
		setDrafts(new Map());
		if (!conversationId) return;
		const expiry = new Map<string, ReturnType<typeof setTimeout>>();
		const forget = (id: string) =>
			setDrafts((held) => {
				if (!held.has(id)) return held;
				const next = new Map(held);
				next.delete(id);
				return next;
			});
		const unwatch = watchReplyStreams(conversationId, (stream) => {
			const id = stream.messageId;
			const timer = expiry.get(id);
			if (timer !== undefined) clearTimeout(timer);
			expiry.delete(id);
			if (stream.state === "dropped") return forget(id);
			if (stream.state === "done")
				expiry.set(
					id,
					setTimeout(() => forget(id), DONE_DRAFT_MS),
				);
			setDrafts((held) => new Map(held).set(id, stream));
		});
		return () => {
			unwatch();
			for (const timer of expiry.values()) clearTimeout(timer);
		};
	}, [conversationId]);

	return drafts.size === 0 ? NO_DRAFTS : [...drafts.values()];
}
