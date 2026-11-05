"use client";

import { useEffect } from "react";
import {
	AGENT_EMBEDDING_MODEL_KEY,
	AGENT_MODEL_KEY,
	AGENT_PROVIDER_KEY,
	AGENT_REASONING_KEY,
	useGlobalChatStore,
} from "../../state/global-chat/global-chat-store";
import type { AIProvider } from "../flowpilot/types";

// Restored once per session rather than per surface: the hero, the Bots tab and the
// docked overlay all share one store, so whichever mounts first hydrates it.
let hydrated = false;

/**
 * Restore the last explicitly chosen provider, models and reasoning effort into the
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
	const setEmbeddingModelId = useGlobalChatStore((s) => s.setEmbeddingModelId);

	useEffect(() => {
		if (hydrated) return;
		hydrated = true;
		try {
			const provider = localStorage.getItem(AGENT_PROVIDER_KEY);
			const model = localStorage.getItem(AGENT_MODEL_KEY);
			const effort = localStorage.getItem(AGENT_REASONING_KEY);
			const embedding = localStorage.getItem(AGENT_EMBEDDING_MODEL_KEY);
			if (provider) setProvider(provider as AIProvider);
			if (model) setSelectedModelId(model);
			if (effort) setReasoningEffort(effort);
			if (embedding !== null) setEmbeddingModelId(embedding);
		} catch {
			// Storage is unavailable; remembering is best-effort.
		}
	}, [
		setProvider,
		setSelectedModelId,
		setReasoningEffort,
		setEmbeddingModelId,
	]);
}
