"use client";
import { useQueries } from "@tanstack/react-query";
import { useAuth } from "react-oidc-context";
import { useBackend } from "../../../state/backend-state";

export function useUsageNames(
	rows: { appId?: string | null; modelId?: string | null }[],
) {
	const backend = useBackend();
	const auth = useAuth();
	const apps = [
		...new Set(rows.flatMap((row) => (row.appId ? [row.appId] : []))),
	].slice(0, 20);
	const models = [
		...new Set(rows.flatMap((row) => (row.modelId ? [row.modelId] : []))),
	].slice(0, 20);
	const appQueries = useQueries({
		queries: apps.map((id) => ({
			queryKey: ["usage-app-name", auth.user?.profile.sub, id],
			queryFn: () => backend.appState.getAppMeta(id),
			staleTime: 600_000,
			retry: false,
		})),
	});
	const modelQueries = useQueries({
		queries: models.map((id) => ({
			queryKey: ["usage-model-name", auth.user?.profile.sub, id],
			queryFn: () => backend.bitState.getBit(id),
			staleTime: 600_000,
			retry: false,
		})),
	});
	return {
		appName: (id?: string | null) =>
			id ? (appQueries[apps.indexOf(id)]?.data?.name ?? id) : "Standalone AI",
		modelName: (id?: string | null) => {
			if (!id) return "Workflow";
			const meta = modelQueries[models.indexOf(id)]?.data?.meta;
			return (
				meta?.en?.name ??
				(meta ? Object.values(meta)[0]?.name : undefined) ??
				id
			);
		},
	};
}

export function usageFundingLabel(value: string): string {
	return (
		(
			{
				hosted: "Flow-Like allowance",
				cloud: "Cloud runtime",
				byok: "Your provider",
				customer: "Your provider",
				local: "Local · free",
				none: "No hosted AI",
			} as Record<string, string>
		)[value] ?? (value ?? "").replaceAll("_", " ")
	);
}

export function usageStatusLabel(value: string): string {
	return (
		(
			{
				completed: "Completed",
				pending: "Pending",
				settled: "Completed",
				running: "In progress",
				reserved: "Reserved",
				unknown: "Awaiting final usage",
				unknown_usage: "Awaiting final usage",
				failed: "Failed",
				cancelled: "Cancelled",
				released: "Released",
				expired: "Expired",
			} as Record<string, string>
		)[value] ?? (value ?? "").replaceAll("_", " ")
	);
}
