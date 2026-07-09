import type { IInteractionRequest } from "../../../lib/schema/interaction";

/** Minimal profile shape needed to resolve the interaction-response API base url. */
export interface InteractionResponderProfile {
	hub?: string;
	secure?: boolean;
}

/**
 * Submit a user's response to a chat interaction (single/multiple choice, form).
 *
 * Cloud runs carry a `responder_jwt` and are answered through the hub's REST API; local runs are
 * answered through the Tauri `respond_to_interaction` command. Throws on failure — callers own the
 * optimistic state update and error surface.
 */
export async function submitInteractionResponse(
	interaction: IInteractionRequest,
	value: unknown,
	profile: InteractionResponderProfile | undefined,
): Promise<void> {
	if (interaction.responder_jwt) {
		let baseUrl = profile?.hub ?? "api.flow-like.com";
		if (typeof process !== "undefined" && process.env?.NEXT_PUBLIC_API_URL) {
			baseUrl = process.env.NEXT_PUBLIC_API_URL;
		}
		if (!baseUrl.startsWith("http://") && !baseUrl.startsWith("https://")) {
			baseUrl =
				profile?.secure === false ? `http://${baseUrl}` : `https://${baseUrl}`;
		}
		if (!baseUrl.endsWith("/")) baseUrl += "/";
		const url = `${baseUrl}api/v1/interaction/${interaction.id}/respond`;

		const res = await fetch(url, {
			method: "POST",
			headers: {
				"Content-Type": "application/json",
				Authorization: `Bearer ${interaction.responder_jwt}`,
			},
			body: JSON.stringify({ value }),
		});
		if (!res.ok) {
			const errorText = await res.text();
			throw new Error(`API responded ${res.status}: ${errorText}`);
		}
		return;
	}

	const { invoke } = await import("@tauri-apps/api/core");
	await invoke("respond_to_interaction", {
		interactionId: interaction.id,
		value,
	});
}
