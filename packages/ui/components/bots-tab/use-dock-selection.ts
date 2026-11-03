"use client";

import { usePathname, useSearchParams } from "next/navigation";
import { useCallback, useMemo, useState } from "react";
import { useClientRouter } from "../../lib/client-navigation";
import type { BotPanelTab } from "./bot-panel-tabs";
import {
	type DockSelection,
	dockPanelQuery,
	dockSelectionKey,
	dockSelectionQuery,
	parseDockPanel,
	parseDockSelection,
} from "./dock-selection";

interface DockView {
	selection: DockSelection;
	panel: BotPanelTab | null;
}

function dockViewKey(view: DockView): string {
	return `${dockSelectionKey(view.selection)}|${view.panel ?? ""}`;
}

/**
 * What the dock shows, mirrored into `?bot=` / `?conversation=` and `?panel=` so a reload or a
 * shared link lands in the same place. The URL wins whenever it changes from outside.
 */
export function useDockSelection() {
	const router = useClientRouter();
	const pathname = usePathname();
	const params = useSearchParams();
	const fromUrl = useMemo<DockView>(
		() => ({
			selection: parseDockSelection(params),
			panel: parseDockPanel(params),
		}),
		[params],
	);
	const urlKey = dockViewKey(fromUrl);

	const [view, setView] = useState<DockView>(fromUrl);
	const [dockOpen, setDockOpen] = useState(true);
	const [seenUrlKey, setSeenUrlKey] = useState(urlKey);
	if (urlKey !== seenUrlKey) {
		setSeenUrlKey(urlKey);
		setView(fromUrl);
		setDockOpen(true);
	}

	const navigate = useCallback(
		(query: string) =>
			router.replace(query ? `${pathname}?${query}` : pathname, {
				scroll: false,
			}),
		[pathname, router],
	);

	const select = useCallback(
		(next: DockSelection) => {
			setView((current) => ({
				selection: next,
				panel:
					dockSelectionKey(current.selection) === dockSelectionKey(next)
						? current.panel
						: null,
			}));
			setDockOpen(true);
			// The live URL, not the rendered params: the page drops ?voice= with history.replaceState
			// in the same click that can queue a draft, and the reveal selects before params catch up.
			navigate(dockSelectionQuery(next, window.location.search));
		},
		[navigate],
	);

	const selectPanel = useCallback(
		(panel: BotPanelTab | null) => {
			setView((current) => ({ ...current, panel }));
			setDockOpen(true);
			navigate(dockPanelQuery(panel, window.location.search));
		},
		[navigate],
	);

	const closeDock = useCallback(() => setDockOpen(false), []);
	const openDock = useCallback(() => setDockOpen(true), []);

	return {
		selection: view.selection,
		panel: view.panel,
		dockOpen,
		select,
		selectPanel,
		closeDock,
		openDock,
	};
}
