"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowUpRightIcon, Loader2Icon } from "lucide-react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useCallback } from "react";
import { toast } from "sonner";
import type { IBot } from "../../state/backend-state/bot-state";
import { Button } from "../ui/button";
import { workspaceFullViewRoute } from "./bot-workspace-panes";
import {
	type WorkspaceFailure,
	useBotWorkspace,
	workspaceFailure,
} from "./use-bot-workspace";

/**
 * The way into the bot's workspace app. Opening the panel never sets a workspace up; following
 * the link does, the same as opening Files or Data.
 */
export function BotWorkspaceLink({
	bot,
	name,
	className,
}: Readonly<{ bot: IBot; name: string; className?: string }>) {
	const { t } = useTranslation("bots");
	const router = useRouter();
	const { supported, query } = useBotWorkspace(bot.id, false);

	const report = useCallback(
		(failure: WorkspaceFailure) => {
			if (failure.kind === "unsupported") {
				toast.error(
					t(
						"workspace.unsupported",
						"Files and data for {{name}} aren't available here yet.",
						{ name },
					),
				);
				return;
			}
			const title =
				failure.kind === "refused"
					? t(
							"workspace.refused",
							"{{name}} can't keep its own files and data yet.",
							{ name },
						)
					: t("workspace.failed", "Couldn't set up {{name}}'s workspace.", {
							name,
						});
			toast.error(title, { description: failure.message });
		},
		[name, t],
	);

	const open = useCallback(async () => {
		const result = await query.refetch();
		if (result.data) router.push(workspaceFullViewRoute(result.data.app_id));
		else report(workspaceFailure(result.error));
	}, [query, report, router]);

	if (!supported) return null;
	if (query.isError && workspaceFailure(query.error).kind !== "failed")
		return null;

	const label = t("panel.openWorkspace", "Open workspace");
	const icon = query.isFetching ? (
		<Loader2Icon className="size-3.5 animate-spin" aria-hidden />
	) : (
		<ArrowUpRightIcon className="size-3.5" aria-hidden />
	);
	const appId = query.data?.app_id;
	if (appId) {
		return (
			<Button asChild variant="outline" size="sm" className={className}>
				<Link href={workspaceFullViewRoute(appId)}>
					{label}
					{icon}
				</Link>
			</Button>
		);
	}
	return (
		<Button
			type="button"
			variant="outline"
			size="sm"
			className={className}
			disabled={query.isFetching}
			onClick={() => void open()}
		>
			{label}
			{icon}
		</Button>
	);
}
