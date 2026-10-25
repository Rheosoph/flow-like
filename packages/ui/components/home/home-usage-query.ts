import {
	type QueryClient,
	type QueryFunctionContext,
	type QueryKey,
	queryOptions,
} from "@tanstack/react-query";
import type { IUsageState } from "../../state/backend-state/usage-state";
import { homeActivityDays } from "./home-activity-statistics";

export const HOME_USAGE_REFRESH_MS = 5 * 60_000;

function usageSource(usage: IUsageState | undefined): IUsageState {
	if (!usage) throw new Error("Usage history is unavailable on this backend.");
	return usage;
}

function usageRequestOptions({
	client,
	queryKey,
}: Pick<QueryFunctionContext, "client" | "queryKey">) {
	const state = client.getQueryState(queryKey);
	return { fresh: !state?.dataUpdatedAt || state.isInvalidated };
}

export async function refreshHomeUsageQuery<T>(
	client: QueryClient,
	queryKey: QueryKey,
	refetch: () => Promise<T>,
): Promise<T> {
	await client.invalidateQueries({
		queryKey,
		exact: true,
		refetchType: "none",
	});
	return refetch();
}

function refreshOptions(editing: boolean) {
	return {
		staleTime: HOME_USAGE_REFRESH_MS,
		refetchInterval: editing ? (false as const) : HOME_USAGE_REFRESH_MS,
	};
}

export function homeRecentExecutionsQuery(
	usage: IUsageState | undefined,
	scope: readonly string[],
	appId: string,
	limit: number,
	editing = false,
) {
	return queryOptions({
		queryKey: ["home", ...scope, "recent-executions", appId, limit],
		queryFn: (context) =>
			usageSource(usage).getRecentExecutions(
				limit,
				appId || undefined,
				usageRequestOptions(context),
			),
		enabled: Boolean(usage),
		...refreshOptions(editing),
	});
}

export function homeExecutionActivityQuery(
	usage: IUsageState | undefined,
	scope: readonly string[],
	days: unknown,
	appId = "",
	editing = false,
) {
	const window = homeActivityDays(days);
	return queryOptions({
		queryKey: ["home", ...scope, "activity", appId, window],
		queryFn: (context) =>
			usageSource(usage).getExecutionActivity(
				window,
				appId || undefined,
				usageRequestOptions(context),
			),
		enabled: Boolean(usage),
		...refreshOptions(editing),
	});
}

export function homeUsageSummaryQuery(
	usage: IUsageState | undefined,
	scope: readonly string[],
	editing = false,
) {
	return queryOptions({
		queryKey: ["home", ...scope, "usage-summary"],
		queryFn: (context) =>
			usageSource(usage).getUsageSummary(usageRequestOptions(context)),
		enabled: Boolean(usage),
		...refreshOptions(editing),
	});
}
