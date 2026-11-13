"use client";

import { useTranslation } from "@flow-like/locales";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { useBackend } from "../../state/backend-state";
import type {
	ComposerAttachResult,
	ComposerAttachSource,
} from "../../state/composer-capture";
import { StorageAttachDialog } from "./storage-attach-dialog";
import {
	STORAGE_ATTACH_MAX_BYTES,
	STORAGE_ATTACH_MAX_FILES,
	type StoragePick,
	checkStoragePicks,
	fetchStorageFiles,
} from "./storage-attach-limits";

export interface StorageAttachSource {
	readonly source: ComposerAttachSource | undefined;
	readonly dialog: ReactNode;
}

type Resolver = (picks: readonly StoragePick[] | null) => void;

/** "From storage" source for one app; render `dialog` somewhere in the same tree as the chat. */
export function useStorageAttachSource(
	appId: string | undefined,
): StorageAttachSource {
	const { t } = useTranslation();
	const backend = useBackend();
	const [open, setOpen] = useState(false);
	const resolver = useRef<Resolver | null>(null);

	const settle = useCallback((picks: readonly StoragePick[] | null) => {
		resolver.current?.(picks);
		resolver.current = null;
		setOpen(false);
	}, []);

	useEffect(() => () => settle(null), [settle]);
	useEffect(() => {
		if (!appId) settle(null);
	}, [appId, settle]);

	const attach = useCallback(async (): Promise<ComposerAttachResult> => {
		if (!appId) return {};
		resolver.current?.(null);
		const picks = await new Promise<readonly StoragePick[] | null>(
			(resolve) => {
				resolver.current = resolve;
				setOpen(true);
			},
		);
		if (!picks || picks.length === 0) return {};
		const problem = checkStoragePicks(picks);
		if (problem === "tooMany")
			throw new Error(
				t("composerStorageTooMany", "Pick at most {{max}} files.", {
					max: STORAGE_ATTACH_MAX_FILES,
				}),
			);
		if (problem === "tooLarge")
			throw new Error(
				t(
					"composerStorageTooLarge",
					"The files are larger than {{max}} MB together.",
					{ max: STORAGE_ATTACH_MAX_BYTES / 1024 / 1024 },
				),
			);
		try {
			return {
				files: await fetchStorageFiles(backend.storageState, appId, picks),
			};
		} catch (error) {
			console.warn("[StorageAttach] fetch failed:", error);
			throw new Error(
				t(
					"composerStorageFetchFailed",
					"Could not load the files from storage.",
				),
			);
		}
	}, [appId, backend.storageState, t]);

	const source = useMemo<ComposerAttachSource | undefined>(
		() => (appId ? { kind: "storage", attach } : undefined),
		[appId, attach],
	);
	const dialog = appId ? (
		<StorageAttachDialog
			open={open}
			appId={appId}
			onPick={settle}
			onCancel={() => settle(null)}
		/>
	) : null;
	return { source, dialog };
}
