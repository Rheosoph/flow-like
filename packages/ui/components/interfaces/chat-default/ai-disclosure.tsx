"use client";

import { useTranslation } from "@flow-like/locales";
import { BotIcon } from "lucide-react";
import { DEFAULT_CHAT_AI_DISCLOSURE } from "../../../lib/chat-appearance";

export const FLOWPILOT_AI_DISCLOSURE =
	"FlowPilot is an AI assistant. Responses may be inaccurate—check important information.";

/**
 * Anything an app author typed themselves is shown verbatim — only the two
 * built-in defaults are translated, since those are our copy, not theirs.
 */
export function ChatAiDisclosure({ text }: Readonly<{ text?: string | null }>) {
	const { t } = useTranslation("chat");
	const configured = typeof text === "string" ? text.trim() : "";

	// The defaults are repeated as literals because the extractor reads the
	// fallback argument statically — a constant here leaves the key empty.
	let disclosure: string;
	if (!configured || configured === DEFAULT_CHAT_AI_DISCLOSURE) {
		disclosure = t(
			"defaultChatAiDisclosure",
			"You’re chatting with AI — brilliant at patterns, occasionally creative with facts. Double-check the important stuff.",
		);
	} else if (configured === FLOWPILOT_AI_DISCLOSURE) {
		disclosure = t(
			"flowpilotAiDisclosure",
			"FlowPilot is an AI assistant. Responses may be inaccurate—check important information.",
		);
	} else {
		disclosure = configured;
	}

	return (
		<div
			className="mx-auto flex w-full items-center justify-center gap-1.5 px-3 text-center text-[11px] leading-4 text-muted-foreground"
			data-fl-chat-ai-disclosure
			role="note"
		>
			<BotIcon aria-hidden="true" className="size-3 shrink-0 opacity-70" />
			<span className="text-balance">{disclosure}</span>
		</div>
	);
}
