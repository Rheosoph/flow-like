"use client";

import { useEffect } from "react";
import {
	AGENT_MODEL_KEY,
	AGENT_PROVIDER_KEY,
	AGENT_REASONING_KEY,
	useGlobalChatStore,
} from "../../state/global-chat/global-chat-store";
import type { AIProvider } from "../flowpilot/types";

// Restored once per session rather than per surface: the hero, /chat and the
// docked overlay all share one store, so whichever mounts first hydrates it.
let hydrated = false;

/**
 * Restore the last explicitly chosen provider / model / reasoning effort into the
 * shared global-chat store. Deliberately runs post-mount (not at store creation)
 * so the statically exported HTML never disagrees with the first client render.
 *
 * Uses the plain store setters on purpose: hydration is not a new user choice, so
 * it must not rewrite the remembered values.
 */
export function useHydrateAgentSelection() {
	const setProvider = useGlobalChatStore((s) => s.setProvider);
	const setSelectedModelId = useGlobalChatStore((s) => s.setSelectedModelId);
	const setReasoningEffort = useGlobalChatStore((s) => s.setReasoningEffort);

	useEffect(() => {
		if (hydrated) return;
		hydrated = true;
		try {
			const provider = localStorage.getItem(AGENT_PROVIDER_KEY);
			const model = localStorage.getItem(AGENT_MODEL_KEY);
			const effort = localStorage.getItem(AGENT_REASONING_KEY);
			if (provider) setProvider(provider as AIProvider);
			if (model) setSelectedModelId(model);
			if (effort) setReasoningEffort(effort);
		} catch {
			// storage unavailable — remembering is best-effort
		}
	}, [setProvider, setSelectedModelId, setReasoningEffort]);
}
