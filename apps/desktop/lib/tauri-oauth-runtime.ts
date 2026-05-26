import { fetch as tauriFetch } from "@tauri-apps/plugin-http";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { IOAuthRuntime } from "@flow-like/flow-like-ui";

export const tauriOAuthRuntime: IOAuthRuntime = {
	async openUrl(url: string): Promise<void> {
		await openUrl(url);
	},

	async httpPost(url: string, body: string, headers?: Record<string, string>) {
		const response = await tauriFetch(url, {
			method: "POST",
			headers: {
				...headers,
			},
			body,
		});

		return {
			ok: response.ok,
			status: response.status,
			json: () => response.json(),
			text: () => response.text(),
		};
	},

	async httpGet(url: string, headers?: Record<string, string>) {
		const response = await tauriFetch(url, {
			method: "GET",
			headers: {
				...headers,
			},
		});

		return {
			ok: response.ok,
			status: response.status,
			json: () => response.json(),
			text: () => response.text(),
		};
	},
};
