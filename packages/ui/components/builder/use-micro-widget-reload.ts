"use client";

import { useTranslation } from "@flow-like/locales";
import {
	useIsMutating,
	useMutation,
	useQueryClient,
} from "@tanstack/react-query";
import { useCallback, useEffect, useMemo, useRef } from "react";
import { toast } from "sonner";
import {
	fetchAppPackageWidgets,
	invalidateAppPackageWidgets,
	useAppPackageWidgets,
} from "../../hooks/use-app-package-widgets";
import { useInvalidateInvoke } from "../../hooks/use-invoke";
import { useBackend } from "../../state/backend-state";
import {
	reloadMicroWidgetsInPage,
	unavailableMicroWidgetsError,
} from "../a2ui/micro-widget-project-reload";
import {
	type MicroWidgetReloadReport,
	type MicroWidgetReloader,
	findMicroWidgetUpdate,
	reloadMicroWidgetInstance,
} from "../a2ui/micro-widget-reload";
import { reloadProjectMicroWidgets } from "../a2ui/reload-project-micro-widgets";
import type {
	A2UIComponent,
	MicroWidgetInstanceComponent,
} from "../a2ui/types";
import { useBuilder } from "./BuilderContext";

type Translate = ReturnType<typeof useTranslation>["t"];

export function describeMicroWidgetReload(
	report: MicroWidgetReloadReport,
	t: Translate,
): string[] {
	const lines: string[] = [];
	const add = (keys: string[], line: (keys: string) => string) => {
		if (keys.length > 0) lines.push(line(keys.join(", ")));
	};
	add(report.converted, (keys) =>
		t("widgetReloadConverted", "Converted to the new input type: {{keys}}", {
			keys,
		}),
	);
	add(report.defaulted, (keys) =>
		t("widgetReloadDefaulted", "Now using the new default: {{keys}}", {
			keys,
		}),
	);
	add(report.reset, (keys) =>
		t("widgetReloadReset", "Reset because the value no longer fits: {{keys}}", {
			keys,
		}),
	);
	add(report.removed, (keys) =>
		t("widgetReloadRemoved", "Removed, no longer declared: {{keys}}", {
			keys,
		}),
	);
	add(report.undeclaredEvents, (keys) =>
		t(
			"widgetReloadUndeclaredEvents",
			"Handlers kept for events the widget no longer declares: {{keys}}",
			{ keys },
		),
	);
	return lines;
}

/**
 * Moves placed package widgets onto the installed build of their package.
 * The reload is one undo step; the page save that follows re-types the
 * Widget Action Event pins from the new contract.
 */
