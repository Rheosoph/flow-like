"use client";

import { useTranslation } from "@flow-like/locales";
import {
	PencilLineIcon,
	RefreshCwIcon,
	SquareDashedBottomCodeIcon,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import { useInvoke } from "../../../../hooks";
import { useBackend, useBackendReady } from "../../../../state/backend-state";
import { Button } from "../../../ui/button";
import {
	ContextMenu,
	ContextMenuContent,
	ContextMenuItem,
	ContextMenuTrigger,
} from "../../../ui/context-menu";
import {
	EmptyRow,
	NameField,
	SectionHeader,
	TreeRow,
} from "./explorer-primitives";

/**
 * The app's widgets.
 *
 * Widgets are indexed by `App.widget_ids` and carry no board relation at all — unlike
 * pages, which hang off `Board.page_ids`. The heading says so, because a board-scoped
 * tree that silently lists app-scoped things is the kind of ambiguity people build on.
 */
export function WidgetsRoot({
	appId,
	onOpenWidget,
	onWidgetName,
	readOnly,
	enabled = true,
}: Readonly<{
	appId: string;
	onOpenWidget: (widgetId: string) => void;
	/** Reports a widget's name so the tab strip can label it without fetching again. */
	onWidgetName?: (widgetId: string, name: string) => void;
	readOnly: boolean;
	enabled?: boolean;
}>) {
	const { t } = useTranslation("flow");
	const backend = useBackend();
	const ready = useBackendReady() && enabled && appId.length > 0;
	const [renaming, setRenaming] = useState<string | null>(null);

	const widgets = useInvoke(
		backend.widgetState.getWidgets,
		backend.widgetState,
		[appId],
		ready,
	);

	const commitRename = useCallback(
		async (widgetId: string, name: string) => {
			setRenaming(null);
			try {
				await backend.widgetState.renameWidget(appId, widgetId, name);
				await widgets.refetch();
			} catch (cause) {
				console.error("Failed to rename widget", widgetId, cause);
				toast.error(t("failedToRenameWidget", "Failed to rename widget"));
			}
		},
		[appId, backend.widgetState, t, widgets],
	);

	const rows = widgets.data ?? [];

	// Reported from an effect, not from the row: naming a tab is a write into the host's
	// state, and doing it while rendering is a render-phase update on another component.
	useEffect(() => {
		if (!onWidgetName) return;
		for (const [widgetId, fallbackName, metadata] of rows) {
			onWidgetName(
				widgetId,
				metadata?.name?.trim() || fallbackName || widgetId,
			);
		}
	}, [onWidgetName, rows]);

	return (
		<>
			<SectionHeader
				label={t("widgetsAppScoped", "Widgets (app)")}
				action={
					<Button
						size="icon"
						variant="ghost"
						className="size-5 text-muted-foreground"
						title={t("refresh", "Refresh")}
						aria-label={t("refresh", "Refresh")}
						onClick={() => void widgets.refetch()}
					>
						<RefreshCwIcon
							className={widgets.isFetching ? "size-3 animate-spin" : "size-3"}
						/>
					</Button>
				}
			/>
			{!widgets.isLoading && rows.length === 0 && (
				<EmptyRow label={t("noWidgetsYet", "No widgets yet")} />
			)}
			{rows.map(([widgetId, fallbackName, metadata]) => {
				const name = metadata?.name?.trim() || fallbackName || widgetId;
				if (renaming === widgetId) {
					return (
						<NameField
							key={widgetId}
							initial={name}
							depth={0}
							validate={(value) =>
								value.trim()
									? null
									: t("nameCannotBeEmpty", "Name cannot be empty")
							}
							onSubmit={(next) => void commitRename(widgetId, next)}
							onCancel={() => setRenaming(null)}
						/>
					);
				}
				return (
					<ContextMenu key={widgetId}>
						<ContextMenuTrigger asChild>
							<TreeRow
								depth={0}
								icon={<SquareDashedBottomCodeIcon />}
								label={name}
								onSelect={() => onOpenWidget(widgetId)}
							/>
						</ContextMenuTrigger>
						<ContextMenuContent className="w-56">
							<ContextMenuItem onSelect={() => onOpenWidget(widgetId)}>
								<SquareDashedBottomCodeIcon className="size-3.5" />
								{t("openInBuilder", "Open in Builder")}
							</ContextMenuItem>
							{!readOnly && (
								<ContextMenuItem onSelect={() => setRenaming(widgetId)}>
									<PencilLineIcon className="size-3.5" />
									{t("rename", "Rename")}
								</ContextMenuItem>
							)}
						</ContextMenuContent>
					</ContextMenu>
				);
			})}
		</>
	);
}
