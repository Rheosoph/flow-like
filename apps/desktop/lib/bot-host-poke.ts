import { invoke } from "@tauri-apps/api/core";

const POKE_DEBOUNCE_MS = 100;

let timer: ReturnType<typeof setTimeout> | undefined;

/** Brings the account bot host's next poll forward; failures only mean the host polls on schedule. */
export function pokeBotHost(): void {
	if (timer !== undefined) return;
	timer = setTimeout(() => {
		timer = undefined;
		void invoke("bot_host_poke").catch(() => undefined);
	}, POKE_DEBOUNCE_MS);
}
