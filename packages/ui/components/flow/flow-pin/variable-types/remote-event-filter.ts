import type { IRemoteEvent } from "../../../../state/backend-state/types";

const REMOTE_EVENT_TYPES_BY_NODE: Readonly<
	Partial<Record<string, readonly string[]>>
> = {
	call_remote_api: ["rest"],
	call_remote_chat: ["simple_chat"],
};

/**
 * Fine-grained remote-call nodes only offer events they can execute. Nodes
 * without an entry (including the legacy call_remote_event node) keep the
 * complete remote event list.
 */
export function remoteEventTypesForNode(
	nodeName?: string,
): readonly string[] | undefined {
	return nodeName ? REMOTE_EVENT_TYPES_BY_NODE[nodeName] : undefined;
}

export function filterRemoteEventsForNode(
	events: readonly IRemoteEvent[],
	nodeName?: string,
): IRemoteEvent[] {
	const acceptedTypes = remoteEventTypesForNode(nodeName);
	if (!acceptedTypes) return [...events];
	return events.filter((event) => acceptedTypes.includes(event.event_type));
}
