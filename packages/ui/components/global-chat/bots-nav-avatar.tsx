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
import { dockRoute } from "../bots-tab/dock-selection";
import { BotAvatar } from "../bots/bot-avatar";
import { readLatestBotChatActivity } from "./bots-nav-history";
import {
	botsNavSelection,
	latestBotActivity,
	latestInboxBotActivity,
	latestLiveBotActivity,
	navBotAgentState,
} from "./bots-nav-model";
import { useChatBotList, useChatBotView } from "./use-global-chat-bot";

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

export interface BotsNavTarget {
	/** The last bot you exchanged messages with; null is Flux. */
	botId: string | null;
	/** The Bots tab on where that bot last talked. */
	href: string;
}

/** What the Bots button shows and opens, so both always name the same bot. */
export function useBotsNavTarget(): BotsNavTarget {
	const conversations = useConversations().data;
	const history = useLiveQuery(readHistory, []);
	const live = useLatestLiveActivity();
	const bots = useChatBotList().data;
	const recent = latestBotActivity(
		latestInboxBotActivity(conversations ?? []),
		history,
		live,
	);
	return {
		// Null is Flux; only an absent conversation falls back to the Assistant.
		botId: recent ? recent.botId : ASSISTANT_BOT_ID,
		href: dockRoute(botsNavSelection(recent, bots)),
	};
}

/** The last bot you exchanged messages with, drawn with its current saved look. */
export function BotsNavAvatar({
	size,
	className,
}: Readonly<{ size?: number; className?: string }>) {
	const { botId } = useBotsNavTarget();
	return <BotsNavBotAvatar botId={botId} size={size} className={className} />;
}

/** A bot as the Bots button draws it: its saved look, or its directory look for another's bot. */
export function BotsNavBotAvatar({
	botId,
	size = 20,
	className,
}: Readonly<{ botId: string | null; size?: number; className?: string }>) {
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
