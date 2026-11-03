"use client";

import { useTranslation } from "@flow-like/locales";
import { useMutation, useQuery } from "@tanstack/react-query";
import {
	GlobeIcon,
	MonitorIcon,
	MonitorOffIcon,
	SquareIcon,
} from "lucide-react";
import { useBackend } from "../../state/backend-state";
import type {
	IBot,
	IBotComputerView,
} from "../../state/backend-state/bot-state";
import { Button } from "../ui/button";

const ACTIVE_POLL_MS = 2000;
const ASLEEP_POLL_MS = 5000;

export type ComputerStageState = "asleep" | "active" | "failed";

/** What the tab shows: a sleeping computer, the bot's latest picture, or a read that failed. */
export function computerStageState(
	view: IBotComputerView | undefined,
	failed: boolean,
): ComputerStageState {
	if (view?.state === "active" && view.image) return "active";
	return failed && !view ? "failed" : "asleep";
}

export function ComputerStage({
	state,
	view,
	name,
	stopping,
	onStop,
	onRetry,
}: Readonly<{
	state: ComputerStageState;
	view?: IBotComputerView;
	name: string;
	stopping?: boolean;
	onStop: () => void;
	onRetry: () => void;
}>) {
	const { t } = useTranslation("bots");
	if (state === "active" && view?.image) {
		const KindIcon = view.kind === "browser" ? GlobeIcon : MonitorIcon;
		return (
			<div className="flex min-h-0 flex-1 flex-col gap-2 p-3">
				<div className="flex items-center gap-2 text-xs text-muted-foreground">
					<KindIcon className="size-3.5 shrink-0" aria-hidden />
					<span className="min-w-0 flex-1 truncate" title={view.caption ?? ""}>
						{view.caption ||
							(view.kind === "browser"
								? t("computer.browser", "Browser")
								: t("computer.screen", "Screen"))}
					</span>
					<Button
						type="button"
						variant="outline"
						size="sm"
						className="h-7 gap-1.5 px-2 text-xs"
						disabled={stopping}
						onClick={onStop}
					>
						<SquareIcon className="size-3 fill-current" aria-hidden />
						{t("computer.stop", "Stop")}
					</Button>
				</div>
				<div className="flex min-h-0 flex-1 items-center justify-center overflow-hidden rounded-lg border bg-muted/40">
					<img
						src={view.image}
						alt={t("computer.alt", "What {{name}} sees right now", { name })}
						className="max-h-full max-w-full object-contain"
					/>
				</div>
			</div>
		);
	}
	const failed = state === "failed";
	return (
		<div className="flex flex-1 flex-col items-center justify-center gap-3 px-6 py-10 text-center">
			<span className="flex size-11 items-center justify-center rounded-2xl bg-muted text-muted-foreground [&_svg]:size-5">
				<MonitorOffIcon aria-hidden />
			</span>
			<p className="max-w-72 text-sm font-medium">
				{failed
					? t("computer.failed", "Couldn't reach {{name}}'s computer.", {
							name,
						})
					: t("computer.asleep", "Computer is asleep")}
			</p>
			{!failed && (
				<p className="max-w-72 text-xs text-muted-foreground">
					{t(
						"computer.asleepHint",
						"{{name}} wakes it when it opens a browser or uses this computer.",
						{ name },
					)}
				</p>
			)}
			{failed && (
				<Button type="button" variant="outline" size="sm" onClick={onRetry}>
					{t("crew.retry", "Try again")}
				</Button>
			)}
		</div>
	);
}

/**
 * The Computer tab: the latest picture of what the bot's browser or screen shows, pulled every
 * couple of seconds while it runs. It sleeps when the bot has done nothing for a while, and
 * wherever the bot does not run on this device.
 */
export function BotComputerPane({
	bot,
	name,
}: Readonly<{ bot: IBot; name: string }>) {
	const host = useBackend().botState.computer;
	const query = useQuery({
		queryKey: ["bot-computer", bot.id],
		queryFn: () => (host ? host.view(bot.id) : Promise.resolve(undefined)),
		enabled: Boolean(host),
		refetchInterval: (current) =>
			current.state.data?.state === "active" ? ACTIVE_POLL_MS : ASLEEP_POLL_MS,
		refetchIntervalInBackground: false,
	});
	const stop = useMutation({
		mutationFn: async () => {
			await host?.stop(bot.id);
			await query.refetch();
		},
	});
	return (
		<ComputerStage
			state={computerStageState(query.data, query.isError)}
			view={query.data}
			name={name}
			stopping={stop.isPending}
			onStop={() => stop.mutate()}
			onRetry={() => void query.refetch()}
		/>
	);
}
