"use client";

import { useTranslation } from "@flow-like/locales";
import { useMemo } from "react";
import { useInvoke } from "../../../../hooks/use-invoke";
import {
	type LocalModelHostCapabilities,
	filterHostableLlmModels,
	isDeviceModelBit,
	isFreeLlmModel,
	isLocalLlmModel,
	selectProfileLlmModels,
} from "../../../../lib/bit/local-model-filter";
import {
	listableModels,
	searchAllBitsOfType,
} from "../../../../lib/bit/model-listing";
import { asArray } from "../../../../lib/response-shape";
import { type IBit, IBitTypes } from "../../../../lib/schema/bit/bit";
import { useBackend } from "../../../../state/backend-state";

export interface BotModelOption {
	id: string;
	label: string;
	bit: IBit;
	/** Runs on a computer or device of the user's, so a cloud bot can't use it. */
	local: boolean;
	free: boolean;
}

/** Which catalog models a picker may offer beside the profile's own. */
export interface ModelCatalogSpec {
	types: readonly IBitTypes[];
	accepts?: (bit: IBit) => boolean;
}

/** The bot's host is not this client, so no model is hidden for what this client can run. */
const EVERY_HOST: LocalModelHostCapabilities = {
	canHostLlamaCPP: true,
	canHostMLX: true,
	canUseNativeAgentProviders: true,
	deviceModels: true,
};

function bitName(bit: IBit, language: string): string {
	return bit.meta?.[language]?.name ?? bit.meta?.en?.name ?? bit.id;
}

/** Served by the user's own computer or device rather than a hosted provider. */
export function runsOnDevice(bit: IBit): boolean {
	if (isLocalLlmModel(bit) || isDeviceModelBit(bit)) return true;
	const provider = bit.parameters?.provider?.provider_name;
	return (
		typeof provider === "string" &&
		provider.trim().toLowerCase().startsWith("local")
	);
}

function toOption(bit: IBit, language: string): BotModelOption {
	return {
		id: bit.id,
		label: bitName(bit, language),
		bit,
		local: runsOnDevice(bit),
		free: isFreeLlmModel(bit),
	};
}

function useSettingsProfile() {
	const backend = useBackend();
	return useInvoke(backend.userState.getSettingsProfile, backend.userState, []);
}

const IMAGE_MODEL_NEEDLES = [
	"image",
	"imagen",
	"flux",
	"kontext",
	"stable-diffusion",
	"diffusion",
	"sdxl",
	"dall-e",
	"sourceful",
	"riverflow",
];

/** An image generation Bit, or a Vlm whose upstream model id names an image model (as the image tool accepts). */
export function isImageCapableBit(bit: IBit): boolean {
	if (bit.type === IBitTypes.ImageGeneration) return true;
	if (bit.type !== IBitTypes.Vlm) return false;
	const provider = (bit.parameters as { provider?: { model_id?: unknown } })
		?.provider;
	const modelId =
		typeof provider?.model_id === "string"
			? provider.model_id.toLowerCase()
			: "";
	return IMAGE_MODEL_NEEDLES.some((needle) => modelId.includes(needle));
}

/** The profile's models of these Bit types, from the same queries as the chat's memory picker. */
export function useBotBitOptions(
	types: readonly IBitTypes[],
	accepts?: (bit: IBit) => boolean,
): {
	options: BotModelOption[];
	isLoading: boolean;
} {
	const { i18n } = useTranslation("bots");
	const backend = useBackend();
	const settingsProfile = useSettingsProfile();
	const profileBits = useInvoke(
		backend.bitState.getProfileBits,
		backend.bitState,
		[],
		!!settingsProfile.data,
		[settingsProfile.data?.hub_profile.id],
	);
	const language = i18n.language;
	const options = useMemo(
		() =>
			asArray(profileBits.data)
				.filter((bit) => types.includes(bit.type) && (accepts?.(bit) ?? true))
				.map((bit) => toOption(bit, language)),
		[profileBits.data, language, types, accepts],
	);
	return {
		options,
		isLoading: settingsProfile.isLoading || profileBits.isLoading,
	};
}

