"use client";

import { type PropsWithChildren, createContext, useContext } from "react";

/** Posts `content` as the user's next message in the surrounding chat. */
export type ChatFollowUpSender = (content: string) => void;

const ChatFollowUpContext = createContext<ChatFollowUpSender | null>(null);

/**
 * Lets content inside a message answer on the user's behalf (an `a2ui` reply's
 * `send_message` buttons) through the composer's own send path. `send` is null
 * while the composer could not send either.
 */
export function ChatFollowUpProvider({
	send,
	children,
}: Readonly<PropsWithChildren<{ send: ChatFollowUpSender | null }>>) {
	return (
		<ChatFollowUpContext.Provider value={send}>
			{children}
		</ChatFollowUpContext.Provider>
	);
}

/** The surrounding chat's sender, or null outside a chat or while it cannot send. */
export function useChatFollowUp(): ChatFollowUpSender | null {
	return useContext(ChatFollowUpContext);
}
