"use client";

import { useTranslation } from "@flow-like/locales";
import {
	useIsMutating,
	useMutation,
	useQuery,
	useQueryClient,
} from "@tanstack/react-query";
import { useCallback, useEffect, useRef } from "react";
import { toast } from "sonner";
import { fetchAppPackageWidgets } from "../../../hooks/use-app-package-widgets";
import type { AppPackageWidget } from "../../../lib/package-widgets";
import { waitForPendingPageSaves } from "../../../lib/pending-page-saves";
import { useBackend } from "../../../state/backend-state";
import { reloadProjectMicroWidgets } from "../../a2ui/reload-project-micro-widgets";
import { scanProjectMicroWidgets } from "../../a2ui/scan-project-micro-widgets";
import { describeMicroWidgetReload } from "../../builder/use-micro-widget-reload";
import {
	APP_PACKAGE_MANIFEST_KEY,
	invalidateAppPackageQueries,
} from "./use-package-manifests";

interface PackageWidgetUpdateOptions {
	appId: string;
	widgets: readonly AppPackageWidget[];
	latestVersions: ReadonlyMap<string, string>;
	enabled: boolean;
	/** Apply an available release before resolving the package's new widget contract. */
	preparePackage: (
		packageId: string,
		shouldContinue: () => boolean,
	) => Promise<string | undefined>;
}

export function usePackageWidgetUpdates({
	appId,
	widgets,
	latestVersions,
	enabled,
	preparePackage,
}: PackageWidgetUpdateOptions) {
	const { t } = useTranslation("store");
	const { t: flowT } = useTranslation("flow");
	const backend = useBackend();
	const queryClient = useQueryClient();
	const mounted = useRef(true);
	const activeAppId = useRef(appId);
	activeAppId.current = appId;
	useEffect(() => {
		mounted.current = true;
		return () => {
			mounted.current = false;
		};
	}, []);
	const mutationKey = ["reload-project-micro-widgets", appId];
	const isUpdating = useIsMutating({ mutationKey }) > 0;
	const usageKey = ["app-package-widget-usage", appId];
	const scan = useQuery({
		queryKey: [
			...usageKey,
			widgets.map(({ packageId, packageVersion, bundleHash, widget }) => [
				packageId,
				packageVersion,
				bundleHash,
				widget.id,
			]),
			Array.from(latestVersions),
		],
		queryFn: async () => {
			await waitForPendingPageSaves(backend.pageState, appId);
			return scanProjectMicroWidgets(
				backend.pageState,
				appId,
				widgets,
				latestVersions,
			);
		},
		enabled: enabled && widgets.length > 0 && !isUpdating,
		staleTime: 30_000,
		refetchOnMount: "always",
		retry: false,
		meta: { persist: false },
	});

	const mutation = useMutation<void, Error, string>({
		mutationKey,
		mutationFn: async (packageId) => {
			const isCurrentView = () =>
				mounted.current && activeAppId.current === appId;
			await queryClient.cancelQueries({
				queryKey: ["app-package-widget-usage", appId],
			});
			await waitForPendingPageSaves(backend.pageState, appId);
			if (!isCurrentView()) return;
			const version = await preparePackage(packageId, isCurrentView);
			// Invalidate the desktop's cached project pins before resolving the new manifest.
			await queryClient.invalidateQueries({
				queryKey: ["app", appId, "packages"],
			});
			if (!isCurrentView()) return;
			const installed = await fetchAppPackageWidgets(
				queryClient,
				backend,
				appId,
				{ strict: true, packageId },
			);
			if (installed.length === 0)
				throw new Error(
					t(
						"appPackageWidgetNoContract",
						"This package version has no widgets available to update.",
					),
				);
			if (
				version &&
				installed.some((widget) => widget.packageVersion !== version)
			) {
				throw new Error(
					t(
						"appPackageWidgetVersionNotReady",
						"Package version {{version}} could not be loaded. Retry the widget update.",
						{ version },
					),
				);
			}
			const result = await reloadProjectMicroWidgets(
				backend.pageState,
				appId,
				installed,
				{ shouldContinue: isCurrentView },
			);
			await Promise.all([
				...[
					backend.pageState.getPage,
					backend.pageState.getPages,
					backend.pageState.getPageBootstrap,
				].map((method) =>
					queryClient.invalidateQueries({
						queryKey: [method.name || "backendFn", appId],
					}),
				),
				...result.boardIds.map((boardId) =>
					queryClient.invalidateQueries({
						queryKey: [
							backend.boardState.getBoard.name || "backendFn",
							appId,
							boardId,
						],
					}),
				),
			]);
			const changes = describeMicroWidgetReload(result.report, flowT);
			const summary = t(
				"appPackageWidgetUpdated",
				"Updated {{widgets}} widgets on {{pages}} pages.",
				{ widgets: result.count, pages: result.pages },
			);
			if (result.failures.length > 0) {
				toast.warning(summary, {
					description: [
						...changes,
						t(
							"appPackageWidgetPartial",
							"Some pages could not be updated: {{pages}}",
							{
								pages: result.failures
									.map(
										({ name, error }) =>
											`${name} (${error instanceof Error ? error.message : String(error)})`,
									)
									.join(", "),
							},
						),
					].join("\n"),
					classNames: { description: "whitespace-pre-line" },
					action: isCurrentView()
						? {
								label: t("retry", "Retry"),
								onClick: () => void update(packageId),
							}
						: undefined,
				});
			} else if (result.count === 0) {
				toast.info(
					t(
						"appPackageWidgetAlreadyCurrent",
						"All widgets of this package are up to date.",
					),
				);
			} else {
				toast.success(summary, {
					description: changes.join("\n") || undefined,
					classNames: { description: "whitespace-pre-line" },
				});
			}
		},
		onSettled: () => {
			invalidateAppPackageQueries(queryClient, appId);
		},
	});
	const update: (packageId: string) => Promise<void> = useCallback(
		async (packageId) => {
			if (
				queryClient.isMutating({
					mutationKey: ["reload-project-micro-widgets", appId],
				})
			)
				return;
			try {
				await mutation.mutateAsync(packageId);
			} catch (error) {
				toast.error(
					t(
						"appPackageWidgetUpdateFailed",
						"Could not update this package's widgets: {{detail}}",
						{
							detail: error instanceof Error ? error.message : String(error),
						},
					),
				);
			}
		},
		[appId, queryClient, mutation.mutateAsync, t],
	);

	const check = useCallback(async () => {
		await queryClient.invalidateQueries({
			queryKey: [APP_PACKAGE_MANIFEST_KEY, appId],
		});
		await queryClient.invalidateQueries({
			queryKey: ["app", appId, "package-updates"],
		});
		await queryClient.invalidateQueries({
			queryKey: ["app-package-widget-usage", appId],
		});
	}, [appId, queryClient]);

	return {
		byPackage: scan.data?.byPackage,
		isChecking: !enabled || scan.isFetching,
		checkFailed: scan.isError || (scan.data?.failures.length ?? 0) > 0,
		isUpdating,
		updatingPackageId: mutation.isPending ? mutation.variables : undefined,
		update,
		check,
	};
}
