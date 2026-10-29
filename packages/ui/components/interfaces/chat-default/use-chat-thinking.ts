import { useCallback, useEffect, useMemo, useState } from "react";
import type { IEventPayloadChat } from "../../../lib/schema/flow/event-payload-chat";
import {
	type ChatThinkingLevel,
	chatThinkingLevel,
	chatThinkingOptions,
	isChatThinkingLevel,
} from "../../thinking/thinking-options";
import { useThinkingLabels } from "../../thinking/thinking-picker";
import type { ChatBoxThinking } from "./chatbox";

function storageKey(appId: string, eventId: string) {
	return `flow-like:chat-thinking:${appId}:${eventId}`;
}

function readStoredLevel(key: string): ChatThinkingLevel | null {
	try {
		const value = globalThis.localStorage?.getItem(key);
		return isChatThinkingLevel(value) ? value : null;
	} catch {
		return null;
	}
}

function storeLevel(key: string, level: ChatThinkingLevel) {
	try {
		globalThis.localStorage?.setItem(key, level);
	} catch {
		// Private mode or blocked storage: the pick still holds for this session.
	}
}

/**
 * The thinking level an app chat sends with every turn. The user's pick is remembered per chat
 * event; without one (or when the app hides the picker) the app's default applies, then "low".
 */
export function useChatThinking(
	appId: string,
	eventId: string,
	config?: Partial<IEventPayloadChat>,
) {
	const key = storageKey(appId, eventId);
	const enabled = config?.allow_thinking_selection !== false;
	const fallback = chatThinkingLevel(config?.default_thinking);
	const [picked, setPicked] = useState(() => readStoredLevel(key));

	useEffect(() => {
		setPicked(readStoredLevel(key));
	}, [key]);

	const labels = useThinkingLabels();
	const options = useMemo(() => chatThinkingOptions(labels), [labels]);
	const level: ChatThinkingLevel = enabled ? (picked ?? fallback) : fallback;

	const onChange = useCallback(
		(id: string) => {
			if (!isChatThinkingLevel(id)) return;
			setPicked(id);
			storeLevel(key, id);
		},
		[key],
	);

	const picker = useMemo<ChatBoxThinking | undefined>(
		() => (enabled ? { value: level, options, onChange } : undefined),
		[enabled, level, options, onChange],
	);

	// Messages are stamped only with a level the user could choose; a hidden picker keeps the
	// app's default out of the transcript.
	return { level, picker, stamp: enabled ? level : undefined };
}
