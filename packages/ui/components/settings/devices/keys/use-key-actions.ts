"use client";

import { useTranslation } from "@flow-like/locales";
import { useCallback, useEffect, useMemo, useRef } from "react";
import { deviceKeys } from "../../../../lib/device-management/hub/queries";
import type {
	ActionId,
	GateFailure,
	GateResult,
} from "../../../../lib/device-management/model/types";
import type {
	DeviceWorkspace,
	LocalSummary,
} from "../../../../lib/device-management/workspace/types";
import { gateCopy } from "../copy/gate-copy";
import {
	type AreaTime,
	type DevicesT,
	useAreaTime,
} from "../primitives/area-context";
import type {
	ConfirmOptions,
	ConfirmStrength,
} from "../primitives/confirm-sheet";
import type { ConsequenceRows } from "../primitives/consequence-preview";
import type { Gate } from "../primitives/gate-notice";
import { hubErrorCopy } from "../workspace/area-context";
import { useDeviceWorkspace } from "../workspace/device-workspace-provider";
import { useDeviceAction } from "../workspace/use-device-action";
import { useGate } from "../workspace/use-gate";
import {
	type KeyFailureCode,
	classifyKeyFailure,
	keyFailureCopy,
} from "./key-errors";
import {
	type ImportOutcome,
	type ParsedKeyFile,
	type PasswordChange,
	changePassword,
	deleteLocalKeys,
	importKeyFile,
	keyFileName,
	readAccountBackupRevision,
	resetMetricIdentity,
	restoreFromAccount,
	retryAccountBackup,
	saveAccountBackup,
	sealKeyFile,
	verifyAccountBackup,
} from "./key-operations";
import { keyFileLogOf } from "./key-store";

/** What a key flow hands back to the sheet or row that started it. */
export type KeyOutcome<T> =
	| { ok: true; value: T }
	| {
			ok: false;
			/** The sentence to show; empty when the user cancelled or the control was already running. */
			text: string;
			code?: KeyFailureCode;
			/** The failed gate, when the control was not allowed. */
			gate?: GateFailure;
	  };

export interface KeyTarget {
	deviceId: string;
	name: string;
}

interface FlowSpec<T> {
	action: ActionId;
	target: KeyTarget;
	/** Verb + object, for the confirm button and the tray. */
	label: string;
	/** What a failure means when nothing else explains it. */
	fallback?: KeyFailureCode;
	/** Track it in the tray as an account backup. */
	tray?: boolean;
	backupChanged?: boolean;
	consequence?: ConsequenceRows;
	strength?: ConfirmStrength;
	confirm?: Pick<
		ConfirmOptions,
		"icon" | "title" | "sub" | "typed" | "checkLabel" | "tone" | "extra"
	>;
	call(workspace: DeviceWorkspace): Promise<T>;
}

function tracked<T>(
	workspace: DeviceWorkspace,
	target: KeyTarget,
	run: () => Promise<T>,
): Promise<T> {
	const { activity } = workspace;
	const id = activity.start({
		kind: "account_backup",
		target: { deviceId: target.deviceId, deviceName: target.name },
		state: "active",
		label: { code: "account_backup" },
		progress: "indeterminate",
		actions: [],
		startedBy: "you",
	});
	return run().then(
		(value) => {
			activity.finish(id, "done", { code: "done" });
			return value;
		},
		(error: unknown) => {
			activity.finish(id, "failed", { code: "failed" });
			throw error;
		},
	);
}

function saveBlob(name: string, blob: Blob) {
	const url = URL.createObjectURL(blob);
	const link = document.createElement("a");
	link.href = url;
	link.download = name;
	link.click();
	setTimeout(() => URL.revokeObjectURL(url), 1000);
}

