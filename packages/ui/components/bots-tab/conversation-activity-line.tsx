"use client";

import { useTranslation } from "@flow-like/locales";
import { useMemo } from "react";
import { IdentityAvatar } from "../identity/identity-avatar";
import type { CrewDirectory } from "./use-crew-directory";

export interface BotActivity {
	botId: string;
	status: "queued" | "running" | "waiting";
}

const MAX_AVATARS = 3;
const DOT_DELAYS = ["0ms", "150ms", "300ms"] as const;

function Dots() {
	return (
		<span aria-hidden className="inline-flex items-end gap-0.5 pb-0.5">
			{DOT_DELAYS.map((delay) => (
				<span
					key={delay}
					className="size-1 rounded-full bg-current motion-safe:animate-bounce"
					style={{ animationDelay: delay }}
				/>
			))}
		</span>
	);
}

/**
 * Who is working on or writing into a conversation right now. It keeps its height when idle, so
 * pinning it above the composer never moves the transcript.
 */
export function ConversationActivityLine({
	bots,
	typing,
	directory,
}: Readonly<{
	bots: readonly BotActivity[];
	typing: readonly string[];
	directory: CrewDirectory;
}>) {
	const { t } = useTranslation("bots");

	const text = useMemo(() => {
		const parts = bots.map((bot) => {
			const name = directory(bot.botId).name;
			if (bot.status === "running")
				return t("crew.activity.running", "{{name}} is answering", { name });
			if (bot.status === "queued")
				return t("crew.activity.queued", "Waiting for {{name}}", { name });
			return t("crew.activity.waiting", "{{name}} is waiting for approval", {
				name,
			});
		});
		if (typing.length === 1) {
			parts.push(
				t("crew.activity.typingOne", "{{name}} is typing", {
					name: directory(typing[0] as string).name,
				}),
			);
		} else if (typing.length === 2) {
			parts.push(
				t("crew.activity.typingTwo", "{{first}} and {{second}} are typing", {
					first: directory(typing[0] as string).name,
					second: directory(typing[1] as string).name,
				}),
			);
		} else if (typing.length > 2) {
			parts.push(t("crew.activity.typingMany", "Several people are typing"));
		}
		return parts.join(" · ");
	}, [bots, typing, directory, t]);

	const avatars = useMemo(
		() => [...bots.map((bot) => bot.botId), ...typing].slice(0, MAX_AVATARS),
		[bots, typing],
	);

	return (
		<div
			aria-live="polite"
			data-fl-conversation-activity
			className="flex h-6 items-center gap-2 px-3 text-xs text-muted-foreground"
		>
			{text && (
				<>
					<span className="flex shrink-0 -space-x-1.5">
						{avatars.map((id) => (
							<span key={id} className="rounded-full ring-2 ring-background">
								<IdentityAvatar subject={id} size={16} />
							</span>
						))}
					</span>
					<span className="truncate">{text}</span>
					<Dots />
				</>
			)}
		</div>
	);
}
