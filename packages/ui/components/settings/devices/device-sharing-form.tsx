"use client";

import { useEffect, useId, useRef, useState } from "react";
import { z } from "zod";
import {
	base64url,
	loadDeviceCrypto,
	withPassword,
} from "../../../lib/device-management/crypto";
import type {
	Capability,
	DeviceReceipt,
	ManagementGrant,
	ManagementPolicy,
	OnboardingManifest,
	PolicyView,
} from "../../../lib/device-management/types";
import { useBackend } from "../../../state/backend-state";
import type { IProfile } from "../../../types";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { Textarea } from "../../ui/textarea";

const recipientsSchema = z
	.array(
		z
			.object({
				user_id: z.string().min(1).max(128),
				grant_id: z
					.string()
					.regex(/^[A-Za-z0-9_:.\-]{1,128}$/u)
					.optional(),
				controller_key: z
					.object({
						kty: z.literal("OKP"),
						crv: z.literal("Ed25519"),
						x: z.string().regex(/^[A-Za-z0-9_-]{43}$/u),
					})
					.strict(),
			})
			.strict(),
	)
	.min(1)
	.max(24);
const capabilities: Capability[] = [
	"status",
	"metrics",
	"logs",
	"deploy",
	"start",
	"stop",
	"restart",
	"remove",
	"scale",
	"update_agent",
	"reboot",
];

