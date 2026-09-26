"use client";
import { useEffect, useId, useRef, useState } from "react";
import {
	loadDeviceCrypto,
	withPassword,
} from "../../../lib/device-management/crypto";
import {
	type ManagementCall,
	digestText,
	readTelemetryChunks,
} from "../../../lib/device-management/telemetry";
import type {
	ArchiveRecipient,
	ArchiveRoster,
	BrowserController,
	DeviceReceipt,
	EncryptedArchive,
	OnboardingManifest,
	PolicyView,
} from "../../../lib/device-management/types";
import { useBackend } from "../../../state/backend-state";
import type { IProfile } from "../../../types";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { Textarea } from "../../ui/textarea";

export function DeviceArchiveHistory({
	controller,
	account,
	manifest,
	receipt,
	scope,
	projectId,
	profile,
	invitationVault,
	run,
}: {
	controller: BrowserController;
	account: string;
	manifest: OnboardingManifest;
	receipt: DeviceReceipt;
	scope: string;
	projectId?: string;
	profile: IProfile;
	invitationVault?: Uint8Array;
	run: <T>(operation: (call: ManagementCall) => Promise<T>) => Promise<T>;
}) {
	const id = useId();
	const backend = useBackend();
	const [kind, setKind] = useState<"logs" | "metrics">("logs");
	const [password, setPassword] = useState("");
	const [recipients, setRecipients] = useState("");
	const [sequence, setSequence] = useState("0");
	const [archiveId, setArchiveId] = useState("");
	const [cloud, setCloud] = useState<
		{
			archive_id: string;
			sequence: number;
			created_at: number;
			expires_at: number;
		}[]
	>([]);
	const [after, setAfter] = useState(0);
	const [plaintext, setPlaintext] = useState<string>();
	const [error, setError] = useState<string>();
	const [status, setStatus] = useState<string>();
	const [busy, setBusy] = useState(false);
	const alive = useRef(true);
	useEffect(() => {
		alive.current = true;
		return () => {
			alive.current = false;
		};
	}, []);
	const publicKey = controller.publicBundle();
	const ownRecipient: ArchiveRecipient = {
		recipient_id: publicKey.controller_key.x,
		user_id: account,
		public_key: publicKey.archive_key,
	};
	async function execute(operation: () => Promise<void>) {
		if (busy) return;
		setBusy(true);
		setError(undefined);
		try {
			await operation();
		} catch (error) {
			if (alive.current)
				setError(
					error instanceof Error ? error.message : "Retained history failed.",
				);
		} finally {
			if (alive.current) setBusy(false);
		}
	}
	async function approve() {
		const secret = password;
		setPassword("");
		await execute(() =>
			run(async (call) => {
				if (!invitationVault || account !== manifest.owner_id)
					throw new Error("Only the owner can approve archive recipients.");
				const module = await loadDeviceCrypto();
				const now = Math.floor(Date.now() / 1000);
				const previous = await readTelemetryChunks(
					call,
					{ type: "archive_roster_read", scope, kind },
					16_384,
				);
				const roster = previous.text
					? module.verifyArchiveRosterHead(
							previous.text,
							manifest.owner_invitation_key,
						)
					: undefined;
				if (
					roster &&
					(roster.device_id !== manifest.device_id ||
						roster.scope !== scope ||
						roster.kind !== kind)
				)
					throw new Error("The archive roster belongs to another audience.");
				const requested = recipients.trim()
					? (JSON.parse(recipients) as ArchiveRecipient[])
					: [];
				if (!Array.isArray(requested) || requested.length > 31)
					throw new Error(
						"Provide an array of at most 31 additional recipients.",
					);
				const approved = [
					ownRecipient,
					...requested.filter(
						(item) => item.recipient_id !== ownRecipient.recipient_id,
					),
				];
				const view = await backend.apiState.get<PolicyView>(
					profile,
					`devices/${encodeURIComponent(manifest.device_id)}/management/policy`,
				);
				let expires = now + 86_400;
				if (view.policy_jws) {
					const policy = module.verifyManagementPolicy(
						view.policy_jws,
						manifest.owner_invitation_key,
					);
					if (
						policy.device_id !== manifest.device_id ||
						policy.policy_version !== view.version ||
						(await digestText(view.policy_jws)) !== view.digest
					)
						throw new Error(
							"The management policy could not be authenticated.",
						);
					expires = Math.min(expires, policy.expires_at);
					for (const recipient of approved) {
						if (recipient.user_id === manifest.owner_id) continue;
						const matching = policy.grants.filter(
							(grant) =>
								grant.user_id === recipient.user_id &&
								grant.capabilities.includes(kind) &&
								grant.expires_at > now &&
								(grant.scope.kind === "device" ||
									(grant.scope.kind === "placement" &&
										grant.scope.project_id === projectId &&
										grant.scope.placement_id === scope) ||
									(grant.scope.kind === "project" &&
										grant.scope.project_id === projectId)),
						);
						if (!matching.length)
							throw new Error(
								`Recipient ${recipient.user_id} has no current ${kind} permission for this scope.`,
							);
						expires = Math.min(
							expires,
							Math.max(...matching.map((grant) => grant.expires_at)),
						);
					}
				} else if (
					view.version !== 0 ||
					view.digest !== null ||
					approved.some((recipient) => recipient.user_id !== manifest.owner_id)
				)
					throw new Error(
						"Approve recipient management access before retaining history for them.",
					);
				if (
					view.version !== view.applied_version ||
					view.digest !== view.applied_digest
				)
					throw new Error(
						"Wait for the device to apply its current management policy.",
					);
				const next: ArchiveRoster = {
					version: 1,
					device_id: manifest.device_id,
					scope,
					project_id: scope === "device" ? null : (projectId ?? null),
					kind,
					policy_version: (roster?.policy_version ?? 0) + 1,
					previous_policy_digest: previous.text
						? await digestText(previous.text)
						: null,
					management_policy_digest: view.digest,
					recipients: approved,
					issued_at: now,
					expires_at: expires,
				};
				const policy_jws = await withPassword(secret, (bytes) =>
					module.signArchiveRoster(next, bytes, invitationVault),
				);
				if (policy_jws.length > 13_000)
					throw new Error(
						"This archive roster exceeds the management message size. Approve fewer recipients.",
					);
				const response = await call({ type: "archive_policy", policy_jws });
				if (response.state !== "completed" && response.state !== "accepted")
					throw new Error("The device rejected the archive roster.");
				if (alive.current)
					setStatus(
						`Archive recipient policy ${next.policy_version} applied. It expires ${new Date(expires * 1000).toLocaleString()}.`,
					);
			}),
		);
	}
	function decrypt(bundle: EncryptedArchive) {
		const bytes = controller.openArchive(
			{
				device_id: manifest.device_id,
				scope,
				kind,
				owner_invitation_key: manifest.owner_invitation_key,
				device_signing_key: receipt.identity.telemetry_key,
			},
			bundle,
			ownRecipient.recipient_id,
		);
		try {
			const value = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
			if (alive.current) setPlaintext(value);
		} finally {
			bytes.fill(0);
		}
	}
	return (
		<details className="rounded border p-3">
			<summary className="cursor-pointer font-medium">
				Retained encrypted history
			</summary>
			<div className="mt-3 space-y-3">
				<p className="text-sm text-muted-foreground">
					Each retained segment has its own key, encrypted separately for
					approved recipients. New downloads require current permission.
					Previously downloaded segments remain readable with their recipient
					key. Cloud storage follows the hub's tier allowance.
				</p>
				<label className="block text-sm">
					History type
					<select
						value={kind}
						disabled={busy}
						onChange={(event) => {
							setKind(event.target.value as typeof kind);
							setPlaintext(undefined);
							setStatus(undefined);
							setCloud([]);
							setAfter(0);
							setSequence("0");
						}}
						className="ml-2 rounded border bg-background p-2"
					>
						<option value="logs">Logs</option>
						<option value="metrics">Metrics</option>
					</select>
				</label>
				<label htmlFor={`${id}-request`} className="block space-y-1 text-sm">
					Your public archive recipient request
					<Textarea
						id={`${id}-request`}
						readOnly
						value={JSON.stringify([ownRecipient], null, 2)}
						rows={3}
					/>
				</label>
				{invitationVault && account === manifest.owner_id && (
					<>
						<label
							htmlFor={`${id}-recipients`}
							className="block space-y-1 text-sm"
						>
							Complete additional recipient list
							<Textarea
								id={`${id}-recipients`}
								value={recipients}
								maxLength={12_000}
								onChange={(event) => setRecipients(event.target.value)}
								rows={4}
								placeholder="[] means only your own controller"
							/>
						</label>
						<p className="text-xs text-muted-foreground">
							Approval replaces the previous recipient list. Your own controller
							is included. Paste every other recipient who should read future
							segments.
						</p>
						<label
							htmlFor={`${id}-password`}
							className="block space-y-1 text-sm"
						>
							Owner password
							<Input
								id={`${id}-password`}
								type="password"
								autoComplete="current-password"
								value={password}
								onChange={(event) => setPassword(event.target.value)}
							/>
						</label>
						<Button disabled={busy || !password} onClick={() => void approve()}>
							Approve retained history recipients
						</Button>
					</>
				)}
				<div className="flex flex-wrap gap-2">
					<Input
						aria-label="Retained segment sequence"
						type="number"
						min={0}
						value={sequence}
						onChange={(event) => setSequence(event.target.value)}
						className="w-36"
					/>
					<Button
						variant="outline"
						disabled={busy}
						onClick={() =>
							void execute(() =>
								run(async (call) => {
									const selected = Number(sequence);
									if (!Number.isSafeInteger(selected) || selected < 0)
										throw new Error("Enter a valid segment sequence.");
									const result = await readTelemetryChunks(call, {
										type: "archive_read",
										scope,
										kind,
										sequence: selected,
									});
									if (!result.text) {
										if (alive.current)
											setStatus(
												"No retained segment is available for this sequence.",
											);
										return;
									}
									decrypt(JSON.parse(result.text) as EncryptedArchive);
									if (alive.current) setSequence(String(result.sequence + 1));
								}),
							)
						}
					>
						Read from device
					</Button>
				</div>
				<div className="flex gap-2">
					<Input
						aria-label="Cloud archive ID"
						placeholder="Archive ID from a retained segment"
						value={archiveId}
						onChange={(event) => setArchiveId(event.target.value)}
					/>
					<Button
						variant="outline"
						disabled={busy || !archiveId}
						onClick={() =>
							void execute(async () => {
								const bundle = await backend.apiState.get<EncryptedArchive>(
									profile,
									`devices/${encodeURIComponent(manifest.device_id)}/archives/${encodeURIComponent(archiveId)}`,
								);
								if (alive.current) decrypt(bundle);
							})
						}
					>
						Read from cloud
					</Button>
				</div>
				<Button
					variant="outline"
					disabled={busy}
					onClick={() =>
						void execute(async () => {
							const query = new URLSearchParams({
								scope,
								kind,
								after: String(after),
							});
							const result = await backend.apiState.get<{
								archives: {
									archive_id: string;
									sequence: number;
									created_at: number;
									expires_at: number;
									manifest_jws: string;
									roster_jws: string;
								}[];
								next: number;
							}>(
								profile,
								`devices/${encodeURIComponent(manifest.device_id)}/archives?${query}`,
							);
							if (
								!Array.isArray(result.archives) ||
								result.archives.length > 100 ||
								!Number.isSafeInteger(result.next) ||
								result.next < after
							)
								throw new Error("Invalid cloud history listing.");
							if (alive.current) {
								setCloud(
									result.archives.map(
										({ archive_id, sequence, created_at, expires_at }) => ({
											archive_id,
											sequence,
											created_at,
											expires_at,
										}),
									),
								);
								setAfter(result.next);
								if (!result.archives.length)
									setStatus(
										"No more retained segments are available in this cloud history.",
									);
							}
						})
					}
				>
					{after ? "List next cloud segments" : "List cloud segments"}
				</Button>
				{cloud.length > 0 && (
					<div className="space-y-1">
						{cloud.map((row) => (
							<Button
								key={row.archive_id}
								variant="ghost"
								disabled={busy}
								className="w-full justify-start text-xs"
								onClick={() => {
									setArchiveId(row.archive_id);
									void execute(async () => {
										const bundle = await backend.apiState.get<EncryptedArchive>(
											profile,
											`devices/${encodeURIComponent(manifest.device_id)}/archives/${encodeURIComponent(row.archive_id)}`,
										);
										if (alive.current) decrypt(bundle);
									});
								}}
							>
								Read segment {row.sequence} · {row.archive_id}
							</Button>
						))}
					</div>
				)}
				{plaintext && (
					<pre className="max-h-72 overflow-auto rounded bg-muted p-3 text-xs">
						{plaintext}
					</pre>
				)}
				{status && <output className="block text-sm">{status}</output>}
				{error && (
					<p role="alert" className="text-sm text-destructive">
						{error}
					</p>
				)}
			</div>
		</details>
	);
}
