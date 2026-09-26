"use client";

import { useEffect, useId, useRef, useState } from "react";
import { loadDeviceCrypto } from "../../../lib/device-management/crypto";
import { changeDevicePassword } from "../../../lib/device-management/password";
import {
	type DeviceAccountScope,
	type LocalDeviceVault,
	encryptedControllerBackup,
} from "../../../lib/device-management/storage";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";

export function DevicePasswordChange({
	scope,
	record,
	disabled,
	onChanged,
}: {
	scope: DeviceAccountScope;
	record: LocalDeviceVault;
	disabled: boolean;
	onChanged: (record: LocalDeviceVault) => void;
}) {
	const id = useId();
	const [currentPassword, setCurrentPassword] = useState("");
	const [newPassword, setNewPassword] = useState("");
	const [confirmation, setConfirmation] = useState("");
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string>();
	const [changed, setChanged] = useState(false);
	const [backupUrl, setBackupUrl] = useState<string>();
	const active = useRef(true);
	const working = useRef(false);
	const cancellation = useRef(new AbortController());
	const url = useRef<string | undefined>(undefined);
	useEffect(() => {
		active.current = true;
		cancellation.current = new AbortController();
		return () => {
			active.current = false;
			cancellation.current.abort();
			if (url.current) URL.revokeObjectURL(url.current);
		};
	}, []);
	async function submit() {
		if (working.current || disabled) return;
		const old = currentPassword;
		const next = newPassword;
		const matching = next === confirmation;
		setCurrentPassword("");
		setNewPassword("");
		setConfirmation("");
		setError(undefined);
		if (!matching) {
			setError("The new passwords do not match.");
			return;
		}
		working.current = true;
		setBusy(true);
		try {
			const crypto = await loadDeviceCrypto();
			const replacement = await changeDevicePassword(
				scope,
				record,
				old,
				next,
				crypto,
				cancellation.current.signal,
			);
			if (!active.current) return;
			onChanged(replacement);
			setChanged(true);
			try {
				if (url.current) URL.revokeObjectURL(url.current);
				url.current = URL.createObjectURL(
					encryptedControllerBackup(scope, replacement),
				);
				setBackupUrl(url.current);
			} catch {
				setError(
					"The password changed, but the backup download could not be prepared. Keep this app's local storage until you can save a new backup.",
				);
			}
		} catch (failure) {
			if (active.current)
				setError(
					failure instanceof Error
						? failure.message
						: "The password could not be changed.",
				);
		} finally {
			working.current = false;
			if (active.current) setBusy(false);
		}
	}
	return (
		<details className="rounded border p-3">
			<summary className="cursor-pointer font-medium">
				Change this app's device password
			</summary>
			<p className="mt-3 text-sm text-muted-foreground">
				Re-encrypt your saved keys on this browser or desktop app. The password
				stays here. Your identities, group telemetry state, existing sessions
				and other users remain valid. Old backups still use the old password;
				replace or delete copies you no longer need.
			</p>
			{changed ? (
				<div className="mt-3 space-y-2">
					<output className="block">
						Password changed. Use the new password to unlock this app's device
						keys.
					</output>
					{backupUrl && (
						<Button asChild variant="outline">
							<a
								href={backupUrl}
								download={`flow-like-controller-${record.deviceId}.json`}
							>
								Save updated encrypted backup
							</a>
						</Button>
					)}
				</div>
			) : (
				<form
					className="mt-3 space-y-3"
					onSubmit={(event) => {
						event.preventDefault();
						void submit();
					}}
				>
					<label className="block space-y-1 text-sm" htmlFor={`${id}-current`}>
						Current password
						<Input
							id={`${id}-current`}
							type="password"
							autoComplete="current-password"
							value={currentPassword}
							onChange={(event) => setCurrentPassword(event.target.value)}
							disabled={disabled || busy}
							required
							maxLength={4096}
						/>
					</label>
					<label className="block space-y-1 text-sm" htmlFor={`${id}-new`}>
						New password
						<Input
							id={`${id}-new`}
							type="password"
							autoComplete="new-password"
							value={newPassword}
							onChange={(event) => setNewPassword(event.target.value)}
							disabled={disabled || busy}
							required
							minLength={12}
							maxLength={4096}
						/>
					</label>
					<label
						className="block space-y-1 text-sm"
						htmlFor={`${id}-confirmation`}
					>
						Repeat new password
						<Input
							id={`${id}-confirmation`}
							type="password"
							autoComplete="new-password"
							value={confirmation}
							onChange={(event) => setConfirmation(event.target.value)}
							disabled={disabled || busy}
							required
							minLength={12}
							maxLength={4096}
						/>
					</label>
					<Button type="submit" disabled={disabled || busy}>
						{busy ? "Changing password…" : "Change local password"}
					</Button>
				</form>
			)}
			{error && (
				<p role="alert" className="mt-3 text-sm text-destructive">
					{error}
				</p>
			)}
		</details>
	);
}
