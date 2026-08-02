"use client";

import { toast } from "sonner";
import { isTauri } from "./platform";

/**
 * Opens a URL outside the app: the system browser on desktop (Tauri), a new
 * tab on web. Use for checkout/billing URLs obtained asynchronously — those
 * bypass the global anchor interception.
 */
export async function openExternalUrl(
	url: string,
	announce?: string,
): Promise<void> {
	if (isTauri()) {
		const { openUrl } = await import("@tauri-apps/plugin-opener");
		await openUrl(url);
		if (announce) toast.info(`Opening ${announce} in your browser...`);
	} else {
		window.open(url, "_blank", "noopener,noreferrer");
		if (announce) toast.info(`Opening ${announce} in a new tab...`);
	}
}
