"use client";

import { useTranslation } from "@flow-like/locales";
import { FoldVerticalIcon } from "lucide-react";
import { useMemo } from "react";
import { IRole } from "../../lib/schema/llm/history";
import { cn } from "../../lib/utils";
import { useCompactionRequestStore } from "../../state/global-chat/compaction-request";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";
import type { IMessage } from "../interfaces/chat-default/chat-db";
import { usageEfficiency } from "../interfaces/chat-default/usage-efficiency";
import { Button } from "../ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "../ui/popover";

const RING_RADIUS = 6;
const RING_LENGTH = 2 * Math.PI * RING_RADIUS;

interface ContextReading {
	tokens: number;
	window: number;
	measured: boolean;
	share: number;
}

/** The context of the newest finished reply's last request; a reply still streaming has none yet. */
function latestReading(messages: readonly IMessage[]): ContextReading | null {
	for (let index = messages.length - 1; index >= 0; index--) {
		const message = messages[index];
		if (message.inner.role === IRole.User || !message.usage_stats?.length)
			continue;
		const context = usageEfficiency(message.usage_stats).context;
		if (!context) return null;
		return {
			tokens: context.tokens,
			window: context.context_window,
			measured: context.measured,
			share: Math.min(1, Math.max(0, context.tokens / context.context_window)),
		};
	}
	return null;
}

function fillTone(share: number | undefined): string {
	if (share === undefined) return "text-muted-foreground";
	if (share >= 0.85) return "text-critical-solid";
	if (share >= 0.6) return "text-warning-solid";
	return "text-primary";
}

function Ring({ share }: Readonly<{ share: number | undefined }>) {
	return (
		<svg
			viewBox="0 0 16 16"
			aria-hidden="true"
			className={cn("size-4 shrink-0 -rotate-90", fillTone(share))}
		>
			<circle
				cx="8"
				cy="8"
				r={RING_RADIUS}
				fill="none"
				strokeWidth="2"
				className="stroke-muted-foreground/25"
			/>
			{share !== undefined && share > 0 && (
				<circle
					cx="8"
					cy="8"
					r={RING_RADIUS}
					fill="none"
					stroke="currentColor"
					strokeWidth="2"
					strokeLinecap="round"
					strokeDasharray={RING_LENGTH}
					strokeDashoffset={RING_LENGTH * (1 - share)}
				/>
			)}
		</svg>
	);
}

function CompactAction({
	conversationId,
	canCompact,
}: Readonly<{ conversationId: string; canCompact: boolean }>) {
	const { t } = useTranslation("chat");
	const requested = useCompactionRequestStore((s) =>
		Boolean(s.requested[conversationId]),
	);
	const setRequested = useCompactionRequestStore((s) => s.setRequested);
	if (!canCompact)
		return (
			<p className="text-xs text-muted-foreground">
				{t(
					"contextMeter.hostedBot",
					"This bot answers on its own host, so compacting isn't available from this chat.",
				)}
			</p>
		);
	if (requested)
		return (
			<div className="space-y-2">
				<p className="text-xs">
					{t(
						"contextMeter.queued",
						"Your next message starts by summarizing the earlier conversation.",
					)}
				</p>
				<Button
					type="button"
					variant="outline"
					size="sm"
					className="h-11 w-full sm:h-8"
					onClick={() => setRequested(conversationId, false)}
				>
					{t("contextMeter.cancel", "Don't compact")}
				</Button>
			</div>
		);
	return (
		<div className="space-y-2">
			<p className="text-xs text-muted-foreground">
				{t(
					"contextMeter.compactHint",
					"Summarizes the earlier conversation before the next reply. Later replies build on the summary; the newest messages stay word for word.",
				)}
			</p>
			<Button
				type="button"
				variant="secondary"
				size="sm"
				className="h-11 w-full gap-1.5 sm:h-8"
				onClick={() => setRequested(conversationId, true)}
			>
				<FoldVerticalIcon className="size-3.5" />
				{t("contextMeter.compact", "Compact before next reply")}
			</Button>
		</div>
	);
}

/**
 * How full the model's context was in the chat's last reply, always in the composer. Its menu
 * queues a compaction: the bot's next turn summarizes the older conversation before it answers.
 */
export function ContextMeter({
	canCompact,
}: Readonly<{ canCompact: boolean }>) {
	const { t } = useTranslation("chat");
	const conversationId = useGlobalChatStore((s) => s.activeConversationId);
	const messages = useGlobalChatStore((s) => s.messages);
	const requested = useCompactionRequestStore((s) =>
		Boolean(s.requested[conversationId]),
	);
	const reading = useMemo(() => latestReading(messages), [messages]);
	const number = useMemo(() => new Intl.NumberFormat(), []);
	const percent = reading ? `${Math.round(reading.share * 100)}%` : null;
	const label = percent
		? t("contextMeter.labelUsed", "Context window: {{percent}} used", {
				percent,
			})
		: t("contextMeter.label", "Context window");
	return (
		<Popover>
			<PopoverTrigger asChild>
				<Button
					type="button"
					variant="ghost"
					size="sm"
					aria-label={label}
					title={label}
					className="relative h-11 shrink-0 gap-1.5 rounded-lg px-2 text-xs tabular-nums sm:h-8"
				>
					<Ring share={reading?.share} />
					{percent && <span className="hidden sm:inline">{percent}</span>}
					{requested && (
						<FoldVerticalIcon
							aria-hidden="true"
							className="size-3.5 text-primary"
						/>
					)}
				</Button>
			</PopoverTrigger>
			<PopoverContent side="top" align="start" className="z-10000 w-72 p-3">
				<div className="space-y-3">
					<div className="space-y-1">
						<p className="text-xs font-medium text-muted-foreground">
							{t("contextMeter.label", "Context window")}
						</p>
						{reading ? (
							<>
								<p className="text-lg font-semibold tabular-nums">
									{t("contextMeter.used", "{{percent}} used", {
										percent: percent ?? "",
									})}
								</p>
								<div
									role="meter"
									aria-label={t("contextMeter.label", "Context window")}
									aria-valuemin={0}
									aria-valuemax={reading.window}
									aria-valuenow={reading.tokens}
									className="h-1.5 overflow-hidden rounded-full bg-muted"
								>
									<div
										className={cn(
											"h-full rounded-full bg-current",
											fillTone(reading.share),
										)}
										style={{ width: `${reading.share * 100}%` }}
									/>
								</div>
								<p className="text-xs text-muted-foreground tabular-nums">
									{reading.measured
										? t(
												"contextMeter.tokens",
												"{{tokens}} of {{window}} tokens in the last reply's final request",
												{
													tokens: number.format(reading.tokens),
													window: number.format(reading.window),
												},
											)
										: t(
												"contextMeter.tokensEstimated",
												"About {{tokens}} of {{window}} tokens in the last reply's final request",
												{
													tokens: number.format(reading.tokens),
													window: number.format(reading.window),
												},
											)}
								</p>
							</>
						) : (
							<p className="text-xs text-muted-foreground">
								{t(
									"contextMeter.empty",
									"Shows how full the model's context is after the next reply.",
								)}
							</p>
						)}
					</div>
					<p className="text-xs text-muted-foreground">
						{t(
							"contextMeter.automatic",
							"Older messages are also summarized on their own as the conversation grows.",
						)}
					</p>
					<CompactAction
						conversationId={conversationId}
						canCompact={canCompact}
					/>
				</div>
			</PopoverContent>
		</Popover>
	);
}
