"use client";

import { useBackend } from "@flow-like/flow-like-ui";
import { ModelUnlockPrompts } from "@flow-like/flow-like-ui/components/settings/devices/models/unlock-prompt/model-unlock-dialog";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { useContext, useEffect, useMemo, useRef, useState } from "react";
import { AuthContext } from "react-oidc-context";
import { desktopUnlockBridge } from "../lib/device-model-unlock";
import { isMobileDevice, isTauriRuntime } from "../lib/platform";

/** Answers the desktop connector when a run wants a model on a locked device; only the main window asks. */
export function DeviceModelUnlockProvider() {
	const backend = useBackend();
	const auth = useContext(AuthContext);
	const host = useRef({ backend, auth });
	host.current = { backend, auth };
	const [main, setMain] = useState(false);

	useEffect(() => {
		setMain(
			isTauriRuntime() &&
				!isMobileDevice() &&
				getCurrentWebviewWindow().label === "main",
		);
	}, []);

	const bridge = useMemo(
		() =>
			desktopUnlockBridge({
				auth: () => host.current.auth,
				profile: () => host.current.backend.userState.getProfile(),
			}),
		[],
	);

	return main ? <ModelUnlockPrompts bridge={bridge} /> : null;
}
