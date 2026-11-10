"use client";

import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import { speechBitReference } from "../lib/realtime/bot-call-speech";
import {
	type SttChoice,
	isLocalSttBit,
	resolveSttChoice,
} from "../lib/realtime/call-listening";
import { asArray } from "../lib/response-shape";
import { IBitTypes } from "../lib/schema/bit/bit";
import { useBackend } from "../state/backend-state";

const NO_IDS: ReadonlySet<string> = new Set();

export interface SpeechModelState {
	choice: SttChoice | null;
	/** Everything was read, so a null `choice` means there is nothing to transcribe with. */
	settled: boolean;
	/** The host has no audio state, so no model would help. */
	unsupported: boolean;
	/** `pinned` names a model this client can't transcribe with, or one that no longer exists. */
	pinnedUnusable: boolean;
}

/**
 * The speech-to-text bit that transcribes this person: `pinned` (a bot's own model) when this
 * client can run it, else an installed local model on desktop, else a hosted one from the profile.
 */
export function useSpeechModelState(
	enabled = true,
	pinned?: string | null,
): SpeechModelState {
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
	const pin = useQuery({
		queryKey: ["speech-model-pin", pinned, desktop],
		enabled: active && Boolean(pinned),
		queryFn: async (): Promise<SttChoice | null> => {
			const reference = speechBitReference(pinned as string);
			const bit = await backend.bitState.getBit(reference.id, reference.hub);
			if (bit.type !== IBitTypes.Stt) return null;
			const local =
				desktop &&
				isLocalSttBit(bit) &&
				(await backend.bitState.isBitInstalled(bit));
			return resolveSttChoice([bit], {
				desktop,
				installed: local ? new Set([bit.id]) : NO_IDS,
			});
		},
		retry: false,
	});
	const choice = useMemo(
		() =>
			backend.audioState
				? (pin.data ??
					resolveSttChoice(sttBits, {
						desktop,
						installed: installed.data ?? NO_IDS,
					}))
				: null,
		[backend.audioState, pin.data, sttBits, desktop, installed.data],
	);
	const settled =
		!backend.audioState ||
		Boolean(choice) ||
		(profileBits.isSuccess &&
			(localBits.length === 0 || installed.isSuccess) &&
			(!pinned || pin.isSuccess || pin.isError));
	return {
		choice,
		settled: enabled && settled,
		unsupported: !backend.audioState,
		pinnedUnusable:
			Boolean(pinned) && (pin.isError || (pin.isSuccess && pin.data === null)),
	};
}

/** The speech-to-text choice alone; null while loading or when there is none. */
export function useSpeechModel(enabled = true): SttChoice | null {
	return useSpeechModelState(enabled).choice;
}