export function DeviceSharingForm({
	profile,
	manifest,
	receipt,
	invitationVault,
}: {
	profile: IProfile;
	manifest: OnboardingManifest;
	receipt: DeviceReceipt;
	invitationVault: Uint8Array;
}) {
	const backend = useBackend();
	const formId = useId();
	const [recipients, setRecipients] = useState("");
	const [password, setPassword] = useState("");
	const [scope, setScope] = useState<"device" | "project" | "placement">(
		"device",
	);
	const [project, setProject] = useState("");
	const [placement, setPlacement] = useState("");
	const [selected, setSelected] = useState<Capability[]>([
		"status",
		"metrics",
		"logs",
	]);
	const [groupId, setGroupId] = useState("");
	const [groupVersion, setGroupVersion] = useState("1");
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string>();
	const [saved, setSaved] = useState<PolicyView>();
	const [activeGrants, setActiveGrants] = useState<ManagementGrant[]>();
	const [bundleUrl, setBundleUrl] = useState<string>();
	useEffect(() => {
		const url = URL.createObjectURL(
			new Blob(
				[
					JSON.stringify({
						version: 1,
						receipt,
						owner_controller_key: manifest.controller_key,
					}),
				],
				{ type: "application/json" },
			),
		);
		setBundleUrl(url);
		return () => URL.revokeObjectURL(url);
	}, [receipt, manifest.controller_key]);
	const current = useRef(true);
	useEffect(() => {
		current.current = true;
		return () => {
			current.current = false;
		};
	}, []);
	async function loadGrants() {
		if (busy) return;
		setBusy(true);
		setError(undefined);
		try {
			const module = await loadDeviceCrypto();
			const previous = await backend.apiState.get<PolicyView>(
				profile,
				`devices/${encodeURIComponent(manifest.device_id)}/management/policy`,
			);
			if (!previous.policy_jws) {
				if (previous.version !== 0 || previous.digest !== null)
					throw new Error("The sharing policy is incomplete.");
				if (current.current) setActiveGrants([]);
				return;
			}
			const accepted = module.verifyManagementPolicy(
				previous.policy_jws,
				manifest.owner_invitation_key,
			);
			const digest = base64url(
				new Uint8Array(
					await crypto.subtle.digest(
						"SHA-256",
						new TextEncoder().encode(previous.policy_jws),
					),
				),
			);
			if (
				accepted.device_id !== manifest.device_id ||
				accepted.policy_version !== previous.version ||
				digest !== previous.digest
			)
				throw new Error("The sharing policy could not be authenticated.");
			if (current.current)
				setActiveGrants(
					accepted.grants.filter(
						(grant) => grant.expires_at > Date.now() / 1000,
					),
				);
		} catch (error) {
			if (current.current)
				setError(
					error instanceof Error
						? error.message
						: "Sharing policy could not be read.",
				);
		} finally {
			if (current.current) setBusy(false);
		}
	}
	async function approve(removeGrant?: string) {
		if (busy) return;
		setBusy(true);
		setError(undefined);
		setSaved(undefined);
		const secret = password;
		setPassword("");
		try {
			const users = removeGrant
				? []
				: recipientsSchema.parse(JSON.parse(recipients));
			if (new Set(users.map((user) => user.user_id)).size !== users.length)
				throw new Error("List each recipient once.");
			if (!removeGrant && selected.length === 0)
				throw new Error("Choose at least one permission.");
			if (
				!removeGrant &&
				scope !== "device" &&
				(!project ||
					selected.includes("reboot") ||
					selected.includes("update_agent"))
			)
				throw new Error(
					"Select a project and remove host reboot/update from project permissions.",
				);
			if (!removeGrant && scope === "placement" && !placement)
				throw new Error("Enter a placement ID.");
			if (
				!removeGrant &&
				groupId &&
				(!Number.isSafeInteger(Number(groupVersion)) ||
					Number(groupVersion) < 1)
			)
				throw new Error("Enter the approved group roster version.");
			const module = await loadDeviceCrypto();
			const path = `devices/${encodeURIComponent(manifest.device_id)}/management/policy`;
			const previous = await backend.apiState.get<PolicyView>(profile, path);
			let grants: ManagementGrant[] = [];
			const now = Math.floor(Date.now() / 1000);
			if (previous.policy_jws) {
				const accepted = module.verifyManagementPolicy(
					previous.policy_jws,
					manifest.owner_invitation_key,
				);
				const digest = base64url(
					new Uint8Array(
						await crypto.subtle.digest(
							"SHA-256",
							new TextEncoder().encode(previous.policy_jws),
						),
					),
				);
				if (
					accepted.device_id !== manifest.device_id ||
					accepted.policy_version !== previous.version ||
					digest !== previous.digest
				)
					throw new Error(
						"The current sharing policy could not be authenticated.",
					);
				grants = accepted.grants.filter((grant) => grant.expires_at > now);
			} else if (previous.version !== 0 || previous.digest !== null)
				throw new Error("The current sharing policy is incomplete.");
			if (removeGrant) {
				if (!grants.some((grant) => grant.grant_id === removeGrant))
					throw new Error(
						"This grant is no longer active. Reload sharing access.",
					);
				grants = grants.filter((grant) => grant.grant_id !== removeGrant);
			}
			const resolvedScope: ManagementGrant["scope"] =
				scope === "device"
					? { kind: "device" }
					: scope === "project"
						? { kind: "project", project_id: project }
						: {
								kind: "placement",
								project_id: project,
								placement_id: placement,
							};
			for (const recipient of users)
				grants.push({
					grant_id: recipient.grant_id ?? crypto.randomUUID(),
					user_id: recipient.user_id,
					controller_key: recipient.controller_key,
					scope: resolvedScope,
					capabilities: selected,
					expires_at: now + 86_400,
					group_id: groupId || null,
					group_version: groupId ? Number(groupVersion) : null,
				});
			if (grants.length > 24)
				throw new Error(
					"This policy would exceed 24 grants. Revoke unused access first.",
				);
			const policy: ManagementPolicy = {
				version: 1,
				device_id: manifest.device_id,
				policy_version: previous.version + 1,
				previous_policy_digest: previous.digest,
				grants,
				issued_at: now,
				expires_at: now + 31 * 86_400,
			};
			const signed = await withPassword(secret, (bytes) =>
				module.signManagementPolicy(policy, bytes, invitationVault),
			);
			if (!current.current) return;
			const result = await backend.apiState.put<PolicyView>(profile, path, {
				policy_jws: signed,
			});
			if (current.current) {
				setSaved(result);
				setActiveGrants(grants);
				setRecipients("");
			}
		} catch (error) {
			if (current.current)
				setError(
					error instanceof z.ZodError
						? "Enter recipient account IDs and their Ed25519 controller public keys."
						: error instanceof Error
							? error.message
							: "Device sharing could not be approved.",
				);
		} finally {
			if (current.current) setBusy(false);
		}
	}
	return (
		<form
			className="space-y-3 rounded border p-4"
			onSubmit={(event) => {
				event.preventDefault();
				void approve();
			}}
		>
			<h3 className="font-semibold">Approve device sharing</h3>
			{bundleUrl && (
				<Button variant="outline" asChild>
					<a
						href={bundleUrl}
						download={`flow-like-connection-${manifest.device_id}.json`}
					>
						Download public connection bundle
					</a>
				</Button>
			)}
			<p className="text-sm text-muted-foreground">
				Each recipient creates their own password-protected controller key.
				Approve their public key and exact account ID here. New access lasts one
				day.
			</p>
			<label
				htmlFor={`${formId}-recipients`}
				className="block space-y-1 text-sm"
			>
				Recipients and controller public keys
				<Textarea
					id={`${formId}-recipients`}
					rows={4}
					value={recipients}
					onChange={(event) => setRecipients(event.target.value)}
					maxLength={12_000}
					placeholder={
						'[{"user_id":"account-id","controller_key":{"kty":"OKP","crv":"Ed25519","x":"recipient public key"}}]'
					}
					required
					disabled={busy}
				/>
			</label>
			<label className="block text-sm">
				Access scope
				<select
					className="ml-2 rounded border bg-background p-2"
					value={scope}
					onChange={(event) => setScope(event.target.value as typeof scope)}
					disabled={busy}
				>
					<option value="device">Whole device</option>
					<option value="project">One project</option>
					<option value="placement">One placement</option>
				</select>
			</label>
			{scope !== "device" && (
				<Input
					aria-label="Shared project ID"
					placeholder="Project ID"
					value={project}
					onChange={(event) => setProject(event.target.value)}
					required
				/>
			)}
			{scope === "placement" && (
				<Input
					aria-label="Shared placement ID"
					placeholder="Placement ID"
					value={placement}
					onChange={(event) => setPlacement(event.target.value)}
					required
				/>
			)}
			<fieldset className="flex flex-wrap gap-3">
				<legend className="mb-2 text-sm">Permissions</legend>
				{capabilities.map((capability) => (
					<label className="flex items-center gap-1 text-sm" key={capability}>
						<input
							type="checkbox"
							checked={selected.includes(capability)}
							onChange={(event) =>
								setSelected((value) =>
									event.target.checked
										? [...value, capability]
										: value.filter((cap) => cap !== capability),
								)
							}
							disabled={
								busy ||
								(["reboot", "update_agent"].includes(capability) &&
									scope !== "device")
							}
						/>
						{capability}
					</label>
				))}
			</fieldset>
			<details>
				<summary className="cursor-pointer text-sm">
					Record an approved group roster
				</summary>
				<p className="my-2 text-xs text-muted-foreground">
					Only the listed recipients receive access. Later additions to the
					group need another owner approval.
				</p>
				<div className="grid grid-cols-2 gap-2">
					<Input
						aria-label="Group ID"
						placeholder="Group ID (optional)"
						value={groupId}
						onChange={(event) => setGroupId(event.target.value)}
					/>
					<Input
						aria-label="Group roster version"
						placeholder="Roster version"
						value={groupVersion}
						onChange={(event) => setGroupVersion(event.target.value)}
					/>
				</div>
			</details>
			<label htmlFor={`${formId}-password`} className="block space-y-1 text-sm">
				Owner password
				<Input
					id={`${formId}-password`}
					type="password"
					autoComplete="current-password"
					value={password}
					onChange={(event) => setPassword(event.target.value)}
					disabled={busy}
					required
				/>
			</label>
			<Button type="submit" disabled={busy}>
				{busy ? "Approving access…" : "Approve these recipients"}
			</Button>
			<details>
				<summary className="cursor-pointer text-sm">
					Review or revoke shared access
				</summary>
				<div className="mt-3 space-y-2">
					<Button
						type="button"
						variant="outline"
						disabled={busy}
						onClick={() => void loadGrants()}
					>
						Load current grants
					</Button>
					{activeGrants?.length === 0 && (
						<p className="text-sm">No active shared grants.</p>
					)}
					{activeGrants?.map((grant) => (
						<div
							key={grant.grant_id}
							className="flex items-center justify-between gap-2 rounded border p-2 text-sm"
						>
							<div>
								<p>{grant.user_id}</p>
								<p className="text-xs text-muted-foreground">
									{grant.scope.kind} · {grant.capabilities.join(", ")}
								</p>
							</div>
							<Button
								type="button"
								variant="outline"
								disabled={busy || !password}
								onClick={() => void approve(grant.grant_id)}
							>
								Revoke with owner password
							</Button>
						</div>
					))}
					<p className="text-xs text-muted-foreground">
						Revocation stops access when the device applies the new policy.
						Group telemetry pauses until you approve an updated recipient
						roster.
					</p>
				</div>
			</details>
			{saved && (
				<output className="block text-sm">
					Policy {saved.version} saved.{" "}
					{saved.applied_version === saved.version
						? "The device has applied it."
						: "Waiting for the device to apply it."}
				</output>
			)}
			{error && (
				<p role="alert" className="text-sm text-destructive">
					{error}
				</p>
			)}
		</form>
	);
}
