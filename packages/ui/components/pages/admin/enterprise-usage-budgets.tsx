"use client";

import { useTranslation } from "@flow-like/locales";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { toast } from "sonner";
import { apiErrorMessage } from "../../../lib/api-error";
import {
	type BotUsageOverview,
	type EnterpriseUsageDefaults,
	type MonthlyUsageLimits,
	type UsageBudgetView,
	adminUsageLimitPath,
} from "../../../lib/monthly-usage";
import { useBackend } from "../../../state/backend-state";
import type { IProfile } from "../../../types";
import {
	MonthlyUsageLimitsEditor,
	UsageBudgetSummary,
} from "../../settings/monthly-usage-budget";
import { Button } from "../../ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../../ui/card";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";

function AdminBudget({
	view,
	onSave,
}: Readonly<{
	view: UsageBudgetView;
	onSave: (
		view: UsageBudgetView,
		limits: MonthlyUsageLimits | null,
	) => Promise<void>;
}>) {
	const { t } = useTranslation("admin");
	const [resetting, setResetting] = useState(false);
	const [error, setError] = useState<string | null>(null);
	return (
		<div className="space-y-4">
			<UsageBudgetSummary view={view} />
			{!view.archived && (
				<details className="rounded-lg border p-3">
					<summary className="cursor-pointer text-sm font-medium">
						{t(
							"monthlyUsage.approve",
							"Approve a different enterprise ceiling",
						)}
					</summary>
					<div className="mt-3 space-y-3">
						<p className="text-xs text-muted-foreground">
							{t(
								"monthlyUsage.overrideHint",
								"This approval replaces the enterprise default for this scope. Lower limits chosen by its owner still apply.",
							)}
						</p>
						<MonthlyUsageLimitsEditor
							key={JSON.stringify(view.adminLimits ?? view.defaultLimits)}
							initial={view.adminLimits ?? view.defaultLimits}
							inherit={false}
							onSave={(limits) => onSave(view, limits)}
						/>
						{view.adminLimits && (
							<Button
								variant="outline"
								size="sm"
								disabled={resetting}
								onClick={async () => {
									setResetting(true);
									setError(null);
									try {
										await onSave(view, null);
									} catch (failure) {
										setError(
											apiErrorMessage(
												failure,
												t(
													"monthlyUsage.resetFailed",
													"The default could not be restored.",
												),
											),
										);
									} finally {
										setResetting(false);
									}
								}}
							>
								{t("monthlyUsage.reset", "Use enterprise default")}
							</Button>
						)}
						{error && (
							<p role="alert" className="text-sm text-destructive">
								{error}
							</p>
						)}
					</div>
				</details>
			)}
			{view.groups.length > 0 && (
				<div className="space-y-2">
					<h4 className="text-sm font-medium">
						{t("monthlyUsage.groups", "Groups")}
					</h4>
					{view.groups.map((group) => (
						<details key={group.groupId} className="rounded-lg border p-3">
							<summary className="cursor-pointer break-words text-sm font-medium">
								{group.name ?? group.groupId}
							</summary>
							<div className="mt-3">
								<AdminBudget view={group} onSave={onSave} />
							</div>
						</details>
					))}
				</div>
			)}
		</div>
	);
}

const PAGE_SIZE = 20;

