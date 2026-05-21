import type { IOAuthRuntime } from "@flow-like/flow-like-ui";

export const webOAuthRuntime: IOAuthRuntime = {
	async openUrl(url: string): Promise<void> {
		window.open(url, "_blank");
	},

	async httpPost(url: string, body: string, headers?: Record<string, string>) {
		const response = await fetch(url, {
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
		const response = await fetch(url, {
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

// Re-export with old name for backwards compatibility
export const tauriOAuthRuntime = webOAuthRuntime;
