import { createId } from "@paralleldrive/cuid2";

/**
 * Session-scoped draft/acceptance identity for the board-panel FlowPilot. Prompt text alone is not
 * a safe identity — identical short prompts ("yes, build it") from another surface could hijack or
 * resume this panel's retained drafts — so the panel folds a stable per-board conversation scope id
 * into the request, mirroring how the global chat scopes identity by its conversation id. Persisted
 * in sessionStorage so a reload keeps draft continuity for the panel.
 */
const STORAGE_KEY_PREFIX = "flow-like:flowpilot:panel-conversation:";

const memoryFallback = new Map<string, string>();

function defaultStorage(): Pick<Storage, "getItem" | "setItem"> | undefined {
	try {
		return typeof sessionStorage === "undefined" ? undefined : sessionStorage;
	} catch {
		return undefined;
	}
}

export function flowPilotPanelConversationId(
	scopeKey: string,
	storage: Pick<Storage, "getItem" | "setItem"> | undefined = defaultStorage(),
): string {
	const key = `${STORAGE_KEY_PREFIX}${scopeKey}`;

	try {
		const existing = storage?.getItem(key);
		if (existing?.trim()) {
			memoryFallback.set(key, existing);
			return existing;
		}
	} catch {
		// fall through to the in-memory scope id
	}

	const generated =
		memoryFallback.get(key) ?? `flowpilot-panel:${scopeKey}:${createId()}`;
	memoryFallback.set(key, generated);

	try {
		storage?.setItem(key, generated);
	} catch {
		// reload continuity is best-effort; identity stays stable within this session
	}

	return generated;
}