export interface KeyActions {
	/** "14:02" on the area clock, for result sentences. */
	timeNow(): string;
	saveBackup(
		target: KeyTarget,
		password: string,
		signal?: AbortSignal,
	): Promise<KeyOutcome<number>>;
	retryUpload(
		target: KeyTarget,
		signal?: AbortSignal,
	): Promise<KeyOutcome<number>>;
	verifyBackup(
		target: KeyTarget,
		password: string,
		signal?: AbortSignal,
	): Promise<KeyOutcome<number>>;
	/** The account's version for one device, read fresh; no password. */
	readRevision(
		target: KeyTarget,
		signal?: AbortSignal,
	): Promise<KeyOutcome<number>>;
	restore(
		target: KeyTarget,
		password: string,
		signal?: AbortSignal,
	): Promise<KeyOutcome<number>>;
	importFile(
		target: KeyTarget,
		parsed: ParsedKeyFile,
		password: string,
	): Promise<KeyOutcome<ImportOutcome>>;
	downloadFile(
		target: KeyTarget,
		password: string,
	): Promise<KeyOutcome<string>>;
	changePassword(
		target: KeyTarget,
		current: string,
		next: string,
		signal?: AbortSignal,
	): Promise<KeyOutcome<PasswordChange>>;
	/** Saves a file this computer already sealed (after a password change) and records it. */
	saveSealedFile(target: KeyTarget, file: Blob): string;
	deleteKeys(
		target: KeyTarget,
		confirm: Pick<FlowSpec<void>, "consequence" | "strength" | "confirm">,
	): Promise<KeyOutcome<void>>;
	/** The sheet shows the consequence rows and takes the password, so no second confirm opens. */
	resetMetricIdentity(
		target: KeyTarget,
		password: string,
	): Promise<KeyOutcome<void>>;
	keepSafe(): Promise<LocalSummary["persistence"]>;
}