/**
 * Every model of the spec in the hub's catalog and the user's own library that this client can
 * list, the way the model catalog lists them. Loads only while `enabled`.
 */
export function useCatalogModelOptions(
	spec: ModelCatalogSpec,
	enabled: boolean,
): {
	options: BotModelOption[];
	isLoading: boolean;
	isError: boolean;
} {
	const { i18n } = useTranslation("bots");
	const backend = useBackend();
	const settingsProfile = useSettingsProfile();
	const profileId = settingsProfile.data?.hub_profile.id;
	const active = enabled && !!settingsProfile.data;
	const types = useMemo(() => [...spec.types], [spec.types]);
	const catalog = useInvoke(
		searchAllBitsOfType,
		backend.bitState,
		[types],
		active,
		[profileId],
	);
	const customBits = useInvoke(
		backend.bitState.listCustomBits,
		backend.bitState,
		[],
		active,
		[profileId],
	);
	const capabilities = backend.capabilities?.();
	const canHostLlamaCPP = capabilities?.canHostLlamaCPP ?? false;
	const canHostMLX = capabilities?.canHostMLX ?? false;
	const canUseNativeAgentProviders =
		capabilities?.canUseNativeAgentProviders ?? false;
	const deviceModels = capabilities?.deviceModels ?? false;
	const language = i18n.language;
	const { accepts } = spec;
	const options = useMemo(() => {
		const merged = new Map<string, IBit>();
		for (const bit of listableModels(asArray(catalog.data)))
			merged.set(bit.id, bit);
		for (const bit of asArray(customBits.data)) merged.set(bit.id, bit);
		return filterHostableLlmModels([...merged.values()], {
			canHostLlamaCPP,
			canHostMLX,
			canUseNativeAgentProviders,
			deviceModels,
		})
			.filter((bit) => types.includes(bit.type) && (accepts?.(bit) ?? true))
			.map((bit) => toOption(bit, language));
	}, [
		catalog.data,
		customBits.data,
		canHostLlamaCPP,
		canHostMLX,
		canUseNativeAgentProviders,
		deviceModels,
		types,
		accepts,
		language,
	]);
	return {
		options,
		isLoading: active && catalog.isLoading,
		isError: catalog.isError,
	};
}

const EMBEDDING_TYPES: readonly IBitTypes[] = [IBitTypes.Embedding];

export const EMBEDDING_CATALOG: ModelCatalogSpec = { types: EMBEDDING_TYPES };

/** The profile's embedding models, from the same queries as the chat's memory picker. */
export function useBotEmbeddingModelOptions() {
	return useBotBitOptions(EMBEDDING_TYPES);
}

const LLM_TYPES: IBitTypes[] = [IBitTypes.Llm, IBitTypes.Vlm];

export const LLM_CATALOG: ModelCatalogSpec = { types: LLM_TYPES };

/**
 * The profile's language models. Every catalog page is read, so a model added from deep in the
 * catalog still has its name; the picker's catalog shares the query.
 */
export function useBotModelOptions(): {
	options: BotModelOption[];
	isLoading: boolean;
} {
	const { i18n } = useTranslation("bots");
	const backend = useBackend();
	const settingsProfile = useSettingsProfile();
	const profileId = settingsProfile.data?.hub_profile.id;
	const llmBits = useInvoke(
		searchAllBitsOfType,
		backend.bitState,
		[LLM_TYPES],
		!!settingsProfile.data,
		[profileId],
	);
	const customBits = useInvoke(
		backend.bitState.listCustomBits,
		backend.bitState,
		[],
		!!settingsProfile.data,
		[profileId],
	);
	const language = i18n.language;
	const options = useMemo(
		() =>
			selectProfileLlmModels(
				llmBits.data,
				customBits.data,
				settingsProfile.data?.hub_profile.bits,
				EVERY_HOST,
			).map((bit) => toOption(bit, language)),
		[
			llmBits.data,
			customBits.data,
			settingsProfile.data?.hub_profile.bits,
			language,
		],
	);
	return {
		options,
		isLoading: settingsProfile.isLoading || llmBits.isLoading,
	};
}
