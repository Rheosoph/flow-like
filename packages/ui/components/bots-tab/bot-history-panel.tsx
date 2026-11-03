"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { HistoryIcon, Loader2Icon, PlusIcon } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { toast } from "sonner";
import { getErrorMessage } from "../../lib/error-message";
import { jumpList } from "../../state/global-chat/bot-history";
import { useBotHistoryStore } from "../../state/global-chat/bot-history-store";
import {
	botHasInboxHistory,
	jumpBotChat,
	loadBotSegments,
	showLatestBotChat,
	startBotTopic,
} from "../../state/global-chat/bot-inbox";
import { Button } from "../ui/button";

function segmentsKey(botId: string) {
	return ["bot-history", "segments", botId] as const;
}

function formatRange(from: number, to: number): string {
	const first = new Date(from).toLocaleDateString();
	const last = new Date(to).toLocaleDateString();
	return first === last ? first : `${first} – ${last}`;
}

/**
 * A bot's chat as one continuous history: the summarised stretches as a jump list, newest first,
 * and "New topic", which starts the next stretch. Clicking a row shows that part of the chat.
 */
export function BotHistoryPanel({
	botId,
	name,
}: Readonly<{ botId: string; name: string }>) {
	const { t } = useTranslation("bots");
	const queryClient = useQueryClient();
	const [busy, setBusy] = useState(false);
	const windowed = useBotHistoryStore((state) => state.views[botId]?.windowed);
	const topics = useBotHistoryStore((state) => state.topics[botId]);
	const available = botHasInboxHistory(botId);
	const segments = useQuery({
		queryKey: segmentsKey(botId),
		queryFn: () => loadBotSegments(botId),
		enabled: available,
		staleTime: 30_000,
	});

	const rows = useMemo(() => {
		const summarised = jumpList(segments.data ?? [], (from) =>
			t("history.untitled", "Chat of {{date}}", {
				date: from.toLocaleDateString(),
			}),
		);
		const fresh = (topics ?? [])
			.filter((mark) => !summarised.some((row) => row.seqFrom >= mark.seq))
			.map((mark) => ({
				id: `topic-${mark.seq}`,
				title: mark.title ?? t("history.newTopic", "New topic"),
				seqFrom: mark.seq,
				seqTo: mark.seq,
				from: mark.at,
				to: mark.at,
			}));
		return [...fresh.reverse(), ...summarised];
	}, [segments.data, topics, t]);

	const run = useCallback(
		async (action: () => Promise<boolean>) => {
			setBusy(true);
			try {
				if (!(await action())) {
					toast.info(
						t("history.busy", "Wait for the reply to finish, then try again."),
					);
				}
			} catch (error) {
				toast.error(getErrorMessage(error));
			} finally {
				setBusy(false);
			}
		},
		[t],
	);

	const newTopic = useCallback(
		() =>
			run(async () => {
				const started = await startBotTopic(botId);
				if (started) {
					toast.success(
						t("history.topicStarted", "{{name}} starts a new topic.", { name }),
					);
					await queryClient.invalidateQueries({ queryKey: segmentsKey(botId) });
				}
				return started;
			}),
		[botId, name, queryClient, run, t],
	);

	if (!available) return null;
	return (
		<section
			aria-label={t("history.title", "History")}
			className="flex flex-col gap-2 border-b px-4 py-3"
		>
			<div className="flex items-center justify-between gap-2">
				<h3 className="flex items-center gap-1.5 text-sm font-medium">
					<HistoryIcon className="size-4" aria-hidden />
					{t("history.title", "History")}
				</h3>
				<Button
					type="button"
					size="sm"
					variant="outline"
					disabled={busy}
					onClick={() => void newTopic()}
					className="h-7 gap-1 px-2 text-xs"
				>
					{busy ? (
						<Loader2Icon className="size-3.5 animate-spin" aria-hidden />
					) : (
						<PlusIcon className="size-3.5" aria-hidden />
					)}
					{t("history.newTopicAction", "New topic")}
				</Button>
			</div>
			{windowed && (
				<Button
					type="button"
					size="sm"
					variant="secondary"
					disabled={busy}
					onClick={() => void run(() => showLatestBotChat(botId))}
					className="h-7 justify-start px-2 text-xs"
				>
					{t("history.latest", "Back to the latest messages")}
				</Button>
			)}
			{rows.length === 0 ? (
				<p className="text-xs text-muted-foreground">
					{segments.isPending
						? t("history.loading", "Loading…")
						: t(
								"history.empty",
								"{{name}} summarises the chat into stretches you can jump to as it grows.",
								{ name },
							)}
				</p>
			) : (
				<ul className="flex max-h-56 flex-col gap-1 overflow-y-auto">
					{rows.map((row) => (
						<li key={row.id}>
							<button
								type="button"
								disabled={busy}
								onClick={() => void run(() => jumpBotChat(botId, row.seqFrom))}
								className="flex w-full flex-col items-start rounded-md px-2 py-1.5 text-left hover:bg-muted focus-visible:bg-muted"
							>
								<span className="line-clamp-1 text-sm">{row.title}</span>
								<span className="text-[11px] text-muted-foreground">
									{formatRange(row.from, row.to)} · #{row.seqFrom}
								</span>
							</button>
						</li>
					))}
				</ul>
			)}
		</section>
	);
}
