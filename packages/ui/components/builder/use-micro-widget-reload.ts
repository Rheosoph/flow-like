"use client";

import { useTranslation } from "@flow-like/locales";
import { useQueryClient } from "@tanstack/react-query";
import { useCallback, useMemo, useRef } from "react";
import { toast } from "sonner";
import {
	fetchAppPackageWidgets,
	invalidateAppPackageWidgets,
	useAppPackageWidgets,
} from "../../hooks/use-app-package-widgets";
import { useBackend } from "../../state/backend-state";
import {
	type MicroWidgetReloadReport,
	type MicroWidgetReloader,
	findMicroWidgetUpdate,
	reloadMicroWidgetInstance,
} from "../a2ui/micro-widget-reload";
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
	const { actionContext, getComponent, updateComponent, pushHistory } =
		useBuilder();
	const appId = actionContext?.appId;
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
			if (!appId) return;
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

	return useMemo(
		() => ({ updateFor, reload, refresh }),
		[updateFor, reload, refresh],
	);
}
