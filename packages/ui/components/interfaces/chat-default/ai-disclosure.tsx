"use client";

import { BotIcon } from "lucide-react";
import { resolveChatAiDisclosure } from "../../../lib/chat-appearance";

export function ChatAiDisclosure({ text }: Readonly<{ text?: string | null }>) {
	return (
		<div
			className="mx-auto flex w-full max-w-sm items-start gap-2 rounded-xl px-3 py-2 text-left text-[11px] leading-4 sm:w-fit sm:max-w-full sm:items-center sm:gap-1.5 sm:rounded-full sm:py-1 sm:text-center sm:text-xs sm:leading-normal"
			data-fl-chat-ai-disclosure
			role="note"
			style={{
				backgroundColor: "var(--fl-chat-disclosure-background, var(--muted))",
				color: "var(--fl-chat-disclosure-foreground, var(--foreground))",
			}}
		>
			<BotIcon
				aria-hidden="true"
				className="mt-px h-3.5 w-3.5 shrink-0 sm:mt-0"
			/>
			<span className="min-w-0 flex-1 text-balance sm:flex-none">
				{resolveChatAiDisclosure(text)}
			</span>
		</div>
	);
}
