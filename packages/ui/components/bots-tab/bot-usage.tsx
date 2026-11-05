"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { GaugeIcon, RefreshCwIcon } from "lucide-react";
import { useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { useInvoke } from "../../hooks/use-invoke";
import { apiErrorMessage } from "../../lib/api-error";
import {
	type MonthlyUsageLimits,
	type UsageBudgetView,
	botUsagePath,
} from "../../lib/monthly-usage";
import { useBackend } from "../../state/backend-state";
import {
	MonthlyUsageLimitsEditor,
	UsageBudgetSummary,
} from "../settings/monthly-usage-budget";
import { Button } from "../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";

function BotUsageScope({
	view,
	canEdit,
	onSave,
}: Readonly<{
	view: UsageBudgetView;
	canEdit: boolean;
	onSave: (view: UsageBudgetView, limits: MonthlyUsageLimits) => Promise<void>;
}>) {
	const { t } = useTranslation("bots");
	return (
		<div className="space-y-4">
			<UsageBudgetSummary view={view} />
			{canEdit && (
				<MonthlyUsageLimitsEditor
					key={JSON.stringify(view.ownerLimits)}
					initial={view.ownerLimits}
					ceiling={view.adminLimits ?? view.defaultLimits}
					label={t("usage.yourLimits", "Your monthly limits")}
					onSave={(limits) => onSave(view, limits)}
				/>
			)}
			{view.groups.length > 0 && (
				<section className="space-y-2">
					<h3 className="text-sm font-medium">
						{t("usage.groups", "Usage by group")}
					</h3>
					<p className="text-xs text-muted-foreground">
						{t(
							"usage.groupHint",
							"Group limits apply to this bot in that group. All groups also share the bot's overall limit.",
						)}
					</p>
					{view.groups.map((group) => (
						<details key={group.groupId} className="rounded-lg border p-3">
							<summary className="cursor-pointer break-words text-sm font-medium">
								{group.name ?? group.groupId}
							</summary>
							<div className="mt-3">
								<BotUsageScope view={group} canEdit={canEdit} onSave={onSave} />
							</div>
						</details>
					))}
				</section>
			)}
		</div>
	);
}

function BotUsageContent({
	botId,
	groupId,
	canEdit,
}: Readonly<{
	botId: string;
	groupId?: string;
	canEdit: boolean;
}>) {
	const { t } = useTranslation("bots");
	const backend = useBackend();
	const accountId = useAuth().user?.profile.sub;
	const profileQuery = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
		!backend.profile,
	);
	const profile = backend.profile ?? profileQuery.data;
	const client = useQueryClient();
	const queryScope = ["monthly-usage", profile?.hub, profile?.id, accountId];
	const path = botUsagePath(botId, groupId);
	const query = useQuery({
		queryKey: [...queryScope, path],
		queryFn: () => {
			if (!profile) throw new Error("Profile not loaded");
			return backend.apiState.get<UsageBudgetView>(profile, path);
		},
		enabled: Boolean(profile),
		meta: { persist: false },
		refetchOnWindowFocus: false,
	});
	const save = async (view: UsageBudgetView, limits: MonthlyUsageLimits) => {
		if (!profile) throw new Error("Profile not loaded");
		await backend.apiState.put(
			profile,
			`${botUsagePath(view.subjectId, view.groupId)}/limits`,
			limits,
		);
		await client.invalidateQueries({ queryKey: queryScope });
		toast.success(t("usage.saved", "Monthly limits saved."));
	};
	if (query.isError || (!profile && profileQuery.isError)) {
		return (
			<div className="space-y-3">
				<p role="alert" className="text-sm text-destructive">
					{apiErrorMessage(
						query.error ?? profileQuery.error,
						t("usage.loadFailed", "Usage could not be loaded."),
					)}
				</p>
				<Button
					variant="outline"
					onClick={() =>
						void (profile ? query.refetch() : profileQuery.refetch())
					}
				>
					{t("crew.retry", "Try again")}
				</Button>
			</div>
		);
	}
	if (!query.data)
		return (
			<output className="text-sm text-muted-foreground">
				{t("usage.loading", "Loading usage…")}
			</output>
		);
	return (
		<div className="space-y-4">
			<div className="flex justify-end">
				<Button
					size="sm"
					variant="ghost"
					disabled={query.isFetching}
					onClick={() => void query.refetch()}
				>
					<RefreshCwIcon className="size-3.5" aria-hidden />
					{t("usage.refresh", "Refresh usage")}
				</Button>
			</div>
			<BotUsageScope view={query.data} canEdit={canEdit} onSave={save} />
		</div>
	);
}

export function BotUsageButton({
	botId,
	name,
	groupId,
	canEdit = true,
}: Readonly<{
	botId: string;
	name: string;
	groupId?: string;
	canEdit?: boolean;
}>) {
	const { t } = useTranslation("bots");
	const [open, setOpen] = useState(false);
	return (
		<>
			<Button
				type="button"
				variant={groupId ? "ghost" : "outline"}
				size={groupId ? "icon" : "sm"}
				aria-label={t("usage.action", "Usage & limits")}
				title={t("usage.action", "Usage & limits")}
				onClick={() => setOpen(true)}
			>
				<GaugeIcon aria-hidden />
				{!groupId && t("usage.action", "Usage & limits")}
			</Button>
			<Dialog open={open} onOpenChange={setOpen}>
				<DialogContent className="max-h-[85dvh] overflow-y-auto sm:max-w-2xl">
					<DialogHeader>
						<DialogTitle>
							{t("usage.title", "{{name}} usage", { name })}
						</DialogTitle>
						<DialogDescription>
							{groupId
								? t(
										"usage.roomDescription",
										"Model usage and monthly limits for this bot in this group. The bot's overall and enterprise limits also apply.",
									)
								: t(
										"usage.description",
										"Track model usage across this bot and its groups. You can lower its monthly limits. Higher enterprise ceilings require an administrator's approval.",
									)}
						</DialogDescription>
					</DialogHeader>
					{open && (
						<BotUsageContent
							botId={botId}
							groupId={groupId}
							canEdit={canEdit}
						/>
					)}
				</DialogContent>
			</Dialog>
		</>
	);
}
