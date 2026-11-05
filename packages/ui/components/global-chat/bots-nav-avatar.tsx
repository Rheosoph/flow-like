"use client";

import { useLiveQuery } from "dexie-react-hooks";
import { useState } from "react";
import { useAuth } from "react-oidc-context";
import { useShallow } from "zustand/react/shallow";
import { useConversations } from "../../hooks/use-inbox";
import { useUserLookup } from "../../hooks/use-user-lookup";
import { cn } from "../../lib/utils";
import { useSignedIn } from "../../state/backend-state";
import { ASSISTANT_BOT_ID } from "../../state/global-chat/global-chat-bots";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";
import { BotAvatar } from "../bots/bot-avatar";
import { readLatestBotChatActivity } from "./bots-nav-history";
import {
	latestBotActivity,
	latestInboxBotActivity,
	latestLiveBotActivity,
	navBotAgentState,
} from "./bots-nav-model";
import { useChatBotView } from "./use-global-chat-bot";

const readHistory = () => readLatestBotChatActivity().catch(() => null);

/** Keep a just-finished background turn visible while the inbox refresh catches up. */
function useLatestLiveActivity() {
	const owner = useAuth().user?.profile.sub;
	const live = useGlobalChatStore(useShallow(latestLiveBotActivity));
	const [observed, setObserved] = useState({ owner, activity: live });
	if (observed.owner !== owner) {
		setObserved({ owner, activity: live });
		return live;
	}
	const activity = latestBotActivity(observed.activity, live);
	if (activity !== observed.activity) setObserved({ owner, activity });
	return activity;
}

/** The last bot you exchanged messages with, drawn with its current saved look. */
export function BotsNavAvatar({
	size = 20,
	className,
}: Readonly<{ size?: number; className?: string }>) {
	const conversations = useConversations().data;
	const history = useLiveQuery(readHistory, []);
	const live = useLatestLiveActivity();
	const recent = latestBotActivity(
		latestInboxBotActivity(conversations ?? []),
		history,
		live,
	);
	// Null is Flux; only an absent conversation falls back to the Assistant.
	const botId = recent ? recent.botId : ASSISTANT_BOT_ID;
	const view = useChatBotView(botId);
	const signedIn = useSignedIn();
	const external = useUserLookup(
		signedIn && !view.bot && botId !== ASSISTANT_BOT_ID ? botId : undefined,
	).data;
	const agentState = useGlobalChatStore((state) =>
		navBotAgentState(state, botId),
	);

	return (
		<BotAvatar
			appearance={external?.bot?.appearance ?? view.appearance}
			size={size}
			agentState={agentState}
			animated
			decorative
			className={cn("[&_.fl-bot-badges]:hidden", className)}
		/>
	);
}
