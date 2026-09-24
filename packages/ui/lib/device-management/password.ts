import { withPassword } from "./crypto";
import {
	type DeviceAccountScope,
	type LocalDeviceVault,
	acquireDeviceLock,
	replaceRewrappedVault,
} from "./storage";
import type { DeviceCrypto } from "./types";

/** Only ciphertext reaches storage. No request to the device or hub is needed. */
export async function changeDevicePassword(
	scope: DeviceAccountScope,
	previous: LocalDeviceVault,
	currentPassword: string,
	newPassword: string,
	crypto: Pick<DeviceCrypto, "rewrapControllerVaults">,
	signal?: AbortSignal,
): Promise<LocalDeviceVault> {
	if (currentPassword === newPassword)
		throw new Error("Choose a different password.");
	signal?.throwIfAborted();
	const release = await acquireDeviceLock(scope, previous.deviceId);
	try {
		signal?.throwIfAborted();
		return await withPassword(currentPassword, (current) =>
			withPassword(newPassword, async (next) => {
				const changed = crypto.rewrapControllerVaults(
					previous.deviceId,
					current,
					next,
					previous.controllerVault,
					previous.invitationVault ?? new Uint8Array(),
				);
				if (
					JSON.stringify(changed.controller.public_bundle) !==
						JSON.stringify(previous.controllerPublic) ||
					Boolean(changed.invitation) !== Boolean(previous.invitationVault)
				)
					throw new Error(
						"The unlocked vault does not match this device's saved identities.",
					);
				const replacement: LocalDeviceVault = {
					...previous,
					controllerVault: Uint8Array.from(changed.controller.vault),
					invitationVault: changed.invitation
						? Uint8Array.from(changed.invitation.vault)
						: undefined,
				};
				signal?.throwIfAborted();
				await replaceRewrappedVault(scope, previous, replacement);
				return replacement;
			}),
		);
	} finally {
		release();
	}
}
