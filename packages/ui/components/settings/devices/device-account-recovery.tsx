"use client";

import { useEffect, useId, useRef, useState } from "react";
import {
	restoreAccountRecovery,
	saveAccountRecovery,
} from "../../../lib/device-management/recovery";
import type {
	DeviceAccountScope,
	LocalDeviceVault,
} from "../../../lib/device-management/storage";
import { useBackend } from "../../../state/backend-state";
import type { IProfile } from "../../../types";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";

export function DeviceAccountRecovery({
	profile,
	scope,
	deviceId,
	record,
	disabled,
	onRestored,
}: {
	profile: IProfile;
	scope: DeviceAccountScope;
	deviceId: string;
	record?: LocalDeviceVault;
	disabled: boolean;
	onRestored: (record: LocalDeviceVault) => void;
}) {
	const backend = useBackend();
	const id = useId();
	const [password, setPassword] = useState("");
	const [busy, setBusy] = useState(false);
	const [message, setMessage] = useState<string>();
	const [error, setError] = useState<string>();
	const active = useRef(true);
	const working = useRef(false);
	const cancellation = useRef(new AbortController());
	const accountContext = JSON.stringify([
		scope.issuer,
		scope.account,
		scope.apiOrigin,
		scope.profileId,
		deviceId,
	]);
	useEffect(() => {
		if (!accountContext) return;
		active.current = true;
		cancellation.current = new AbortController();
		working.current = false;
		setBusy(false);
		setPassword("");
		setError(undefined);
		setMessage(undefined);
		return () => {
			active.current = false;
			cancellation.current.abort();
		};
	}, [accountContext]);
	async function run(action: "save" | "restore") {
		if (disabled || working.current) return;
		const secret = password;
		setPassword("");
		setError(undefined);
		setMessage(undefined);
		working.current = true;
		setBusy(true);
		const signal = cancellation.current.signal;
		try {
			const input = {
				api: backend.apiState,
				profile,
				scope,
				deviceId,
				password: secret,
				signal,
			};
			if (action === "save") {
				const revision = await saveAccountRecovery(input);
				if (active.current && !signal.aborted)
					setMessage(
						`Encrypted account backup saved, revision ${revision}. Keep your device password to restore it.`,
					);
			} else {
				const restored = await restoreAccountRecovery(input);
				if (active.current && !signal.aborted) {
					onRestored(restored);
					setMessage(
						record
							? "Account backup verified and its revision reconciled. Your local keys and password remain unchanged. Save again to upload local changes."
							: "Device keys restored. Unlock to connect using a fresh telemetry endpoint.",
					);
				}
			}
		} catch (failure) {
			if (active.current && !signal.aborted)
				setError(
					failure instanceof Error
						? failure.message
						: "Account recovery did not finish.",
				);
		} finally {
			if (active.current && !signal.aborted) {
				working.current = false;
				setBusy(false);
			}
		}
	}
	return (
		<details className="rounded border p-3" open={!record}>
			<summary className="cursor-pointer font-medium">
				Encrypted account backup and recovery
			</summary>
			<p className="mt-3 text-sm text-muted-foreground">
				Restore your device keys on another browser or computer with your
				account and device password. The hub stores encrypted keys; it cannot
				recover a forgotten password. Save an updated account backup after
				changing this app's password. Older copies still use their original
				password. If another browser saved first, verify its backup to reconcile
				the revision, then save your local changes.
			</p>
			<form
				className="mt-3 space-y-3"
				onSubmit={(event) => {
					event.preventDefault();
					void run(record ? "save" : "restore");
				}}
			>
				<label htmlFor={id} className="block space-y-1 text-sm">
					Device password for this backup
					<Input
						id={id}
						type="password"
						autoComplete="current-password"
						value={password}
						onChange={(event) => setPassword(event.target.value)}
						required
						minLength={12}
						maxLength={4096}
						disabled={disabled || busy}
					/>
				</label>
				<div className="flex flex-wrap gap-2">
					{record && (
						<Button
							type="submit"
							disabled={disabled || busy || password.length < 12}
						>
							Save account backup
						</Button>
					)}
					<Button
						type={record ? "button" : "submit"}
						variant={record ? "outline" : "default"}
						disabled={disabled || busy || password.length < 12}
						onClick={record ? () => void run("restore") : undefined}
					>
						{record ? "Verify saved account backup" : "Restore from account"}
					</Button>
				</div>
			</form>
			{busy && (
				<output className="mt-3 block">
					Updating encrypted recovery state…
				</output>
			)}
			{message && <output className="mt-3 block text-sm">{message}</output>}
			{error && (
				<p role="alert" className="mt-3 text-sm text-destructive">
					{error}
				</p>
			)}
		</details>
	);
}
