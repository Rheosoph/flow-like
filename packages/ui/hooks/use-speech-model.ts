"use client";

import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import {
	type SttChoice,
	isLocalSttBit,
	resolveSttChoice,
} from "../lib/realtime/call-listening";
import { asArray } from "../lib/response-shape";
import { IBitTypes } from "../lib/schema/bit/bit";
import { useBackend } from "../state/backend-state";

const NO_IDS: ReadonlySet<string> = new Set();

/**
 * The speech-to-text bit that transcribes this person: an installed local model on desktop,
 * else a hosted one from the profile. null while loading, or when the profile has neither or
 * the host has no audio state to transcribe with.
 */
export function useSpeechModel(enabled = true): SttChoice | null {
	const backend = useBackend();
	const active = enabled && backend.audioState !== undefined;
	const desktop = backend.capabilities?.().canExecuteLocally ?? false;
	const profile = useQuery({
		queryKey: ["speech-model-profile"],
		enabled: active,
		queryFn: () => backend.userState.getSettingsProfile(),
	});
	const profileId = profile.data?.hub_profile.id;
	const profileBits = useQuery({
		queryKey: ["speech-model-bits", profileId],
		enabled: active && !!profile.data,
		queryFn: () => backend.bitState.getProfileBits(),
	});
	const sttBits = useMemo(
		() => asArray(profileBits.data).filter((bit) => bit.type === IBitTypes.Stt),
		[profileBits.data],
	);
	const localBits = useMemo(
		() => (desktop ? sttBits.filter(isLocalSttBit) : []),
		[desktop, sttBits],
	);
	const installed = useQuery({
		queryKey: ["speech-model-installed", localBits.map((bit) => bit.id)],
		enabled: active && localBits.length > 0,
		queryFn: async () => {
			const ids = new Set<string>();
			for (const bit of localBits) {
				if (await backend.bitState.isBitInstalled(bit)) ids.add(bit.id);
			}
			return ids as ReadonlySet<string>;
		},
	});
	return useMemo(
		() =>
			backend.audioState
				? resolveSttChoice(sttBits, {
						desktop,
						installed: installed.data ?? NO_IDS,
					})
				: null,
		[backend.audioState, sttBits, desktop, installed.data],
	);
}