/** Every key flow of N9 and the device Keys tab, through the action layer (gate, confirm, one call). */
export function useKeyActions(): KeyActions {
	const { t } = useTranslation("devices");
	const workspace = useDeviceWorkspace();
	const actions = useDeviceAction();
	const time = useAreaTime();
	const latest = useRef({ t, time });
	latest.current = { t, time };

	const run = useCallback(
		async <T>(spec: FlowSpec<T>): Promise<KeyOutcome<T>> => {
			const { target } = spec;
			const { hub } = workspace;
			const outcome = await actions.run<T>({
				action: spec.action,
				deviceId: target.deviceId,
				label: spec.label,
				resultKey: `keys:${spec.action}:${target.deviceId}`,
				...(spec.consequence ? { consequence: spec.consequence } : {}),
				...(spec.strength ? { strength: spec.strength } : {}),
				...(spec.confirm ? { confirm: spec.confirm } : {}),
				call: ({ workspace: current }) =>
					spec.tray
						? tracked(current, target, () => spec.call(current))
						: spec.call(current),
				invalidate: spec.backupChanged
					? [
							deviceKeys.accountBackups(hub.scopeKey),
							deviceKeys.accountBackup(hub.scopeKey, target.deviceId),
							deviceKeys.usage(hub.scopeKey),
						]
					: [],
			});
			const copy = latest.current;
			switch (outcome.status) {
				case "done":
					return { ok: true, value: outcome.result };
				case "gated":
					return {
						ok: false,
						gate: outcome.gate,
						text: gateCopy(copy.t, outcome.gate, copy.time).inline,
					};
				case "busy":
				case "cancelled":
					return { ok: false, text: "", code: "cancelled" };
				case "failed": {
					const failure = classifyKeyFailure(outcome.error, spec.fallback);
					return {
						ok: false,
						code: failure.code,
						text: keyFailureCopy(copy.t, failure, target.name, (hub) =>
							hubErrorCopy(copy.t, hub),
						),
					};
				}
				default:
					return {
						ok: false,
						code: "unknown",
						text: keyFailureCopy(copy.t, { code: "unknown" }, target.name),
					};
			}
		},
		[actions, workspace],
	);

	return useMemo<KeyActions>(
		() => ({
			timeNow: () =>
				latest.current.time.clock(
					Math.floor(workspace.clock.now() / 1000),
					false,
				),
			saveBackup: (target, password, signal) =>
				run({
					action: "account_backup_save",
					target,
					label: t("keys.action.backUp", "Back up {{device}} to your account", {
						device: target.name,
					}),
					tray: true,
					backupChanged: true,
					call: (current) =>
						saveAccountBackup(current, target.deviceId, password, signal),
				}),
			retryUpload: (target, signal) =>
				run({
					action: "account_backup_save",
					target,
					label: t(
						"keys.action.retryUpload",
						"Retry the backup upload for {{device}}",
						{ device: target.name },
					),
					tray: true,
					backupChanged: true,
					call: (current) =>
						retryAccountBackup(current, target.deviceId, signal),
				}),
			verifyBackup: (target, password, signal) =>
				run({
					action: "account_backup_save",
					target,
					label: t(
						"keys.action.check",
						"Check the account backup for {{device}}",
						{ device: target.name },
					),
					fallback: "wrong_backup_password",
					backupChanged: true,
					call: (current) =>
						verifyAccountBackup(current, target.deviceId, password, signal),
				}),
			readRevision: (target, signal) =>
				run({
					action: "account_backup_save",
					target,
					label: t(
						"keys.action.check",
						"Check the account backup for {{device}}",
						{ device: target.name },
					),
					backupChanged: true,
					call: (current) =>
						readAccountBackupRevision(current, target.deviceId, signal),
				}),
			restore: (target, password, signal) =>
				run({
					action: "account_backup_restore",
					target,
					label: t("keys.action.restore", "Restore the keys for {{device}}", {
						device: target.name,
					}),
					fallback: "wrong_backup_password",
					tray: true,
					backupChanged: true,
					call: (current) =>
						restoreFromAccount(current, target.deviceId, password, signal),
				}),
			importFile: (target, parsed, password) =>
				run({
					action: "import_key_file",
					target,
					label: t("keys.action.import", "Import the keys for {{device}}", {
						device: target.name,
					}),
					fallback: "wrong_backup_password",
					call: (current) => importKeyFile(current, parsed, password),
				}),
			downloadFile: (target, password) =>
				run({
					action: "download_key_file",
					target,
					label: t(
						"keys.action.download",
						"Download the key backup file for {{device}}",
						{ device: target.name },
					),
					call: async (current) => {
						const name = keyFileName(target.deviceId);
						saveBlob(
							name,
							await sealKeyFile(current, target.deviceId, password),
						);
						keyFileLogOf(current).update(target.deviceId, {
							savedAt: current.clock.now(),
						});
						return name;
					},
				}),
			changePassword: (target, current, next, signal) =>
				run({
					action: "change_device_password",
					target,
					label: t(
						"keys.action.changePassword",
						"Change the device password for {{device}}",
						{ device: target.name },
					),
					fallback: "storage",
					call: async (ws) => {
						const change = await changePassword(
							ws,
							target.deviceId,
							current,
							next,
							signal,
						);
						keyFileLogOf(ws).update(target.deviceId, {
							passwordChangedAt: ws.clock.now(),
						});
						return change;
					},
				}),
			saveSealedFile: (target, file) => {
				const name = keyFileName(target.deviceId);
				saveBlob(name, file);
				keyFileLogOf(workspace).update(target.deviceId, {
					savedAt: workspace.clock.now(),
				});
				return name;
			},
			deleteKeys: (target, confirm) =>
				run({
					action: "delete_local_keys",
					target,
					label: t("keys.action.delete", "Delete keys for {{device}}", {
						device: target.name,
					}),
					fallback: "storage",
					...confirm,
					call: async (current) => {
						await deleteLocalKeys(current, target.deviceId);
						keyFileLogOf(current).update(target.deviceId, null);
					},
				}),
			resetMetricIdentity: (target, password) =>
				run({
					action: "reset_metric_reader",
					target,
					label: t(
						"keys.action.resetIdentity",
						"Reset the metric-group identity for {{device}}",
						{ device: target.name },
					),
					fallback: "storage",
					call: (current) =>
						resetMetricIdentity(current, target.deviceId, password),
				}),
			keepSafe: () => workspace.local.requestPersistence(),
		}),
		[run, t, workspace],
	);
}

/** R7: a gate result as `GatedAction` takes it; null when the control is allowed. */
export function gateOf(
	result: GateResult,
	t: DevicesT,
	time: AreaTime,
): Gate | null {
	return result.ok
		? null
		: { kind: result.kind, reason: gateCopy(t, result, time).inline };
}

export function useKeyGate(action: ActionId, deviceId?: string): Gate | null {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const result = useGate(action, deviceId);
	return useMemo(() => gateOf(result, t, time), [result, t, time]);
}

/** Aborts on unmount and when `resetKey` changes, so a late completion reaches nobody. */
export function useFlowGuard(resetKey = ""): () => {
	alive: () => boolean;
	signal: AbortSignal;
} {
	const guard = useRef({ alive: true, controller: new AbortController() });
	// biome-ignore lint/correctness/useExhaustiveDependencies: `resetKey` starts a new guard
	useEffect(() => {
		const current = { alive: true, controller: new AbortController() };
		guard.current = current;
		return () => {
			current.alive = false;
			current.controller.abort();
		};
	}, [resetKey]);
	return useCallback(() => {
		const current = guard.current;
		return {
			alive: () => current.alive,
			signal: current.controller.signal,
		};
	}, []);
}
