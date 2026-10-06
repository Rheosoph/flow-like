import type { IProfile } from "@flow-like/flow-like-ui";
import {
	type DeviceUnlockRequest,
	MODEL_UNLOCK_CLOSED,
	MODEL_UNLOCK_REQUESTED,
	type ModelUnlockPrompt,
	type SignedInAccount,
	lookupVault,
} from "@flow-like/flow-like-ui/components/settings/devices/models/unlock-prompt/model-unlock";
import type { ModelUnlockBridge } from "@flow-like/flow-like-ui/components/settings/devices/models/unlock-prompt/use-model-unlock";
import { readDeviceVault } from "@flow-like/flow-like-ui/lib/device-management/storage";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { AuthContextProps } from "react-oidc-context";

export interface UnlockHost {
	/** Read for each prompt, so a later sign-in counts. */
	auth(): AuthContextProps | undefined;
	profile(): Promise<IProfile | undefined>;
}

export function signedInAccount(
	auth: AuthContextProps | undefined,
): SignedInAccount | undefined {
	const profile = auth?.isAuthenticated ? auth.user?.profile : undefined;
	return profile?.sub
		? { issuer: profile.iss ?? "", account: profile.sub }
		: undefined;
}

/** The desktop connector's prompt: Tauri events in, its commands out, vaults from this window's device storage. */
export function desktopUnlockBridge(host: UnlockHost): ModelUnlockBridge {
	return {
		pending: () => invoke<ModelUnlockPrompt[]>("device_models_prompts"),
		async listen(onRequested, onClosed) {
			const requested = await listen<ModelUnlockPrompt>(
				MODEL_UNLOCK_REQUESTED,
				(event) => onRequested(event.payload),
			);
			try {
				const closed = await listen<string>(MODEL_UNLOCK_CLOSED, (event) =>
					onClosed(event.payload),
				);
				return () => {
					requested();
					closed();
				};
			} catch (error) {
				requested();
				throw error;
			}
		},
		vault: (deviceId) =>
			lookupVault(
				readDeviceVault,
				signedInAccount(host.auth()),
				host.profile,
				deviceId,
			),
		unlock: (unlock: DeviceUnlockRequest) =>
			invoke<void>("device_models_unlock", { unlock }),
		decline: (promptId) => invoke<void>("device_models_decline", { promptId }),
	};
}
