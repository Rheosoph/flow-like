"use client";

import { useTranslation } from "@flow-like/locales";
import { type ReactNode, useCallback, useMemo, useRef, useState } from "react";
import { useOverlay } from "../workspace/overlay-store";
import {
	AccountBackupSheet,
	type BackupMode,
	RestoreKeysSheet,
} from "./account-backup-panel";
import { ChangePasswordSheet } from "./change-password-sheet";
import { useDeleteKeys } from "./delete-keys-flow";
import { DownloadKeyFilesSheet } from "./download-key-files-sheet";
import { ImportKeyFilesSheet } from "./import-key-files-sheet";
import { type KeyFileLog, rowScope, useKeyResults } from "./key-store";
import type { KeyRow, LocalOnlyRow } from "./keys-model";
import { ResetIdentitySheet } from "./reset-identity-sheet";
import { useFlowGuard, useKeyActions } from "./use-key-actions";
import type { KeysRead } from "./use-keys-model";

/** One sheet at a time (SPEC §4.21): opening a flow replaces the one on screen. */
export type KeyFlow =
	| { kind: "restore"; deviceId?: string }
	| { kind: "import" }
	| { kind: "download"; deviceId?: string }
	| { kind: "password"; deviceId?: string }
	| { kind: "backup"; deviceId: string; mode: BackupMode }
	| { kind: "reset"; deviceId: string };

export interface KeyFlowsApi {
	open(flow: KeyFlow): void;
	/** Publishes a pending backup upload; needs no password. */
	retryUpload(row: KeyRow): void;
	deleteKeys(row: KeyRow | LocalOnlyRow): void;
	unlockSeveral(): void;
	/** A row-level flow is running for this device. */
	busy(deviceId: string): boolean;
}

const isDeviceRow = (row: KeyRow | LocalOnlyRow): row is KeyRow =>
	"category" in row;

/** The key flows of one screen: which sheet is open, and the row-level actions without one. */
export function useKeyFlows(
	read: KeysRead,
	files: KeyFileLog,
): { api: KeyFlowsApi; sheets: ReactNode } {
	const { t } = useTranslation("devices");
	const actions = useKeyActions();
	const results = useKeyResults();
	const overlay = useOverlay();
	const guard = useFlowGuard();
	const remove = useDeleteKeys(files);
	const [flow, setFlow] = useState<KeyFlow | null>(null);
	const [retrying, setRetrying] = useState<string | null>(null);
	const sequence = useRef(0);
	const [opened, setOpened] = useState(0);

	const open = useCallback((next: KeyFlow) => {
		sequence.current += 1;
		setOpened(sequence.current);
		setFlow(next);
	}, []);
	const close = useCallback(() => setFlow(null), []);

	const retryUpload = useCallback(
		async (row: KeyRow) => {
			const current = guard();
			setRetrying(row.deviceId);
			const outcome = await actions.retryUpload(
				{ deviceId: row.deviceId, name: row.name },
				current.signal,
			);
			if (current.alive()) setRetrying(null);
			if (outcome.ok)
				results.put(
					rowScope(row.deviceId),
					"good",
					t(
						"keys.result.uploaded",
						"The backup reached your account as version {{version}} at {{at}}.",
						{ version: outcome.value, at: actions.timeNow() },
					),
				);
			else if (outcome.text)
				results.put(rowScope(row.deviceId), "critical", outcome.text);
		},
		[guard, actions, results, t],
	);

	const removeRun = remove.run;
	const api = useMemo<KeyFlowsApi>(
		() => ({
			open,
			retryUpload: (row) => void retryUpload(row),
			deleteKeys: (row) =>
				void removeRun(
					isDeviceRow(row) ? { kind: "device", row } : { kind: "local", row },
				),
			unlockSeveral: () => overlay.openUnlockSeveral(),
			busy: (deviceId) => retrying === deviceId || remove.busy === deviceId,
		}),
		[open, retryUpload, removeRun, overlay, retrying, remove.busy],
	);

	const rowOf = (deviceId: string) =>
		read.model.rows.find((row) => row.deviceId === deviceId);
	let sheet: ReactNode = null;
	if (flow?.kind === "restore")
		sheet = (
			<RestoreKeysSheet
				key={opened}
				read={read}
				preselect={flow.deviceId}
				flows={api}
				onClose={close}
			/>
		);
	else if (flow?.kind === "import")
		sheet = <ImportKeyFilesSheet key={opened} read={read} onClose={close} />;
	else if (flow?.kind === "download")
		sheet = (
			<DownloadKeyFilesSheet
				key={opened}
				read={read}
				preselect={flow.deviceId}
				onClose={close}
			/>
		);
	else if (flow?.kind === "password")
		sheet = (
			<ChangePasswordSheet
				key={opened}
				read={read}
				files={files}
				deviceId={flow.deviceId}
				onClose={close}
			/>
		);
	else if (flow?.kind === "backup") {
		const row = rowOf(flow.deviceId);
		sheet = row ? (
			<AccountBackupSheet
				key={opened}
				row={row}
				mode={flow.mode}
				onClose={close}
			/>
		) : null;
	} else if (flow?.kind === "reset") {
		const row = rowOf(flow.deviceId);
		sheet = row ? (
			<ResetIdentitySheet key={opened} row={row} onClose={close} />
		) : null;
	}
	return { api, sheets: sheet };
}
