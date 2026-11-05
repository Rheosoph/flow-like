"use client";

import { useTranslation } from "@flow-like/locales";
import { useMemo } from "react";
import { useInvoke } from "../../../../hooks/use-invoke";
import {
	type LocalModelHostCapabilities,
	isDeviceModelBit,
	isFreeLlmModel,
	isLocalLlmModel,
	selectProfileLlmModels,
} from "../../../../lib/bit/local-model-filter";
import { asArray } from "../../../../lib/response-shape";
import { type IBit, IBitTypes } from "../../../../lib/schema/bit/bit";
import { useBackend } from "../../../../state/backend-state";

export interface BotModelOption {
	id: string;
	label: string;
	/** Runs on a computer or device of the user's, so a cloud bot can't use it. */
	local: boolean;
	free: boolean;
}

export interface BotEmbeddingModelOption {
	id: string;
	label: string;
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
		typeof provider?.model_id === "string" ? provider.model_id.toLowerCase() : "";
	return IMAGE_MODEL_NEEDLES.some((needle) => modelId.includes(needle));
}

/** The profile's models of these Bit types, from the same queries as the chat's memory picker. */
export function useBotBitOptions(
	types: readonly IBitTypes[],
	accepts?: (bit: IBit) => boolean,
): {
	options: BotEmbeddingModelOption[];
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
				.map((bit) => ({ id: bit.id, label: bitName(bit, language) })),
		[profileBits.data, language, types, accepts],
	);
	return {
		options,
		isLoading: settingsProfile.isLoading || profileBits.isLoading,
	};
}

const EMBEDDING_TYPES: readonly IBitTypes[] = [IBitTypes.Embedding];

/** The profile's embedding models, from the same queries as the chat's memory picker. */
export function useBotEmbeddingModelOptions() {
	return useBotBitOptions(EMBEDDING_TYPES);
}

/** The profile's language models, from the same queries as the chat's model picker. */
export function useBotModelOptions(): {
	options: BotModelOption[];
	isLoading: boolean;
} {
	const { i18n } = useTranslation("bots");
	const backend = useBackend();
	const settingsProfile = useSettingsProfile();
	const profileId = settingsProfile.data?.hub_profile.id;
	const llmBits = useInvoke(
		backend.bitState.searchBits,
		backend.bitState,
		[{ bit_types: [IBitTypes.Llm, IBitTypes.Vlm] }],
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
			).map((bit) => ({
				id: bit.id,
				label: bitName(bit, language),
				local: isLocalLlmModel(bit) || isDeviceModelBit(bit),
				free: isFreeLlmModel(bit),
			})),
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
