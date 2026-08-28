"use client";

import { useTranslation } from "@flow-like/locales";
import { PageBuilderSurface } from "../../../builder/page-builder-surface";
import { WidgetBuilderSurface } from "../../../builder/widget-builder-surface";
import { TableInspector } from "../../../settings/explore/table-inspector";
import type { IEditorDocument, IEditorTab } from "../editor-documents";
import { StorageDocument } from "./storage-document";

/**
 * The editor area for anything that is not the graph.
 *
 * Only the active document is mounted. Two widget builders at once is a correctness bug —
 * the assistant surface is a single slot with identity-matched cleanup, so the second one
 * unmounting nulls it while the first is still live — and mounting every open tab would
 * hold a builder, a table page and a signed URL alive for tabs nobody is looking at. The
 * cost is that a builder reloads when you come back to it, which autosave already covers.
 */
export function EditorDocumentView({
	appId,
	boardId,
	tab,
	onClose,
	onDocumentChange,
}: Readonly<{
	appId: string;
	boardId: string;
	tab: IEditorTab;
	onClose: () => void;
	/** The document moved under the tab — the page builder has its own page switcher. */
	onDocumentChange: (key: string, doc: IEditorDocument) => void;
}>) {
	const { t } = useTranslation("flow");
	const { doc } = tab;

	switch (doc.kind) {
		case "storage":
			return (
				<StorageDocument
					appId={appId}
					scope={doc.scope}
					location={doc.location}
				/>
			);
		case "table":
			return (
				<TableInspector
					appId={appId}
					table={doc.table}
					userScoped={doc.scope === "user"}
					embedded
					className="h-full min-h-0"
				/>
			);
		// Both builders are keyed by their document: neither resets its loading flag or its
		// pending autosave timer when the id changes under a live instance, so reusing one
		// would briefly show the previous document and could flush its edits into this one.
		case "page":
			return (
				<PageBuilderSurface
					key={doc.pageId}
					appId={appId}
					pageId={doc.pageId}
					boardId={boardId}
					className="h-full min-h-0"
					onClose={onClose}
					onPageChange={(pageId) =>
						onDocumentChange(tab.key, { kind: "page", pageId })
					}
				/>
			);
		case "widget":
			return (
				<WidgetBuilderSurface
					key={doc.widgetId}
					appId={appId}
					widgetId={doc.widgetId}
					className="h-full min-h-0"
					onClose={onClose}
				/>
			);
		default:
			return (
				<div className="flex h-full items-center justify-center text-muted-foreground text-sm">
					{t(
						"thisDocumentCannotBeOpenedHere",
						"This document cannot be opened here",
					)}
				</div>
			);
	}
}
