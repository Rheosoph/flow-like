"use client";

import { useTranslation } from "@flow-like/locales";
import { useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { toast } from "sonner";
import type { IBit } from "../lib/schema/bit/bit";
import type { IHub } from "../lib/schema/hub/hub";
import { useBackend } from "../state/backend-state";
import { useDownloadManager } from "../state/download-manager";
import {
	handleUpgradeRequiredError,
	openUpgradeDialogIfEnabled,
} from "../state/upgrade-dialog-state";
import type { ISettingsProfile } from "../types";
import { useHub } from "./use-hub";
import { useInvalidateInvoke, useInvoke } from "./use-invoke";

export interface ModelTierGate {
	isRestricted: boolean;
	requiredTier: string | null;
}

/** Whether the person's plan covers the model; hosted models name the plan tier they need. */
export function modelTierGate(
	bit: IBit,
	tiers: IHub["tiers"] | undefined,
	userTier: string | undefined,
): ModelTierGate {
	const modelTier = (
		bit.parameters as { provider?: { params?: { tier?: string } } } | undefined
	)?.provider?.params?.tier;
	if (!modelTier || !tiers) return { isRestricted: false, requiredTier: null };
	const config = tiers[(userTier ?? "FREE").toUpperCase()];
	if (!config) return { isRestricted: true, requiredTier: modelTier };
	const isRestricted = !(config.llm_tiers ?? []).includes(modelTier);
	return { isRestricted, requiredTier: isRestricted ? modelTier : null };
}

/** `useSpeechModelState` reads the profile through its own keys. */
function isSpeechModelQuery(query: { queryKey: readonly unknown[] }): boolean {
	const [head] = query.queryKey;
	return typeof head === "string" && head.startsWith("speech-model-");
}

export function isBitInProfile(
	profile: ISettingsProfile | undefined,
	bitId: string,
): boolean {
	return (profile?.hub_profile.bits ?? []).some(
		(id) => id.split(":").pop() === bitId,
	);
}

/**
 * Adds models to the active profile the way the model catalog does: a model above the person's
 * plan opens the upgrade dialog, a model with files downloads first. `add` resolves false when the
 * model was not added; it reports why itself. `allows` checks the plan alone, so a picker can use
 * the model at once and leave `add` to finish in the background.
 */
export function useAddModelToProfile(queryScope: string[] = []) {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const queryClient = useQueryClient();
	const invalidate = useInvalidateInvoke();
	const download = useDownloadManager((s) => s.download);
	const { hub } = useHub(queryScope);
	const profile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
		true,
		queryScope,
	);
	const info = useInvoke(
		backend.userState.getInfo,
		backend.userState,
		[],
		true,
		queryScope,
	);
	const tiers = hub?.tiers;
	const userTier = info.data?.tier;
	const gate = useCallback(
		(bit: IBit) => modelTierGate(bit, tiers, userTier),
		[tiers, userTier],
	);
	/** False, after offering the upgrade, when the person's plan doesn't cover the model. */
	const allows = useCallback(
		(bit: IBit): boolean => {
			const tier = gate(bit);
			if (!tier.isRestricted) return true;
			if (
				!openUpgradeDialogIfEnabled({
					reason: "model-tier",
					requiredTier: tier.requiredTier ?? undefined,
				})
			) {
				toast.error(
					t(
						"thisModelRequiresTheRequiredtierPlan",
						"This model requires the {{requiredTier}} plan.",
						{ requiredTier: tier.requiredTier },
					),
				);
			}
			return false;
		},
		[gate, t],
	);
	const fetchFiles = useCallback(
		async (bit: IBit) => {
			if ((await backend.bitState.getBitSize(bit)) > 0) await download(bit);
		},
		[backend.bitState, download],
	);
	const add = useCallback(
		async (
			bit: IBit,
			fetch: (bit: IBit) => Promise<unknown> = fetchFiles,
		): Promise<boolean> => {
			const current = profile.data;
			if (!current) return false;
			if (isBitInProfile(current, bit.id)) return true;
			if (!allows(bit)) return false;
			try {
				await fetch(bit);
				await backend.bitState.addBit(bit, current);
			} catch (error) {
				console.error(`Adding model ${bit.id} to the profile failed:`, error);
				if (!handleUpgradeRequiredError(error, "model-tier")) {
					toast.error(
						error instanceof Error
							? error.message
							: t("failedToUpdateProfile", "Failed to update profile"),
					);
				}
				return false;
			}
			await Promise.all([
				invalidate(backend.userState.getSettingsProfile, []),
				invalidate(backend.userState.getProfile, []),
				invalidate(backend.bitState.getProfileBits, []),
				queryClient.invalidateQueries({ predicate: isSpeechModelQuery }),
			]);
			return true;
		},
		[
			profile.data,
			allows,
			fetchFiles,
			backend.bitState,
			backend.userState,
			invalidate,
			queryClient,
			t,
		],
	);
	return { profile: profile.data, gate, allows, add };
}
