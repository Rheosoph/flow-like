"use client";

import { BotIcon } from "lucide-react";
import { resolveChatAiDisclosure } from "../../../lib/chat-appearance";

export const FLOWPILOT_AI_DISCLOSURE =
	"FlowPilot is an AI assistant. Responses may be inaccurate—check important information.";

export function ChatAiDisclosure({ text }: Readonly<{ text?: string | null }>) {
	return (
		<div
			className="mx-auto flex w-full items-center justify-center gap-1.5 px-3 text-center text-[11px] leading-4 text-muted-foreground"
			data-fl-chat-ai-disclosure
			role="note"
		>
			<BotIcon aria-hidden="true" className="size-3 shrink-0 opacity-70" />
			<span className="text-balance">{resolveChatAiDisclosure(text)}</span>
		</div>
	);
}
