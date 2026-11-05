"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { useInvoke } from "../../hooks/use-invoke";
import { apiErrorMessage } from "../../lib/api-error";
import type { UsageBudgetView } from "../../lib/monthly-usage";
import { useBackend } from "../../state/backend-state";
import { Button } from "../ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../ui/card";
import {
	MonthlyUsageLimitsEditor,
	UsageBudgetSummary,
} from "./monthly-usage-budget";

export function AppUsageBudget({
	appId,
	canEdit,
}: Readonly<{
	appId: string;
	canEdit: boolean;
}>) {
	const { t } = useTranslation("admin");
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
	const scope = ["monthly-usage", profile?.hub, profile?.id, accountId];
	const path = `apps/${encodeURIComponent(appId)}/usage/budget`;
	const query = useQuery({
		queryKey: [...scope, path],
		queryFn: () => {
			if (!profile) throw new Error("Profile not loaded");
			return backend.apiState.get<UsageBudgetView>(profile, path);
		},
		enabled: Boolean(profile),
		meta: { persist: false },
		refetchOnWindowFocus: false,
	});
	const failed = query.isError || (!profile && profileQuery.isError);
	return (
		<Card>
			<CardHeader>
				<CardTitle>
					{t("monthlyUsage.appOwnerTitle", "Monthly usage & limits")}
				</CardTitle>
				<CardDescription>
					{t(
						"monthlyUsage.appOwnerDescription",
						"Set a lower monthly budget for this app. Increasing its enterprise ceiling requires an administrator's approval.",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-4">
				<Button
					size="sm"
					variant="outline"
					disabled={query.isFetching}
					onClick={() =>
						void (profile ? query.refetch() : profileQuery.refetch())
					}
				>
					{t("monthlyUsage.refresh", "Refresh usage")}
				</Button>
				{failed && (
					<p role="alert" className="text-sm text-destructive">
						{apiErrorMessage(
							query.error ?? profileQuery.error,
							t("monthlyUsage.appFailed", "App usage could not be loaded."),
						)}
					</p>
				)}
				{!query.data && !failed && (
					<output className="block text-sm text-muted-foreground">
						{t("monthlyUsage.loadingApp", "Loading app usage…")}
					</output>
				)}
				{query.data && (
					<>
						<UsageBudgetSummary view={query.data} />
						{canEdit && (
							<MonthlyUsageLimitsEditor
								key={JSON.stringify(query.data.ownerLimits)}
								initial={query.data.ownerLimits}
								ceiling={query.data.adminLimits ?? query.data.defaultLimits}
								label={t(
									"monthlyUsage.appOwnerLimits",
									"Your app's monthly limits",
								)}
								onSave={async (limits) => {
									if (!profile) throw new Error("Profile not loaded");
									await backend.apiState.put(profile, `${path}/limits`, limits);
									await client.invalidateQueries({ queryKey: scope });
									toast.success(
										t("monthlyUsage.saved", "Monthly limits saved."),
									);
								}}
							/>
						)}
					</>
				)}
			</CardContent>
		</Card>
	);
}