export function useMicroWidgetReload(): MicroWidgetReloader {
	const { t } = useTranslation("flow");
	const backend = useBackend();
	const queryClient = useQueryClient();
	const invalidate = useInvalidateInvoke();
	const builder = useBuilder();
	const { actionContext, getComponent, updateComponent, pushHistory } = builder;
	const appId = actionContext?.appId;
	const latestBuilder = useRef(builder);
	latestBuilder.current = builder;
	const mounted = useRef(true);
	useEffect(() => {
		mounted.current = true;
		return () => {
			mounted.current = false;
		};
	}, []);
	const mutationKey = ["reload-project-micro-widgets", appId];
	const isReloadingAll = useIsMutating({ mutationKey }) > 0;
	const { data: installed } = useAppPackageWidgets(appId, { staleTime: 0 });
	const latestGetComponent = useRef(getComponent);
	latestGetComponent.current = getComponent;

	const updateFor = useCallback<MicroWidgetReloader["updateFor"]>(
		(component) => findMicroWidgetUpdate(installed, component),
		[installed],
	);

	const refresh = useCallback(() => {
		if (appId) void invalidateAppPackageWidgets(queryClient, appId);
	}, [queryClient, appId]);

	const reload = useCallback(
		async (componentId: string) => {
			if (
				!appId ||
				queryClient.isMutating({
					mutationKey: ["reload-project-micro-widgets", appId],
				})
			)
				return;
			try {
				const fresh = await fetchAppPackageWidgets(queryClient, backend, appId);
				const surfaceComponent = latestGetComponent.current(componentId);
				if (surfaceComponent?.component.type !== "microWidgetInstance") return;
				const placed =
					surfaceComponent.component as unknown as MicroWidgetInstanceComponent;
				const update = findMicroWidgetUpdate(fresh, placed);
				if (!update) {
					toast.info(
						t(
							"widgetAlreadyUsesTheInstalledBuild",
							"This widget already uses the installed build.",
						),
					);
					return;
				}
				const { component, report } = reloadMicroWidgetInstance(placed, update);
				pushHistory();
				updateComponent(componentId, {
					component: component as unknown as A2UIComponent,
				});
				const changes = describeMicroWidgetReload(report, t);
				toast.success(t("widgetReloaded", "Widget reloaded"), {
					description:
						changes.length > 0
							? changes.join("\n")
							: t(
									"allSettingsAndActionsCarriedOver",
									"All settings and actions carried over.",
								),
					classNames: { description: "whitespace-pre-line" },
				});
			} catch (error) {
				toast.error(
					t("widgetReloadFailed", "Could not reload the widget: {{detail}}", {
						detail: error instanceof Error ? error.message : String(error),
					}),
				);
			}
		},
		[appId, queryClient, backend, pushHistory, updateComponent, t],
	);

	const { mutateAsync } = useMutation<void, Error, void>({
		mutationKey,
		mutationFn: async () => {
			const context = latestBuilder.current.actionContext;
			if (!appId || !context?.pageId || !context.saveWidgetUpdates) return;
			const fresh = await fetchAppPackageWidgets(queryClient, backend, appId, {
				strict: true,
			});
			const isCurrentPage = () =>
				mounted.current &&
				latestBuilder.current.actionContext?.appId === appId &&
				latestBuilder.current.actionContext?.pageId === context.pageId;
			const result = await reloadProjectMicroWidgets(
				backend.pageState,
				appId,
				fresh,
				{
					skipPageId: context.pageId,
					shouldContinue: isCurrentPage,
				},
			);
			const current = latestBuilder.current;
			try {
				if (!isCurrentPage()) {
					throw new Error(
						t(
							"widgetProjectPageChanged",
							"The open page changed during the update. Run Update all again.",
						),
					);
				}
				const snapshot = {
					components: Array.from(current.components.values()),
					widgetRefs: Object.fromEntries(current.widgetRefs),
					content: current.actionContext?.pageContent ?? [],
				};
				const updated = reloadMicroWidgetsInPage(snapshot, fresh);
				if (updated.count > 0) {
					// One undo step, based on the live editor after the other pages finished saving.
					current.replaceComponents(updated.page.components);
					for (const [id, widget] of Object.entries(updated.page.widgetRefs)) {
						if (widget !== snapshot.widgetRefs[id])
							current.addWidgetRef(id, widget);
					}
				}
				// A retry must also save a previously migrated working copy whose write failed.
				await context.saveWidgetUpdates(updated.page, updated.page.content);
				if (updated.count > 0) {
					result.count += updated.count;
					result.pages += 1;
					for (const key of Object.keys(
						result.report,
					) as (keyof MicroWidgetReloadReport)[]) {
						result.report[key] = [
							...new Set([...result.report[key], ...updated.report[key]]),
						];
					}
				}
				if (updated.unavailable.length)
					throw unavailableMicroWidgetsError(updated.unavailable);
			} catch (error) {
				result.failures.push({
					pageId: context.pageId,
					name: context.pageId,
					error,
				});
			}

			await Promise.all([
				queryClient.invalidateQueries({
					queryKey: [backend.pageState.getPage.name || "backendFn", appId],
				}),
				invalidate(backend.pageState.getPages, [appId]),
				invalidate(backend.pageState.getPageBootstrap, [appId]),
				...result.boardIds.map((boardId) =>
					invalidate(backend.boardState.getBoard, [appId, boardId]),
				),
			]);
			const summary = t(
				"widgetProjectReloaded",
				"Updated {{widgets}} widgets on {{pages}} pages.",
				{
					widgets: result.count,
					pages: result.pages,
				},
			);
			const changes = describeMicroWidgetReload(result.report, t);
			if (result.failures.length > 0) {
				toast.warning(summary, {
					description: [
						...changes,
						t(
							"widgetProjectReloadPartial",
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
					action: isCurrentPage()
						? { label: t("retry", "Retry"), onClick: () => void reloadAll() }
						: undefined,
				});
			} else if (result.count === 0) {
				toast.info(
					t(
						"widgetProjectAlreadyCurrent",
						"All package widgets in this project use the installed builds.",
					),
				);
			} else {
				toast.success(summary, {
					description: changes.join("\n") || undefined,
					classNames: { description: "whitespace-pre-line" },
				});
			}
		},
	});

	const reloadAll: () => Promise<void> = useCallback(async () => {
		if (
			queryClient.isMutating({
				mutationKey: ["reload-project-micro-widgets", appId],
			})
		)
			return;
		try {
			await mutateAsync();
		} catch (error) {
			toast.error(
				t(
					"widgetProjectReloadFailed",
					"Could not update project widgets: {{detail}}",
					{
						detail: error instanceof Error ? error.message : String(error),
					},
				),
			);
		}
	}, [queryClient, appId, mutateAsync, t]);
	const canReloadAll = Boolean(
		appId && actionContext?.pageId && actionContext.saveWidgetUpdates,
	);

	return useMemo(
		() => ({
			updateFor,
			reload,
			refresh,
			reloadAll: canReloadAll ? reloadAll : undefined,
			isReloadingAll,
		}),
		[updateFor, reload, refresh, canReloadAll, reloadAll, isReloadingAll],
	);
}
