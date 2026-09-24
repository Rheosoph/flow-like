"use client";
import { useEffect, useId, useRef, useState } from "react";
import {
	loadDeviceCrypto,
	withPassword,
} from "../../../lib/device-management/crypto";
import type { DeviceAccountScope } from "../../../lib/device-management/storage";
import {
	GroupMetricsReader,
	type ManagementCall,
	acknowledgeGroupTelemetry,
	digestText,
	readTelemetryChunks,
} from "../../../lib/device-management/telemetry";
import type {
	BrowserController,
	DeviceReceipt,
	OnboardingManifest,
	PolicyView,
	TelemetryMember,
	TelemetryRoster,
} from "../../../lib/device-management/types";
import { useBackend } from "../../../state/backend-state";
import type { IProfile } from "../../../types";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { Textarea } from "../../ui/textarea";

export function DeviceGroupMetrics({
	controller,
	account,
	manifest,
	receipt,
	scope,
	profile,
	invitationVault,
	run,
}: {
	controller: BrowserController;
	account: DeviceAccountScope;
	manifest: OnboardingManifest;
	receipt: DeviceReceipt;
	scope: string;
	profile: IProfile;
	invitationVault?: Uint8Array;
	run: <T>(operation: (call: ManagementCall) => Promise<T>) => Promise<T>;
}) {
	const id = useId();
	const backend = useBackend();
	const [error, setError] = useState<string>();
	const [busy, setBusy] = useState(false);
	const [password, setPassword] = useState("");
	const [packages, setPackages] = useState("");
	const [existingMembers, setExistingMembers] = useState<TelemetryMember[]>([]);
	const [removedMembers, setRemovedMembers] = useState<string[]>([]);
	const [request, setRequest] = useState("");
	const [joinSequence, setJoinSequence] = useState("0");
	const [sample, setSample] = useState<Record<string, unknown>>();
	const [last, setLast] = useState<{
		sequence: number;
		envelopeDigest: string;
		state: string;
		receiptPending: boolean;
	}>();
	const [live, setLive] = useState(false);
	const liveRead = useRef<() => Promise<void>>(async () => {});
	liveRead.current = async () => {
		if (busy) return;
		await execute(async (reader, call) => {
			for (let count = 0; count < 4; count++) {
				const result = await reader.read(call, Number(joinSequence));
				if (!result) break;
				if (alive.current) {
					setLast(result);
					setAcknowledged(false);
					if (result.sample) setSample(result.sample);
				}
				if (result.receiptPending || result.state === "removed") break;
			}
		});
	};
	useEffect(() => {
		if (!live) return;
		const timer = setInterval(() => void liveRead.current(), 3000);
		return () => clearInterval(timer);
	}, [live]);
	const [acknowledged, setAcknowledged] = useState(false);
	const alive = useRef(true);
	useEffect(() => {
		alive.current = true;
		return () => {
			alive.current = false;
		};
	}, []);
	async function execute(
		operation: (
			reader: GroupMetricsReader,
			call: ManagementCall,
		) => Promise<void>,
	) {
		if (busy) return;
		setBusy(true);
		setError(undefined);
		try {
			await run(async (call) => {
				const reader = await GroupMetricsReader.open(
					controller,
					account,
					manifest,
					receipt,
					scope,
				);
				try {
					await operation(reader, call);
				} finally {
					reader.close();
				}
			});
		} catch (error) {
			if (alive.current)
				setError(
					error instanceof Error ? error.message : "Group metrics failed.",
				);
		} finally {
			if (alive.current) setBusy(false);
		}
	}
	async function activate() {
		const secret = password;
		setPassword("");
		await execute(async (reader, call) => {
			if (!invitationVault)
				throw new Error("Only the owner can approve group membership.");
			const module = await loadDeviceCrypto();
			const now = Math.floor(Date.now() / 1000);
			const current = await readTelemetryChunks(
				call,
				{ type: "telemetry_roster_read", scope },
				16_384,
			);
			const previous = current.text
				? module.verifyHistoricalTelemetryRoster(
						current.text,
						manifest.owner_invitation_key,
					)
				: undefined;
			if (
				previous &&
				(previous.device_id !== manifest.device_id || previous.scope !== scope)
			)
				throw new Error("The device returned another group's roster.");
			const view = await backend.apiState.get<PolicyView>(
				profile,
				`devices/${encodeURIComponent(manifest.device_id)}/management/policy`,
			);
			let expiry = now + 86_400;
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
					throw new Error("The sharing policy could not be authenticated.");
				expiry = Math.min(expiry, policy.expires_at);
			} else if (view.version !== 0 || view.digest !== null)
				throw new Error("The sharing policy is incomplete.");
			if (
				view.version !== view.applied_version ||
				view.digest !== view.applied_digest
			)
				throw new Error(
					"Wait for the device to apply its current sharing policy first.",
				);
			const imported = packages.trim()
				? (JSON.parse(packages) as {
						member: TelemetryMember;
						key_package: string;
					}[])
				: [];
			if (!Array.isArray(imported) || imported.length > 30)
				throw new Error("Provide an array of at most 30 reader key packages.");
			const keyPackages = [...imported];
			if (!reader.position().joined)
				keyPackages.push(
					await reader.keyPackage(controller.publicBundle().telemetry_member),
				);
			const publisher = {
				endpoint_id: manifest.device_id,
				signing_key: receipt.identity.telemetry_key,
			};
			const members = [...(previous?.members ?? [publisher])].filter(
				(member) => !removedMembers.includes(member.endpoint_id),
			);
			for (const entry of keyPackages) {
				if (
					!entry?.member?.endpoint_id ||
					typeof entry.key_package !== "string"
				)
					throw new Error("Invalid reader key package.");
				if (
					members.some(
						(member) => member.endpoint_id === entry.member.endpoint_id,
					)
				)
					throw new Error(
						"This reader is already admitted; do not reuse its key package.",
					);
				members.push(entry.member);
			}
			const roster: TelemetryRoster = {
				version: 1,
				device_id: manifest.device_id,
				scope,
				policy_version: (previous?.policy_version ?? 0) + 1,
				previous_policy_digest: current.text
					? await digestText(current.text)
					: null,
				management_policy_digest: view.digest,
				publisher,
				members,
				issued_at: now,
				expires_at: expiry,
			};
			const policy_jws = await withPassword(secret, (bytes) =>
				module.signTelemetryRoster(roster, bytes, invitationVault),
			);
			const position = await readTelemetryChunks(call, {
				type: "telemetry_read",
				scope,
				sequence: 0,
				welcome: false,
			});
			const sequence = position.latest + 1;
			const command = {
				type: "telemetry_policy",
				scope,
				sequence,
				policy_jws,
				key_packages: keyPackages,
			};
			if (new TextEncoder().encode(JSON.stringify(command)).length > 13_000)
				throw new Error(
					"This membership change is too large for one management message. Add fewer readers at once.",
				);
			const response = await call(command);
			if (response.state !== "completed")
				throw new Error(
					"Group admission was rejected. Reload the current roster before retrying.",
				);
			if (alive.current) {
				setJoinSequence(String(sequence));
				setPackages("");
				setRequest(
					`Readers added by this policy should join Welcome sequence ${sequence}.`,
				);
			}
		});
	}
	return (
		<details className="rounded border p-3">
			<summary className="cursor-pointer font-medium">
				Encrypted group metrics
			</summary>
			<div className="mt-3 space-y-3">
				<p className="text-sm text-muted-foreground">
					This browser has its own MLS reader. Share its public key package with
					the owner, then join the welcome sequence they provide. Local receive
					state is encrypted and committed before metrics are shown.
				</p>
				<div className="flex flex-wrap gap-2">
					<Button
						variant="outline"
						disabled={busy}
						onClick={() =>
							void execute(async (reader) => {
								const value = await reader.keyPackage(
									controller.publicBundle().telemetry_member,
								);
								if (alive.current) setRequest(JSON.stringify([value], null, 2));
							})
						}
					>
						Create reader request
					</Button>
					<label
						htmlFor={`${id}-sequence`}
						className="flex items-center gap-2 text-sm"
					>
						Welcome sequence
						<Input
							id={`${id}-sequence`}
							type="number"
							min={0}
							value={joinSequence}
							onChange={(event) => setJoinSequence(event.target.value)}
							className="w-28"
						/>
					</label>
					<Button
						disabled={busy}
						onClick={() =>
							void execute(async (reader, call) => {
								const result = await reader.read(call, Number(joinSequence));
								if (alive.current) {
									if (result) {
										setLast(result);
										setAcknowledged(false);
										if (result.sample) setSample(result.sample);
									} else setRequest("No newer group metrics are available.");
								}
							})
						}
					>
						Read next group message
					</Button>
				</div>
				<label className="flex items-center gap-2 text-sm">
					<input
						type="checkbox"
						checked={live}
						onChange={(event) => setLive(event.target.checked)}
					/>
					Read group metrics while connected and send signed delivery receipts
					after local persistence.
				</label>
				{request && (
					<Textarea
						readOnly
						value={request}
						rows={4}
						aria-label="Public group reader request or admission status"
					/>
				)}
				{last && (
					<p className="text-xs">
						Committed sequence {last.sequence}: {last.state}
						{last.receiptPending
							? ". Delivery receipt is pending; reconnect to retry."
							: ". Delivery receipt confirmed."}
					</p>
				)}
				{sample && (
					<pre className="max-h-48 overflow-auto rounded bg-muted p-3 text-xs">
						{JSON.stringify(sample, null, 2)}
					</pre>
				)}
				{invitationVault && account.account === manifest.owner_id && (
					<>
						<Button
							variant="outline"
							disabled={busy}
							onClick={() =>
								void execute(async (_reader, call) => {
									const module = await loadDeviceCrypto();
									const previous = await readTelemetryChunks(
										call,
										{ type: "telemetry_roster_read", scope },
										16_384,
									);
									const roster = previous.text
										? module.verifyHistoricalTelemetryRoster(
												previous.text,
												manifest.owner_invitation_key,
											)
										: undefined;
									if (
										roster &&
										(roster.device_id !== manifest.device_id ||
											roster.scope !== scope)
									)
										throw new Error("Another group's roster was returned.");
									if (alive.current) {
										setExistingMembers(roster?.members ?? []);
										setRemovedMembers([]);
									}
								})
							}
						>
							Review current group readers
						</Button>
						{existingMembers
							.filter((member) => member.endpoint_id !== manifest.device_id)
							.map((member) => (
								<label
									key={member.endpoint_id}
									className="flex items-center gap-2 text-xs"
								>
									<input
										type="checkbox"
										checked={removedMembers.includes(member.endpoint_id)}
										onChange={(event) =>
											setRemovedMembers((value) =>
												event.target.checked
													? [...value, member.endpoint_id]
													: value.filter((id) => id !== member.endpoint_id),
											)
										}
									/>
									Remove reader {member.endpoint_id}
								</label>
							))}

						<label
							htmlFor={`${id}-packages`}
							className="block space-y-1 text-sm"
						>
							Additional public reader requests
							<Textarea
								id={`${id}-packages`}
								value={packages}
								onChange={(event) => setPackages(event.target.value)}
								maxLength={12_000}
								placeholder="Optional array of reader key packages"
								rows={4}
							/>
						</label>
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
						<Button
							disabled={busy || !password}
							onClick={() => void activate()}
						>
							Approve readers and renew roster
						</Button>
						{last && last.state !== "joined" && (
							<div className="space-y-2">
								<label className="flex items-start gap-2 text-sm">
									<input
										type="checkbox"
										checked={acknowledged}
										onChange={(event) => setAcknowledged(event.target.checked)}
									/>
									Every intended reader has durably received sequence{" "}
									{last.sequence} and all earlier messages. Removing these
									deliveries may require absent readers to rejoin.
								</label>
								<Button
									variant="outline"
									disabled={busy || !acknowledged}
									onClick={() =>
										void execute(async (_reader, call) => {
											await acknowledgeGroupTelemetry(
												call,
												scope,
												last.sequence,
												last.envelopeDigest,
											);
											if (alive.current) {
												setLast(undefined);
												setAcknowledged(false);
											}
										})
									}
								>
									Remove acknowledged deliveries
								</Button>
							</div>
						)}
					</>
				)}
				{error && (
					<p role="alert" className="text-sm text-destructive">
						{error}
					</p>
				)}
			</div>
		</details>
	);
}
