"use client";

import { useEffect, useId, useRef, useState } from "react";
import {
	type ReleaseConfig,
	type ReleaseTarget,
	type VerifiedRelease,
	fetchVerifiedRelease,
	standalonePackageModes,
} from "../../../lib/device-management/package";
import {
	type DeviceSetupReadiness,
	checkDeviceSetup,
} from "../../../lib/device-management/readiness";
import { prepareDevicePackage } from "../../../lib/device-management/setup";
import type { DeviceAccountScope } from "../../../lib/device-management/storage";
import { useBackend } from "../../../state/backend-state";
import type { IProfile } from "../../../types";
import { Button } from "../../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../../ui/dialog";
import { Input } from "../../ui/input";

export function DeviceSetupDialog({
	profile,
	scope,
	release,
	onClose,
}: {
	profile: IProfile;
	scope: DeviceAccountScope;
	release: ReleaseConfig | undefined;
	onClose: () => void;
}) {
	const backend = useBackend();
	const formId = useId();
	const [verified, setVerified] = useState<VerifiedRelease>();
	const [readiness, setReadiness] = useState<DeviceSetupReadiness>();
	const [checkAttempt, setCheckAttempt] = useState(0);
	const latestAttempt = useRef(checkAttempt);
	latestAttempt.current = checkAttempt;
	const [name, setName] = useState("");
	const [password, setPassword] = useState("");
	const [repeat, setRepeat] = useState("");
	const [backupToAccount, setBackupToAccount] = useState(true);
	const [target, setTarget] = useState<ReleaseTarget>(
		"x86_64-unknown-linux-gnu",
	);
	const [mode, setMode] = useState<"binary" | "docker" | "both">("binary");
	const [error, setError] = useState<string>();
	const [busy, setBusy] = useState(false);
	const [result, setResult] = useState<{
		packageUrl: string;
		backupUrl: string;
		deviceId: string;
		accountBackup?: "saved" | "local_only";
	}>();
	const current = useRef(true);
	const abort = useRef(new AbortController());
	const urls = useRef<string[]>([]);
	const modes = verified
		? standalonePackageModes(verified.manifest, target)
		: { binary: false, docker: false };
	const selectionAvailable =
		mode === "binary"
			? modes.binary
			: mode === "docker"
				? modes.docker
				: modes.binary && modes.docker;
	useEffect(() => {
		current.current = true;
		abort.current = new AbortController();
		setVerified(undefined);
		setBusy(false);
		setResult(undefined);
		setReadiness(undefined);
		setError(undefined);
		const signal = abort.current.signal;
		const active = () =>
			current.current &&
			!signal.aborted &&
			latestAttempt.current === checkAttempt;
		void checkDeviceSetup(backend.apiState, profile, signal)
			.then(async (status) => {
				if (!active()) return;
				setReadiness(status);
				if (!status.ready || !release) return;
				const value = await fetchVerifiedRelease(release, signal);
				if (current.current) {
					if (signal.aborted) return;
					setVerified(value);
					const first = value.targets.find((candidate) => {
						const available = standalonePackageModes(value.manifest, candidate);
						return available.binary || available.docker;
					});
					if (first) {
						setTarget(first);
						setMode(
							standalonePackageModes(value.manifest, first).binary
								? "binary"
								: "docker",
						);
					} else {
						setError(
							"This release has no browser package: its binaries exceed 256 MiB and no supported Docker alternative is signed.",
						);
					}
				}
			})
			.catch(() => {
				if (active())
					setError(
						"Device setup checks could not finish. Check the hub version, connection and published release, then retry. No enrollment was created.",
					);
			});
		return () => {
			current.current = false;
			abort.current.abort();
			for (const url of urls.current) URL.revokeObjectURL(url);
		};
	}, [release, backend.apiState, profile, checkAttempt]);
	async function create() {
		if (
			busy ||
			!verified ||
			!release ||
			!readiness?.ready ||
			!selectionAvailable
		)
			return;
		if (password !== repeat) {
			setError("The passwords do not match.");
			return;
		}
		setBusy(true);
		setError(undefined);
		const secret = password;
		const signal = abort.current.signal;
		const active = () => current.current && !signal.aborted;
		setPassword("");
		setRepeat("");
		try {
			const prepared = await prepareDevicePackage({
				api: backend.apiState,
				profile,
				scope,
				name: name.trim(),
				password: secret,
				backupToAccount,
				target,
				mode,
				release,
				verifiedRelease: verified,
				signal,
			});
			if (!active()) return;
			const packageUrl = URL.createObjectURL(prepared.package);
			const backupUrl = URL.createObjectURL(prepared.backup);
			urls.current.push(packageUrl, backupUrl);
			setResult({
				packageUrl,
				backupUrl,
				deviceId: prepared.deviceId,
				accountBackup: prepared.accountBackup,
			});
		} catch (error) {
			if (active())
				setError(
					error instanceof Error
						? error.message
						: "Device package could not be created.",
				);
		} finally {
			if (active()) setBusy(false);
		}
	}
	return (
		<Dialog
			open
			onOpenChange={(open) => {
				if (!open) onClose();
			}}
		>
			<DialogContent className="max-h-[90vh] overflow-auto">
				<DialogHeader>
					<DialogTitle>Set up a device</DialogTitle>
					<DialogDescription>
						Create a deployment package and protect your local management keys
						with a password.
					</DialogDescription>
				</DialogHeader>
				{!result && (
					<div className="space-y-2" aria-label="Device setup checks">
						{readiness ? (
							<ul className="space-y-1 text-sm">
								{readiness.checks.map((check) => (
									<li
										key={check.id}
										className={
											check.ready ? "text-muted-foreground" : "text-destructive"
										}
									>
										{check.ready ? "✓ " : "! "}
										{check.message}
									</li>
								))}
							</ul>
						) : (
							<output>Checking device setup…</output>
						)}
						<Button
							variant="outline"
							disabled={busy}
							onClick={() => setCheckAttempt((value) => value + 1)}
						>
							Check again
						</Button>
					</div>
				)}
				{!release ? (
					<p>
						This hub has not configured a trusted standalone release. Ask the
						hub operator to enable signed release packages.
					</p>
				) : result ? (
					<div className="space-y-4">
						{result.accountBackup === "saved" && (
							<p>
								Encrypted keys are backed up to your account. Your device
								password is required to restore them.
							</p>
						)}
						{result.accountBackup === "local_only" && (
							<p role="alert">
								The account backup did not finish. Save the encrypted backup
								below, then retry account backup from device management.
							</p>
						)}
						<p>
							The package is ready. Start it on the target device within one day
							to complete enrollment.
						</p>
						<Button asChild>
							<a
								href={result.packageUrl}
								download={`flow-like-${result.deviceId}.zip`}
							>
								Download deployment package
							</a>
						</Button>
						<p className="text-sm text-muted-foreground">
							Save this encrypted controller backup separately. Your password
							and invitation keys are not in the deployment package.
						</p>
						<Button variant="outline" asChild>
							<a
								href={result.backupUrl}
								download={`flow-like-controller-${result.deviceId}.json`}
							>
								Save encrypted controller backup
							</a>
						</Button>
						<p className="font-mono text-xs break-all">{result.deviceId}</p>
					</div>
				) : (
					<form
						className="space-y-4"
						onSubmit={(event) => {
							event.preventDefault();
							void create();
						}}
					>
						<label
							htmlFor={`${formId}-name`}
							className="block space-y-1 text-sm"
						>
							Device name
							<Input
								id={`${formId}-name`}
								value={name}
								onChange={(event) => setName(event.target.value)}
								maxLength={128}
								required
								disabled={busy}
							/>
						</label>
						<label className="block space-y-1 text-sm">
							Target platform
							<select
								className="w-full rounded-md border bg-background p-2"
								value={target}
								onChange={(event) => {
									const selected = event.target.value as ReleaseTarget;
									setTarget(selected);
									if (verified) {
										const available = standalonePackageModes(
											verified.manifest,
											selected,
										);
										if (!available.binary) setMode("docker");
										else if (!available.docker) setMode("binary");
									}
								}}
								disabled={busy || !verified}
							>
								{verified?.targets.map((value) => {
									const available = standalonePackageModes(
										verified.manifest,
										value,
									);
									return (
										<option
											key={value}
											value={value}
											disabled={!available.binary && !available.docker}
										>
											{value}
											{!available.binary && !available.docker
												? " (binary exceeds 256 MiB; no Docker alternative)"
												: ""}
										</option>
									);
								})}
							</select>
						</label>
						<label className="block space-y-1 text-sm">
							Run with
							<select
								className="w-full rounded-md border bg-background p-2"
								value={mode}
								onChange={(event) => setMode(event.target.value as typeof mode)}
								disabled={busy}
							>
								<option value="binary" disabled={!modes.binary}>
									Native binary
								</option>
								<option value="docker" disabled={!modes.docker}>
									Docker Compose
								</option>
								<option value="both" disabled={!modes.binary || !modes.docker}>
									Binary and Docker Compose
								</option>
							</select>
							{verified && !modes.binary && (
								<p className="text-muted-foreground">
									The selected binary exceeds the browser package limit of 256
									MiB.
									{modes.docker
										? " Docker Compose is available for this target."
										: " This target has no Docker alternative in this release."}
								</p>
							)}
						</label>
						<label
							htmlFor={`${formId}-password`}
							className="block space-y-1 text-sm"
						>
							Management password
							<Input
								id={`${formId}-password`}
								type="password"
								autoComplete="new-password"
								minLength={12}
								maxLength={4096}
								value={password}
								onChange={(event) => setPassword(event.target.value)}
								required
								disabled={busy}
							/>
						</label>
						<label
							htmlFor={`${formId}-repeat`}
							className="block space-y-1 text-sm"
						>
							Repeat password
							<Input
								id={`${formId}-repeat`}
								type="password"
								autoComplete="new-password"
								value={repeat}
								onChange={(event) => setRepeat(event.target.value)}
								required
								disabled={busy}
							/>
						</label>
						<p className="text-sm text-muted-foreground">
							Remember this password. It stays on your device and cannot be
							recovered by the hub.
						</p>
						{verified ? (
							<p className="text-xs text-muted-foreground">
								Verified release {verified.manifest.release_version}
							</p>
						) : (
							<output>Verifying the configured release…</output>
						)}
						<label className="flex items-start gap-2 text-sm">
							<input
								type="checkbox"
								checked={backupToAccount}
								onChange={(event) => setBackupToAccount(event.target.checked)}
								disabled={busy}
							/>
							Back up password-encrypted management keys to my account for
							recovery on another computer.
						</label>
						<Button
							type="submit"
							disabled={busy || !verified || !selectionAvailable}
						>
							{busy ? "Creating package…" : "Create deployment package"}
						</Button>
					</form>
				)}
				{error && (
					<p role="alert" className="text-sm text-destructive">
						{error}
					</p>
				)}
			</DialogContent>
		</Dialog>
	);
}
