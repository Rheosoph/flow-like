import type { IApiState } from "../../state/backend-state/api-state";
import type { IProfile } from "../../types";
import { loadDeviceCrypto, unbase64url, withPassword } from "./crypto";
import {
	type ReleaseConfig,
	type ReleaseTarget,
	type VerifiedRelease,
	buildStandalonePackage,
	validateStandalonePackageSelection,
	verifyReleaseManifest,
} from "./package";
import { saveAccountRecovery } from "./recovery";
import {
	type DeviceAccountScope,
	type LocalDeviceVault,
	addDeviceVault,
} from "./storage";
import type { BrowserController, OnboardingManifest } from "./types";

export interface DeviceSetupInput {
	api: IApiState;
	profile: IProfile;
	scope: DeviceAccountScope;
	name: string;
	password: string;
	target: ReleaseTarget;
	mode: "binary" | "docker" | "both";
	release: ReleaseConfig;
	verifiedRelease: VerifiedRelease;
	signal?: AbortSignal;
	backupToAccount?: boolean;
}

export function assertEnrollmentTemplate(
	manifest: OnboardingManifest,
	expected: {
		name: string;
		ownerId: string;
		apiBase: string;
		bootstrap: string;
		controller: string;
		invitation: string;
	},
): void {
	if (
		manifest.version !== 1 ||
		manifest.name !== expected.name ||
		manifest.owner_id !== expected.ownerId ||
		manifest.api_base_url !== expected.apiBase ||
		manifest.bootstrap_key.x !== expected.bootstrap ||
		manifest.controller_key.x !== expected.controller ||
		manifest.owner_invitation_key.x !== expected.invitation ||
		!manifest.device_id ||
		!manifest.enrollment_id
	)
		throw new Error(
			"The enrollment response does not match the requested device.",
		);
}

export async function prepareDevicePackage(input: DeviceSetupInput): Promise<{
	package: Blob;
	backup: Blob;
	deviceId: string;
	accountBackup?: "saved" | "local_only";
}> {
	input.signal?.throwIfAborted();
	const verifiedRelease = await verifyReleaseManifest(
		input.verifiedRelease.manifestJws,
		input.release,
	);
	validateStandalonePackageSelection(
		verifiedRelease.manifest,
		input.target,
		input.mode,
	);
	const module = await loadDeviceCrypto();
	let reservation: string | undefined;
	let controller: BrowserController | undefined;
	const bootstrap = module.createBootstrapKey();
	const seed = unbase64url(bootstrap.secret_base64, 32);
	let exported = false;
	try {
		return await withPassword(input.password, async (password) => {
			if (input.signal?.aborted) throw new Error("Device setup cancelled.");
			const prepared = module.createOnboardingVaults(password);
			const publicKey = prepared.controller.public_bundle;
			controller = module.unlockControllerVault(
				publicKey.device_id,
				password,
				Uint8Array.from(prepared.controller.vault),
			);
			const apiBase = `${input.scope.apiOrigin.replace(/\/$/u, "")}/api/v1`;
			const response = await input.api.fetch<{
				enrollment_token: string;
				manifest: OnboardingManifest;
			}>(input.profile, "devices/enrollments", {
				method: "POST",
				headers: { "Content-Type": "application/json" },
				signal: input.signal,
				body: JSON.stringify({
					name: input.name,
					api_base_url: apiBase,
					bootstrap_key: bootstrap.public_key,
					controller_key: publicKey.controller_key,
					owner_invitation_key: prepared.invitation.public_key,
				}),
			});
			reservation = response.manifest.enrollment_id;
			assertEnrollmentTemplate(response.manifest, {
				name: input.name,
				ownerId: input.scope.account,
				apiBase,
				bootstrap: bootstrap.public_key.x,
				controller: publicKey.controller_key.x,
				invitation: prepared.invitation.public_key.x,
			});
			const completed = controller.completeOnboarding(
				response.manifest,
				password,
				Uint8Array.from(prepared.invitation.vault),
			);
			const secret = Array.from(seed);
			let archive: Blob;
			try {
				archive = await buildStandalonePackage({
					manifest: response.manifest,
					manifest_jws: completed.manifest_jws,
					enrollment_token: response.enrollment_token,
					bootstrap_secret: secret,
					target: input.target,
					mode: input.mode,
					release: input.release,
					verifiedRelease,
					signal: input.signal,
				});
			} finally {
				secret.fill(0);
				response.enrollment_token = "";
			}
			if (input.signal?.aborted) throw new Error("Device setup cancelled.");
			const record: LocalDeviceVault = {
				deviceId: response.manifest.device_id,
				controllerPublic: completed.controller.public_bundle,
				controllerVault: Uint8Array.from(completed.controller.vault),
				invitationVault: Uint8Array.from(completed.invitation_vault),
				manifestJws: completed.manifest_jws,
				grantId: "owner",
			};
			const backup = new Blob(
				[
					JSON.stringify({
						version: 1,
						apiOrigin: input.scope.apiOrigin,
						...record,
						controllerVault: Array.from(record.controllerVault),
						invitationVault: Array.from(record.invitationVault ?? []),
					}),
				],
				{ type: "application/json" },
			);
			await addDeviceVault(input.scope, record);
			exported = true;
			let accountBackup: "saved" | "local_only" | undefined;
			if (input.backupToAccount) {
				try {
					await saveAccountRecovery({
						api: input.api,
						profile: input.profile,
						scope: input.scope,
						deviceId: record.deviceId,
						password: input.password,
						signal: input.signal,
						crypto: module,
					});
					accountBackup = "saved";
				} catch {
					accountBackup = "local_only";
				}
			}
			return {
				package: archive,
				backup,
				deviceId: record.deviceId,
				accountBackup,
			};
		});
	} catch (error) {
		if (reservation && !exported) {
			try {
				await input.api.del(
					input.profile,
					`devices/enrollments/${encodeURIComponent(reservation)}`,
				);
			} catch {
				throw new Error(
					`Package creation failed and enrollment ${reservation} could not be cancelled. Cancel it through the hub or let its one-day credential expire before trying again.`,
				);
			}
		}
		throw error;
	} finally {
		seed.fill(0);
		bootstrap.secret_base64 = "";
		controller?.close();
		controller?.free();
	}
}
