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
const REASONING_EFFORT_STORAGE_KEY = "flowpilot.hero.reasoning-effort";
// Persisted provider/model/effort live in an in-memory store shared with /chat;
// hydrate them from localStorage once per session so the user's picks stick.
let hydrated = false;

export interface AgentReasoningEffortOption {
	id: string;
	name: string;
	description?: string;
}

export interface AgentModelOption {
	id: string;
	name: string;
	supportedReasoningEfforts?: AgentReasoningEffortOption[];
	defaultReasoningEffort?: string;
}

export interface AgentProviderOption {
	id: NormalizedAIProvider;
	label: string;
}

const PROFILE_PROVIDER: AgentProviderOption = { id: "bits", label: "Profile" };
const AGENT_PROVIDERS: AgentProviderOption[] = [
	{ id: "github-copilot", label: "Copilot" },
	{ id: "codex", label: "Codex" },
	{ id: "claude-code", label: "Claude Code" },
];

export function useAgentSelection() {
	const provider = useGlobalChatStore((s) => s.provider);
	const selectedModelId = useGlobalChatStore((s) => s.selectedModelId);
	const reasoningEffort = useGlobalChatStore((s) => s.reasoningEffort);
	const setProvider = useGlobalChatStore((s) => s.setProvider);
	const setSelectedModelId = useGlobalChatStore((s) => s.setSelectedModelId);
	const setReasoningEffort = useGlobalChatStore((s) => s.setReasoningEffort);

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
			const savedReasoningEffort = localStorage.getItem(
				REASONING_EFFORT_STORAGE_KEY,
			);
			if (savedProvider) setProvider(savedProvider as AIProvider);
			if (savedModel) setSelectedModelId(savedModel);
			if (savedReasoningEffort) setReasoningEffort(savedReasoningEffort);
		} catch {
			// storage unavailable — remembering is best-effort
		}
	}, [setProvider, setReasoningEffort, setSelectedModelId]);

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
	useEffect(() => {
		try {
			localStorage.setItem(REASONING_EFFORT_STORAGE_KEY, reasoningEffort);
		} catch {}
	}, [reasoningEffort]);

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
				supportedReasoningEfforts: model.supportedReasoningEfforts,
				defaultReasoningEffort: model.defaultReasoningEffort,
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

	const selectedModel = models.find((model) => model.id === selectedModelId);
	const reasoningEffortOptions = selectedModel?.supportedReasoningEfforts ?? [];

	// Model catalogs are loaded asynchronously. `undefined` means the temporary
	// static fallback is still visible; an actual empty array means the backend
	// advertised that this model has no configurable effort levels.
	useEffect(() => {
		if (!reasoningEffort) return;
		if (!isAgent) {
			setReasoningEffort("");
			return;
		}
		if (
			!selectedModel ||
			selectedModel.supportedReasoningEfforts === undefined
		) {
			return;
		}
		if (
			!selectedModel.supportedReasoningEfforts.some(
				(option) => option.id === reasoningEffort,
			)
		) {
			setReasoningEffort("");
		}
	}, [isAgent, reasoningEffort, selectedModel, setReasoningEffort]);

	const providerLabel =
		availableProviders.find((option) => option.id === normalizedProvider)
			?.label ?? PROFILE_PROVIDER.label;
	const selectedModelName =
		selectedModel?.name ??
		(isAgent && copilotSDK.isConnecting ? "Connecting…" : "Auto");
	const defaultReasoningEffortName = reasoningEffortOptions.find(
		(option) => option.id === selectedModel?.defaultReasoningEffort,
	)?.name;
	const autoReasoningEffortName = defaultReasoningEffortName
		? `Auto (${defaultReasoningEffortName} default)`
		: "Auto (provider default)";
	const reasoningEffortName = reasoningEffort
		? (reasoningEffortOptions.find((option) => option.id === reasoningEffort)
				?.name ?? reasoningEffort)
		: autoReasoningEffortName;

	return {
		provider: normalizedProvider,
		setProvider,
		selectedModelId,
		setSelectedModelId,
		availableProviders,
		providerLabel,
		models,
		selectedModelName,
		reasoningEffort,
		setReasoningEffort,
		reasoningEffortOptions,
		reasoningEffortName,
		autoReasoningEffortName,
		isAgent,
		connecting: copilotSDK.isConnecting,
		connected: copilotSDK.isRunning && isAgent,
	};
}
