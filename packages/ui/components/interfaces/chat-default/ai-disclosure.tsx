"use client";

import { BotIcon } from "lucide-react";
import { resolveChatAiDisclosure } from "../../../lib/chat-appearance";

export function ChatAiDisclosure({ text }: Readonly<{ text?: string | null }>) {
	return (
		<div
			className="mx-auto flex w-fit max-w-full items-center gap-1.5 rounded-full px-3 py-1 text-center text-xs"
			data-fl-chat-ai-disclosure
			role="note"
			style={{
				backgroundColor: "var(--fl-chat-disclosure-background, var(--muted))",
				color: "var(--fl-chat-disclosure-foreground, var(--foreground))",
			}}
		>
			<BotIcon aria-hidden="true" className="h-3.5 w-3.5 shrink-0" />
			<span>{resolveChatAiDisclosure(text)}</span>
		</div>
	);
}
