"use client";

import { useEffect, useMemo } from "react";
import {
	IBitTypes,
	useBackend,
	useCopilotSDK,
	useInvoke,
} from "../../../index";
import { isTauri } from "../../../lib/platform";
import { useGlobalChatStore } from "../../../state/global-chat/global-chat-store";
import type {
	AIProvider,
	AgentBackendProvider,
	NormalizedAIProvider,
} from "../../flowpilot/types";
import {
	isAgentBackendProvider,
	normalizeAIProvider,
} from "../../flowpilot/types";

const PROVIDER_STORAGE_KEY = "flowpilot.hero.provider";
const MODEL_STORAGE_KEY = "flowpilot.hero.model";
// Persisted provider/model live in an in-memory store shared with /chat; hydrate
// them from localStorage a single time per session so the user's pick sticks.
let hydrated = false;

export interface AgentModelOption {
	id: string;
	name: string;
}

export interface AgentProviderOption {
	id: NormalizedAIProvider;
	label: string;
}

// Claude Code is defined in the type union but not enabled (normalizeEnabledAIProvider
// folds it into codex), so it is intentionally omitted from the available list.
const PROFILE_PROVIDER: AgentProviderOption = { id: "bits", label: "Profile" };
const AGENT_PROVIDERS: AgentProviderOption[] = [
	{ id: "github-copilot", label: "Copilot" },
	{ id: "codex", label: "Codex" },
];

export function useAgentSelection() {
	const provider = useGlobalChatStore((s) => s.provider);
	const selectedModelId = useGlobalChatStore((s) => s.selectedModelId);
	const setProvider = useGlobalChatStore((s) => s.setProvider);
	const setSelectedModelId = useGlobalChatStore((s) => s.setSelectedModelId);

	const backend = useBackend();
	const settingsProfile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
		true,
	);
	const llmBits = useInvoke(
		backend.bitState.searchBits,
		backend.bitState,
		[{ bit_types: [IBitTypes.Llm, IBitTypes.Vlm] }],
		!!settingsProfile.data,
		[settingsProfile.data?.hub_profile.id],
	);
	const bitsModels = useMemo(() => {
		const profileBits = settingsProfile.data?.hub_profile.bits;
		if (!llmBits.data || !profileBits) return [];
		const ids = new Set(profileBits);
		return llmBits.data.filter((bit) => ids.has(`${bit.hub}:${bit.id}`));
	}, [llmBits.data, settingsProfile.data?.hub_profile.bits]);

	const normalizedProvider = normalizeAIProvider(provider);
	const isAgent = isAgentBackendProvider(normalizedProvider);
	const activeAgentBackend: AgentBackendProvider = isAgent
		? normalizedProvider
		: "github-copilot";
	const copilotSDK = useCopilotSDK(activeAgentBackend);

	const isDesktop = isTauri();
	const availableProviders = useMemo<AgentProviderOption[]>(
		() =>
			isDesktop ? [PROFILE_PROVIDER, ...AGENT_PROVIDERS] : [PROFILE_PROVIDER],
		[isDesktop],
	);

	// Hydrate the shared store from the last remembered pick, once.
	useEffect(() => {
		if (hydrated) return;
		hydrated = true;
		try {
			const savedProvider = localStorage.getItem(PROVIDER_STORAGE_KEY);
			const savedModel = localStorage.getItem(MODEL_STORAGE_KEY);
			if (savedProvider) setProvider(savedProvider as AIProvider);
			if (savedModel) setSelectedModelId(savedModel);
		} catch {
			// storage unavailable — remembering is best-effort
		}
	}, [setProvider, setSelectedModelId]);

	useEffect(() => {
		try {
			localStorage.setItem(PROVIDER_STORAGE_KEY, provider);
		} catch {}
	}, [provider]);
	useEffect(() => {
		if (!selectedModelId) return;
		try {
			localStorage.setItem(MODEL_STORAGE_KEY, selectedModelId);
		} catch {}
	}, [selectedModelId]);

	// Start the chosen agent backend so its models/auth load (mirrors the chat).
	useEffect(() => {
		if (
			isAgent &&
			isDesktop &&
			!copilotSDK.isRunning &&
			!copilotSDK.isConnecting
		) {
			void copilotSDK.start().catch(() => undefined);
		}
	}, [
		isAgent,
		isDesktop,
		copilotSDK.isRunning,
		copilotSDK.isConnecting,
		copilotSDK.start,
	]);

	const models = useMemo<AgentModelOption[]>(() => {
		if (isAgent) {
			return copilotSDK.models.map((model) => ({
				id: model.id,
				name: model.name || model.id,
			}));
		}
		return bitsModels.map((bit) => ({
			id: bit.id,
			name: bit.meta?.en?.name ?? bit.id,
		}));
	}, [isAgent, copilotSDK.models, bitsModels]);

	// Keep a valid model selected whenever the active model list changes.
	useEffect(() => {
		if (models.length === 0) return;
		if (!models.some((model) => model.id === selectedModelId)) {
			setSelectedModelId(models[0].id);
		}
	}, [models, selectedModelId, setSelectedModelId]);

	const providerLabel =
		availableProviders.find((option) => option.id === normalizedProvider)
			?.label ?? PROFILE_PROVIDER.label;
	const selectedModelName =
		models.find((model) => model.id === selectedModelId)?.name ??
		(isAgent && copilotSDK.isConnecting ? "Connecting…" : "Auto");

	return {
		provider: normalizedProvider,
		setProvider,
		selectedModelId,
		setSelectedModelId,
		availableProviders,
		providerLabel,
		models,
		selectedModelName,
		isAgent,
		connecting: copilotSDK.isConnecting,
	};
}
