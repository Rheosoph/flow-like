"use client";

import { GlobalChatView } from "../../components/global-chat/global-chat-view";

export default function ChatPage() {
	return (
		<main className="flex flex-col flex-1 w-full min-h-0 overflow-hidden">
			<GlobalChatView />
		</main>
	);
}
