"use client";

import { useEffect, useMemo } from "react";
import {
	IBitTypes,
	filterHostableLlmModels,
	isFreeLlmModel,
	useBackend,
	useCopilotSDK,
	useInvoke,
} from "../../../index";
import { isTauri } from "../../../lib/platform";
import {
	AGENT_MODEL_KEY,
	useGlobalChatStore,
} from "../../../state/global-chat/global-chat-store";
import type {
	AgentBackendProvider,
	NormalizedAIProvider,
} from "../../flowpilot/types";
import {
	isAgentBackendProvider,
	normalizeAIProvider,
} from "../../flowpilot/types";
import { useHydrateAgentSelection } from "../use-agent-persistence";

export interface AgentReasoningEffortOption {
	id: string;
	name: string;
	description?: string;
}

export interface AgentModelOption {
	id: string;
	name: string;
	isFree?: boolean;
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
	const setSelectedModelId = useGlobalChatStore((s) => s.setSelectedModelId);
	const setReasoningEffort = useGlobalChatStore((s) => s.setReasoningEffort);
	// Explicit picks persist; the "keep a valid model" fallback below deliberately
	// uses the plain setters so a still-loading catalog can never clobber them.
	const selectProvider = useGlobalChatStore((s) => s.selectProvider);
	const selectModel = useGlobalChatStore((s) => s.selectModel);
	const selectReasoningEffort = useGlobalChatStore(
		(s) => s.selectReasoningEffort,
	);

	useHydrateAgentSelection();

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
	const customBits = useInvoke(
		backend.bitState.listCustomBits,
		backend.bitState,
		[],
		!!settingsProfile.data,
		[settingsProfile.data?.hub_profile.id],
	);
	const canHostLlamaCPP = backend.capabilities().canHostLlamaCPP;
	const bitsModels = useMemo(() => {
		const profileBits = settingsProfile.data?.hub_profile.bits;
		if (!llmBits.data || !profileBits) return [];
		const ids = new Set(profileBits);
		const profileModels = llmBits.data.filter((bit) =>
			ids.has(`${bit.hub}:${bit.id}`),
		);
		const seen = new Set(profileModels.map((bit) => bit.id));
		const ownModels = (customBits.data ?? []).filter(
			(bit) =>
				!seen.has(bit.id) &&
				(bit.type === IBitTypes.Llm || bit.type === IBitTypes.Vlm),
		);
		return filterHostableLlmModels(
			[...ownModels, ...profileModels],
			canHostLlamaCPP,
		);
	}, [
		llmBits.data,
		customBits.data,
		settingsProfile.data?.hub_profile.bits,
		canHostLlamaCPP,
	]);

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
			isFree: isFreeLlmModel(bit),
		}));
	}, [isAgent, copilotSDK.models, bitsModels]);

	// Keep a usable model selected as the catalog loads/changes — but prefer the
	// user's remembered pick the moment the catalog that offers it appears, so a
	// transient/fallback list can't strand them on the wrong model (and, because
	// this uses the raw setter, it never rewrites the remembered value).
	useEffect(() => {
		if (models.length === 0) return;
		let remembered: string | null = null;
		try {
			remembered = localStorage.getItem(AGENT_MODEL_KEY);
		} catch {}
		if (
			remembered &&
			remembered !== selectedModelId &&
			models.some((model) => model.id === remembered)
		) {
			setSelectedModelId(remembered);
			return;
		}
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
		setProvider: selectProvider,
		selectedModelId,
		setSelectedModelId: selectModel,
		availableProviders,
		providerLabel,
		models,
		selectedModelName,
		reasoningEffort,
		setReasoningEffort: selectReasoningEffort,
		reasoningEffortOptions,
		reasoningEffortName,
		autoReasoningEffortName,
		isAgent,
		connecting: copilotSDK.isConnecting,
		connected: copilotSDK.isRunning && isAgent,
	};
}
