"use client";

import { useEffect, useSyncExternalStore } from "react";
import {
	getIncomingCallJoin,
	getIncomingCallScope,
	subscribeIncomingCallJoin,
	takeIncomingCallJoin,
} from "../lib/realtime/incoming-call-join";
import { useBackend } from "../state/backend-state";

export function useIncomingCallJoin({
	userId,
	conversationId,
	liveCallId,
	ready,
	join,
}: {
	userId: string | null | undefined;
	conversationId: string;
	liveCallId: string | undefined;
	ready: boolean;
	join: (callId: string, context: AudioContext | null) => Promise<boolean>;
}) {
	const backend = useBackend();
	const pending = useSyncExternalStore(
		subscribeIncomingCallJoin,
		getIncomingCallJoin,
		() => undefined,
	);
	useEffect(() => {
		if (!pending || !ready || !userId || !liveCallId) return;
		const scope = getIncomingCallScope(backend);
		if (!scope) return;
		const request = takeIncomingCallJoin(
			scope,
			userId,
			conversationId,
			liveCallId,
		);
		if (request) void join(request.callId, request.context);
	}, [pending, ready, userId, conversationId, liveCallId, join, backend]);
}
