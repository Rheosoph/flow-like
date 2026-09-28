"use client";

import { useEffect, useId, useRef, useState } from "react";
import {
	loadDeviceCrypto,
	withPassword,
} from "../../../lib/device-management/crypto";
import {
	type DeviceAccountScope,
	type DeviceStoragePersistence,
	type LocalDeviceVault,
	addDeviceVault,
	deviceApiBase,
	deviceStorageWarning,
	pinDeviceIdentity,
	readDeviceVault,
	requestPersistentDeviceStorage,
} from "../../../lib/device-management/storage";
import type {
	DeviceReceipt,
	Ed25519PublicKey,
	OnboardingManifest,
} from "../../../lib/device-management/types";
import { Button } from "../../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../../ui/dialog";
import { Input } from "../../ui/input";

function accessRequest(
	account: string,
	controllerKey: Ed25519PublicKey,
	grantId: string,
): Blob {
	return new Blob(
		[
			JSON.stringify(
				[
					{
						user_id: account,
						controller_key: controllerKey,
						grant_id: grantId,
					},
				],
				null,
				2,
			),
		],
		{ type: "application/json" },
	);
}

/** A vault this dialog created earlier for the same owner bundle can re-issue its public request. */
function requestVault(
	existing: LocalDeviceVault | undefined,
	bundle: { receipt: DeviceReceipt; owner_controller_key: Ed25519PublicKey },
): "reusable" | "conflict" | undefined {
	if (!existing) return undefined;
	return existing.grantId !== "owner" &&
		existing.ownerControllerKey?.x === bundle.owner_controller_key.x &&
		existing.manifestJws === bundle.receipt.manifest_jws
		? "reusable"
		: "conflict";
}

