"use client";

import { useTranslation } from "@flow-like/locales";
import type { ComponentType, ReactNode } from "react";
import {
	type BotFacts,
	botAnswerCase,
	deviceBot,
} from "../../../../lib/device-management/bot-config";
import { botAnswersText } from "./bot-copy";

/** A bot's facts, or its event's type and config as the Events editor holds them. */
type BotSource = { bot: BotFacts } | { eventType: string; config: unknown };

function factsOf(source: BotSource): BotFacts | null {
	if ("bot" in source) return source.bot;
	const result = deviceBot(source.eventType, source.config);
	return result?.ok ? result.bot : null;
}

/**
 * What a bot answers in groups and servers, as one line of its own. Renders
 * nothing for an event that is no bot or whose settings a device can't read.
 */
export function BotAnswersLine({
	as,
	className,
	...source
}: Readonly<
	BotSource & {
		/** The caller's own line component; a paragraph without one. */
		as?: ComponentType<{ className?: string; children?: ReactNode }>;
		className?: string;
	}
>) {
	const { t } = useTranslation("devices");
	const bot = factsOf(source);
	if (!bot) return null;
	const Line = as ?? "p";
	return (
		<Line data-bot-answers={botAnswerCase(bot)} className={className}>
			{botAnswersText(t, bot)}
		</Line>
	);
}