export function EnterpriseUsageBudgets({
	profile,
	accountId,
}: Readonly<{
	profile: IProfile;
	accountId?: string;
}>) {
	const { t } = useTranslation("admin");
	const backend = useBackend();
	const client = useQueryClient();
	const scope = ["monthly-usage", profile.hub, profile.id, accountId];
	const [offset, setOffset] = useState(0);
	const [appDraft, setAppDraft] = useState("");
	const [appId, setAppId] = useState("");
	const defaults = useQuery({
		queryKey: [...scope, "defaults"],
		queryFn: () =>
			backend.apiState.get<EnterpriseUsageDefaults>(
				profile,
				"admin/usage/defaults",
			),
		meta: { persist: false, adminDashboard: true },
		refetchOnWindowFocus: false,
	});
	const bots = useQuery({
		queryKey: [...scope, "admin-bots", offset],
		queryFn: () =>
			backend.apiState.get<BotUsageOverview>(
				profile,
				`admin/usage/bots?limit=${PAGE_SIZE}&offset=${offset}`,
			),
		meta: { persist: false, adminDashboard: true },
		refetchOnWindowFocus: false,
	});
	const app = useQuery({
		queryKey: [...scope, "admin-app", appId],
		queryFn: () =>
			backend.apiState.get<UsageBudgetView>(
				profile,
				`admin/usage/apps/${encodeURIComponent(appId)}/budget`,
			),
		enabled: Boolean(appId),
		meta: { persist: false, adminDashboard: true },
		refetchOnWindowFocus: false,
	});
	const saveDefault = useMutation({
		scope: { id: JSON.stringify([...scope, "defaults"]) },
		mutationFn: async ({
			kind,
			limits,
		}: { kind: keyof EnterpriseUsageDefaults; limits: MonthlyUsageLimits }) => {
			const current = await backend.apiState.get<EnterpriseUsageDefaults>(
				profile,
				"admin/usage/defaults",
			);
			await backend.apiState.put(profile, "admin/usage/defaults", {
				...current,
				[kind]: limits,
			});
			await client.invalidateQueries({ queryKey: scope });
			toast.success(
				t("monthlyUsage.defaultsSaved", "Enterprise defaults saved."),
			);
		},
	});
	const saveOverride = async (
		view: UsageBudgetView,
		limits: MonthlyUsageLimits | null,
	) => {
		await backend.apiState.fetch(profile, adminUsageLimitPath(view), {
			method: "PUT",
			body: JSON.stringify(limits),
		});
		await client.invalidateQueries({ queryKey: scope });
		toast.success(
			t("monthlyUsage.approvalSaved", "Enterprise ceiling updated."),
		);
	};
	return (
		<div className="space-y-4">
			<Card>
				<CardHeader>
					<CardTitle>
						{t("monthlyUsage.defaultsTitle", "Enterprise monthly limits")}
					</CardTitle>
					<CardDescription>
						{t(
							"monthlyUsage.defaultsDescription",
							"Default ceilings apply to every app and bot. Owners can set lower limits; only enterprise administrators can approve higher ceilings. Limits reset on the first day of each calendar month in UTC.",
						)}
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-5">
					{defaults.isPending && (
						<output>
							{t("monthlyUsage.loading", "Loading monthly limits…")}
						</output>
					)}
					{defaults.isError && (
						<div className="space-y-2">
							<p role="alert" className="text-sm text-destructive">
								{apiErrorMessage(
									defaults.error,
									t(
										"monthlyUsage.loadFailed",
										"Monthly limits could not be loaded.",
									),
								)}
							</p>
							<Button variant="outline" onClick={() => void defaults.refetch()}>
								{t("monthlyUsage.retry", "Try again")}
							</Button>
						</div>
					)}
					{defaults.data &&
						(["apps", "bots"] as const).map((kind) => (
							<MonthlyUsageLimitsEditor
								key={`${kind}:${JSON.stringify(defaults.data[kind])}`}
								label={
									kind === "apps"
										? t("monthlyUsage.eachApp", "Default for each app")
										: t("monthlyUsage.eachBot", "Default for each bot")
								}
								initial={defaults.data[kind]}
								inherit={false}
								onSave={(limits) => saveDefault.mutateAsync({ kind, limits })}
							/>
						))}
				</CardContent>
			</Card>
			<Card>
				<CardHeader>
					<CardTitle>
						{t("monthlyUsage.botsTitle", "Bot and group usage")}
					</CardTitle>
					<CardDescription>
						{t(
							"monthlyUsage.botsDescription",
							"Review each bot's usage, model tokens, and group budgets. Group usage is included in the bot total.",
						)}
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-3">
					<Button
						variant="outline"
						size="sm"
						disabled={bots.isFetching}
						onClick={() => void bots.refetch()}
					>
						{t("monthlyUsage.refresh", "Refresh usage")}
					</Button>
					{bots.isPending && (
						<output>
							{t("monthlyUsage.loadingBots", "Loading bot usage…")}
						</output>
					)}
					{bots.isError && (
						<p role="alert" className="text-sm text-destructive">
							{apiErrorMessage(
								bots.error,
								t("monthlyUsage.botsFailed", "Bot usage could not be loaded."),
							)}
						</p>
					)}
					{bots.data?.items.length === 0 && (
						<p className="text-sm text-muted-foreground">
							{t("monthlyUsage.noBots", "No bots in this page.")}
						</p>
					)}
					{bots.data?.items.map((view) => (
						<details key={view.subjectId} className="rounded-lg border p-3">
							<summary className="cursor-pointer text-sm font-medium">
								{view.name ?? view.subjectId}
								{view.archived && (
									<span className="ml-2 rounded bg-muted px-2 py-0.5 text-xs font-normal">
										{t("monthlyUsage.deletedBot", "Deleted bot")}
									</span>
								)}
								<span className="ml-2 text-xs font-normal text-muted-foreground">
									{view.subjectId}
								</span>
							</summary>
							<div className="mt-3">
								<AdminBudget view={view} onSave={saveOverride} />
							</div>
						</details>
					))}
					{bots.data && (
						<div className="flex flex-wrap items-center justify-between gap-2 text-sm">
							<span>
								{t("monthlyUsage.botCount", "{{count}} bots", {
									count: bots.data.total,
								})}
							</span>
							<div className="flex gap-2">
								<Button
									size="sm"
									variant="outline"
									disabled={offset === 0 || bots.isFetching}
									onClick={() =>
										setOffset((value) => Math.max(0, value - PAGE_SIZE))
									}
								>
									{t("monthlyUsage.previous", "Previous")}
								</Button>
								<Button
									size="sm"
									variant="outline"
									disabled={
										offset + PAGE_SIZE >= bots.data.total || bots.isFetching
									}
									onClick={() => setOffset((value) => value + PAGE_SIZE)}
								>
									{t("monthlyUsage.next", "Next")}
								</Button>
							</div>
						</div>
					)}
				</CardContent>
			</Card>
			<Card>
				<CardHeader>
					<CardTitle>
						{t("monthlyUsage.appTitle", "App enterprise ceiling")}
					</CardTitle>
					<CardDescription>
						{t(
							"monthlyUsage.appDescription",
							"Look up an app to review its monthly budget and approve an exception to the default.",
						)}
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-4">
					<form
						className="flex flex-wrap items-end gap-2"
						onSubmit={(event) => {
							event.preventDefault();
							setAppId(appDraft.trim());
						}}
					>
						<div className="min-w-0 flex-1 space-y-1.5">
							<Label htmlFor="monthly-usage-app-id">
								{t("monthlyUsage.appId", "App ID")}
							</Label>
							<Input
								id="monthly-usage-app-id"
								value={appDraft}
								onChange={(event) => setAppDraft(event.target.value)}
							/>
						</div>
						<Button type="submit" disabled={!appDraft.trim() || app.isFetching}>
							{t("monthlyUsage.loadApp", "Load app")}
						</Button>
					</form>
					{appId && app.isPending && (
						<output>
							{t("monthlyUsage.loadingApp", "Loading app usage…")}
						</output>
					)}
					{app.isError && (
						<p role="alert" className="text-sm text-destructive">
							{apiErrorMessage(
								app.error,
								t("monthlyUsage.appFailed", "App usage could not be loaded."),
							)}
						</p>
					)}
					{app.data && <AdminBudget view={app.data} onSave={saveOverride} />}
				</CardContent>
			</Card>
		</div>
	);
}