export function DeviceAccessDialog({
	scope,
	onClose,
}: { scope: DeviceAccountScope; onClose: () => void }) {
	const id = useId();
	const [bundle, setBundle] = useState<{
		receipt: DeviceReceipt;
		owner_controller_key: Ed25519PublicKey;
	}>();
	const [manifest, setManifest] = useState<OnboardingManifest>();
	const [password, setPassword] = useState("");
	const [repeat, setRepeat] = useState("");
	const [error, setError] = useState<string>();
	const [busy, setBusy] = useState(false);
	const [requestUrl, setRequestUrl] = useState<string>();
	const [existing, setExisting] = useState<LocalDeviceVault>();
	const [storage, setStorage] = useState<DeviceStoragePersistence>();
	const current = useRef(true);
	const reuse = bundle ? requestVault(existing, bundle) : undefined;
	const urls = useRef<string[]>([]);
	const importVersion = useRef(0);
	useEffect(() => {
		current.current = true;
		return () => {
			current.current = false;
			importVersion.current++;
			for (const url of urls.current) URL.revokeObjectURL(url);
		};
	}, []);
	function offer(request: Blob) {
		const url = URL.createObjectURL(request);
		urls.current.push(url);
		setRequestUrl(url);
	}
	async function create() {
		if (!bundle || !manifest || busy || reuse) return;
		if (password !== repeat) {
			setError("The passwords do not match.");
			return;
		}
		setBusy(true);
		setError(undefined);
		const secret = password;
		setPassword("");
		setRepeat("");
		try {
			const module = await loadDeviceCrypto();
			const created = await withPassword(secret, (bytes) =>
				module.createControllerVault(manifest.device_id, bytes),
			);
			const grantId = crypto.randomUUID();
			if (!current.current) return;
			const persistence = await requestPersistentDeviceStorage();
			await addDeviceVault(scope, {
				deviceId: manifest.device_id,
				controllerPublic: created.public_bundle,
				controllerVault: Uint8Array.from(created.vault),
				manifestJws: bundle.receipt.manifest_jws,
				ownerControllerKey: bundle.owner_controller_key,
				grantId,
			});
			await pinDeviceIdentity(scope, manifest.device_id, bundle.receipt);
			if (!current.current) return;
			setStorage(persistence);
			offer(
				accessRequest(
					scope.account,
					created.public_bundle.controller_key,
					grantId,
				),
			);
		} catch (error) {
			if (current.current)
				setError(
					error instanceof Error
						? error.message
						: "The access request could not be created.",
				);
		} finally {
			if (current.current) setBusy(false);
		}
	}
	return (
		<Dialog
			open
			onOpenChange={(open) => {
				if (!open) onClose();
			}}
		>
			<DialogContent>
				<DialogHeader>
					<DialogTitle>Request shared device access</DialogTitle>
					<DialogDescription>
						Create your own password-protected controller. The owner must
						approve its public key before it can control the device.
					</DialogDescription>
				</DialogHeader>
				{requestUrl ? (
					<div className="space-y-3">
						<p>
							Give this access request to the owner. Ask them to import its
							recipient list in Share device access. The device will appear in
							your list after approval.
						</p>
						<p className="text-sm text-muted-foreground">
							If the owner grants deploy, start, restart or scale, your
							workflows run with the device agent's full access unless the
							device requires isolation. Treat that access as trust in the whole
							device.
						</p>
						{deviceStorageWarning(storage) && (
							<output className="block text-sm text-destructive">
								{deviceStorageWarning(storage)}
							</output>
						)}
						<Button asChild>
							<a
								href={requestUrl}
								download={`device-access-${manifest?.device_id}.json`}
							>
								Download public access request
							</a>
						</Button>
					</div>
				) : (
					<form
						className="space-y-3"
						onSubmit={(event) => {
							event.preventDefault();
							void create();
						}}
					>
						<label htmlFor={`${id}-bundle`} className="block space-y-1 text-sm">
							Public connection bundle from the owner
							<Input
								id={`${id}-bundle`}
								type="file"
								accept=".json,application/json"
								disabled={busy}
								onChange={async (event) => {
									const file = event.target.files?.[0];
									event.target.value = "";
									const version = ++importVersion.current;
									setBundle(undefined);
									setManifest(undefined);
									setExisting(undefined);
									setError(undefined);
									if (!file) return;
									try {
										if (file.size > 128 * 1024)
											throw new Error("The connection bundle exceeds 128 KiB.");
										const parsed = JSON.parse(await file.text()) as {
											version: number;
											receipt: DeviceReceipt;
											owner_controller_key: Ed25519PublicKey;
										};
										if (
											parsed.version !== 1 ||
											!parsed.receipt?.manifest_jws ||
											!parsed.owner_controller_key
										)
											throw new Error("Invalid public connection bundle.");
										const module = await loadDeviceCrypto();
										const accepted = module.verifyDeviceReceipt(
											parsed.receipt,
											parsed.receipt.manifest_jws,
											parsed.owner_controller_key,
										);
										if (accepted.api_base_url !== deviceApiBase(scope))
											throw new Error("This device belongs to another hub.");
										const stored = await readDeviceVault(
											scope,
											accepted.device_id,
										);
										if (current.current && version === importVersion.current) {
											setBundle(parsed);
											setManifest(accepted);
											setExisting(stored);
										}
									} catch (error) {
										if (current.current && version === importVersion.current)
											setError(
												error instanceof Error
													? error.message
													: "The owner connection bundle could not be verified.",
											);
									}
								}}
							/>
						</label>
						{manifest && (
							<div className="rounded border p-3 text-sm">
								<p>{manifest.name}</p>
								<p className="break-all">Owner: {manifest.owner_id}</p>
								<p className="break-all font-mono text-xs">
									{manifest.device_id}
								</p>
							</div>
						)}
						{reuse === "reusable" && existing && (
							<div className="space-y-2 rounded border p-3 text-sm">
								<p>
									This app already created a controller for this device. Its
									public access request can be downloaded again; no password is
									needed.
								</p>
								<Button
									type="button"
									variant="outline"
									onClick={() =>
										offer(
											accessRequest(
												scope.account,
												existing.controllerPublic.controller_key,
												existing.grantId,
											),
										)
									}
								>
									Recreate the access request
								</Button>
							</div>
						)}
						{reuse === "conflict" && (
							<p role="alert" className="text-sm text-destructive">
								This app already holds other keys for this device. Manage the
								device with them instead of requesting new access.
							</p>
						)}
						<label
							htmlFor={`${id}-password`}
							className="block space-y-1 text-sm"
						>
							Your management password
							<Input
								id={`${id}-password`}
								type="password"
								autoComplete="new-password"
								minLength={12}
								maxLength={4096}
								value={password}
								onChange={(event) => setPassword(event.target.value)}
								disabled={busy}
								required
							/>
						</label>
						<label htmlFor={`${id}-repeat`} className="block space-y-1 text-sm">
							Repeat password
							<Input
								id={`${id}-repeat`}
								type="password"
								autoComplete="new-password"
								value={repeat}
								onChange={(event) => setRepeat(event.target.value)}
								disabled={busy}
								required
							/>
						</label>
						<p className="text-xs text-muted-foreground">
							Use the connection bundle received directly from the owner. Your
							password and private keys are never included in the access
							request.
						</p>
						<Button
							type="submit"
							disabled={busy || !manifest || Boolean(reuse)}
						>
							{busy ? "Creating controller…" : "Create access request"}
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
