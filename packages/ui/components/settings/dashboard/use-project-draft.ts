"use client";

import { isEqual } from "lodash-es";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import type { IApp, IMetadata } from "../../../lib";
import { useBackend } from "../../../state/backend-state";
import type { InspectorPanel } from "./use-project-signals";

/**
 * Which draft fields each inspector panel owns. Saving a panel writes exactly
 * these fields on top of the server's current copy, so one panel can never
 * silently commit another panel's edits — the behaviour today's single
 * "Unsaved changes" bar cannot offer.
 */
const APP_FIELDS: Partial<Record<InspectorPanel, (keyof IApp)[]>> = {
	identity: ["app_type"],
	listing: ["primary_category", "secondary_category"],
	release: ["status", "version", "price", "changelog"],
};

const METADATA_FIELDS: Partial<Record<InspectorPanel, (keyof IMetadata)[]>> = {
	identity: ["name", "description", "long_description"],
	listing: ["tags", "website", "docs_url", "support_url"],
};

const EDITABLE_PANELS: InspectorPanel[] = ["identity", "listing", "release"];

export interface ProjectDraft {
	draftApp: IApp | undefined;
	draftMetadata: IMetadata | undefined;
	setDraftApp: (next: IApp) => void;
	setDraftMetadata: (next: IMetadata) => void;
	isPanelDirty: (panel: InspectorPanel) => boolean;
	dirtyPanels: InspectorPanel[];
	savePanel: (panel: InspectorPanel) => Promise<void>;
	resetPanel: (panel: InspectorPanel) => void;
	isSaving: boolean;
}

export function useProjectDraft(
	appId: string | undefined,
	app: IApp | undefined,
	metadata: IMetadata | undefined,
	onSaved: () => Promise<void> | void,
): ProjectDraft {
	const backend = useBackend();
	const [draftApp, setDraftApp] = useState<IApp | undefined>();
	const [draftMetadata, setDraftMetadata] = useState<IMetadata | undefined>();
	const [isSaving, setIsSaving] = useState(false);
	const seededFor = useRef<string | null>(null);

	useEffect(() => {
		if (!appId || !app || !metadata) {
			seededFor.current = null;
			setDraftApp(undefined);
			setDraftMetadata(undefined);
			return;
		}
		if (seededFor.current === appId) return;
		seededFor.current = appId;
		setDraftApp(app);
		setDraftMetadata(metadata);
	}, [appId, app, metadata]);

	const isPanelDirty = useCallback(
		(panel: InspectorPanel) => {
			if (!app || !metadata || !draftApp || !draftMetadata) return false;
			const appDirty = (APP_FIELDS[panel] ?? []).some(
				(key) => !isEqual(draftApp[key], app[key]),
			);
			const metaDirty = (METADATA_FIELDS[panel] ?? []).some(
				(key) => !isEqual(draftMetadata[key], metadata[key]),
			);
			return appDirty || metaDirty;
		},
		[app, metadata, draftApp, draftMetadata],
	);

	const dirtyPanels = useMemo(
		() => EDITABLE_PANELS.filter((panel) => isPanelDirty(panel)),
		[isPanelDirty],
	);

	const savePanel = useCallback(
		async (panel: InspectorPanel) => {
			if (!appId || !app || !metadata || !draftApp || !draftMetadata) return;
			setIsSaving(true);
			try {
				const metaKeys = METADATA_FIELDS[panel] ?? [];
				if (metaKeys.length > 0) {
					const nextMetadata = { ...metadata } as IMetadata;
					for (const key of metaKeys) {
						(nextMetadata as Record<string, unknown>)[key as string] =
							draftMetadata[key];
					}
					await backend.appState.pushAppMeta(appId, nextMetadata);
				}

				const appKeys = APP_FIELDS[panel] ?? [];
				if (appKeys.length > 0) {
					const nextApp = { ...app } as IApp;
					for (const key of appKeys) {
						(nextApp as Record<string, unknown>)[key as string] = draftApp[key];
					}
					await backend.appState.updateApp(nextApp);
				}

				await onSaved();
				toast.success("Saved");
			} catch (error) {
				toast.error(
					error instanceof Error ? error.message : "Failed to save changes",
				);
			} finally {
				setIsSaving(false);
			}
		},
		[appId, app, metadata, draftApp, draftMetadata, backend.appState, onSaved],
	);

	const resetPanel = useCallback(
		(panel: InspectorPanel) => {
			if (!app || !metadata) return;
			setDraftApp((current) => {
				if (!current) return current;
				const next = { ...current };
				for (const key of APP_FIELDS[panel] ?? []) {
					(next as Record<string, unknown>)[key as string] = app[key];
				}
				return next;
			});
			setDraftMetadata((current) => {
				if (!current) return current;
				const next = { ...current };
				for (const key of METADATA_FIELDS[panel] ?? []) {
					(next as Record<string, unknown>)[key as string] = metadata[key];
				}
				return next;
			});
		},
		[app, metadata],
	);

	return {
		draftApp,
		draftMetadata,
		setDraftApp,
		setDraftMetadata,
		isPanelDirty,
		dirtyPanels,
		savePanel,
		resetPanel,
		isSaving,
	};
}
