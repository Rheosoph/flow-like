"use client";

import { useTranslation } from "@flow-like/locales";
import BotIcon from "lucide-react/dist/esm/icons/bot.js";
import { Fragment, useMemo } from "react";
import { DEFAULT_CHAT_AI_DISCLOSURE } from "../../../lib/chat-appearance";

export const FLOWPILOT_AI_DISCLOSURE =
	"FlowPilot is an AI assistant. Responses may be inaccurate—check important information.";

const sentenceSegmenter = new Intl.Segmenter(undefined, {
	granularity: "sentence",
});

type DisclosureSentence = { readonly text: string; readonly gap: string };

/** A dash stays on the line of the word before it instead of opening the next one. */
function glueDashes(text: string): string {
	return text.replace(/(\s*)([–—])/g, (_, space: string, dash: string) =>
		space ? ` ${dash}` : `⁠${dash}`,
	);
}

function splitSentences(text: string): DisclosureSentence[] {
	const sentences: DisclosureSentence[] = [];
	for (const { segment } of sentenceSegmenter.segment(text)) {
		const content = segment.trimEnd();
		if (content) {
			sentences.push({
				text: glueDashes(content),
				gap: segment.slice(content.length),
			});
		}
	}
	return sentences;
}

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

	const sentences = useMemo(() => splitSentences(disclosure), [disclosure]);

	// Each sentence is its own inline block, so a narrow footer wraps between
	// sentences before it splits one, and the icon stays on the first line.
	return (
		<p
			className="mx-auto w-fit max-w-full rounded-lg px-2 py-0.5 text-center text-[11px] leading-4 text-muted-foreground"
			data-fl-chat-ai-disclosure
			role="note"
		>
			{sentences.map((sentence, index) => (
				<Fragment key={`${index}:${sentence.text}`}>
					<span className="inline-block text-balance">
						{index === 0 && (
							<BotIcon
								aria-hidden="true"
								className="mr-1 inline-block size-3 align-[-2px] opacity-70"
							/>
						)}
						{sentence.text}
					</span>
					{index < sentences.length - 1 && (sentence.gap || <wbr />)}
				</Fragment>
			))}
		</p>
	);
}
