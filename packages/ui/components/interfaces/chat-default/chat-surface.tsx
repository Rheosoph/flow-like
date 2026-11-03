"use client";

import { useTranslation } from "@flow-like/locales";
import { Loader2Icon } from "lucide-react";
import { type Ref, useCallback, useState } from "react";
import { ChatAppearance } from "./appearance";
import { Chat, type IChatProps, type IChatRef } from "./chat";
import {
	ChatWidgetExecutionProvider,
	type RunWidgetAction,
} from "./chat-widget-execution";
import type { ISendMessageFunction } from "./chatbox";
import { ChatWelcome, type ChatWelcomeGreeting } from "./welcome";

function ConversationLoading() {
	const { t } = useTranslation("interfaces");
	return (
		<div className="flex h-full flex-col items-center justify-center gap-3">
			<Loader2Icon className="h-6 w-6 animate-spin text-muted-foreground" />
			<p className="text-sm text-muted-foreground">
				{t("loadingConversation", "Loading conversation...")}
			</p>
		</div>
	);
}

export interface ChatSurfaceProps extends IChatProps {
	chatRef: Ref<IChatRef>;
	/** While the transcript is still being read. */
	loading?: boolean;
	/** Set while the conversation is empty: the welcome stands in for the transcript. */
	welcome?: ChatWelcomeGreeting;
	runWidgetAction: RunWidgetAction;
}

/**
 * The standard chat: appearance, welcome, transcript and composer. It renders what the caller
 * hands it and reports every interaction back; the caller owns the messages and the engine that
 * answers them.
 */
export function ChatSurface({
	chatRef,
	loading = false,
	welcome,
	runWidgetAction,
	...chat
}: Readonly<ChatSurfaceProps>) {
	const { onSendMessage } = chat;
	const [sendingFromWelcome, setSendingFromWelcome] = useState(false);
	const sendFromWelcome = useCallback<ISendMessageFunction>(
		async (content, files, tools, audio, interaction) => {
			setSendingFromWelcome((files?.length ?? 0) > 0 || Boolean(audio));
			try {
				await onSendMessage(content, files, tools, audio, interaction);
			} finally {
				setSendingFromWelcome(false);
			}
		},
		[onSendMessage],
	);

	return (
		<ChatAppearance
			appId={chat.appId}
			eventId={chat.eventId ?? ""}
			config={chat.config}
			fallback={<ConversationLoading />}
		>
			{loading ? (
				<ConversationLoading />
			) : welcome ? (
				<ChatWelcome
					greeting={welcome}
					composer={{
						thinking: chat.thinking,
						model: chat.model,
						actions: chat.composerActions,
						onDraftChange: chat.onDraftChange,
					}}
					onSendMessage={sendFromWelcome}
					config={chat.config}
					isSending={sendingFromWelcome}
					appId={chat.appId}
				/>
			) : (
				<ChatWidgetExecutionProvider runWidgetAction={runWidgetAction}>
					<Chat ref={chatRef} {...chat} />
				</ChatWidgetExecutionProvider>
			)}
		</ChatAppearance>
	);
}
