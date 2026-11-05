"use client";

import { useTranslation } from "@flow-like/locales";
import { useQueries } from "@tanstack/react-query";
import { useEffect, useMemo, useRef } from "react";
import { toast } from "sonner";
import type { ConversationItem } from "../components/bots-tab/fold-entries";
import type { CrewDirectory } from "../components/bots-tab/use-crew-directory";
import { isHostedAudioProvider } from "../lib/bit/hosted-audio";
import {
	type BotSpeechVoice,
	createBotCallSpeech,
	speechBitReference,
	speechCharacterLimit,
} from "../lib/realtime/bot-call-speech";
import { IBitTypes } from "../lib/schema/bit/bit";
import { useBackend } from "../state/backend-state";
import type {
	IConversation,
	IInboxEntry,
} from "../state/backend-state/inbox-state";
import type { ConversationCallState } from "./use-conversation-call";

/** The elected call participant supplies the bot's pinned voice to everyone in the call. */
export function useBotCallSpeech({
	conversation,
	entries,
	items,
	historyReady,
	directory,
	call,
}: {
	conversation: IConversation;
	entries: readonly IInboxEntry[];
	items: readonly ConversationItem[];
	historyReady: boolean;
	directory: CrewDirectory;
	call: ConversationCallState;
}) {
	const backend = useBackend();
	const { t } = useTranslation("bots");
	const callId = call.joined ? call.live?.callId : undefined;
	const desktop = backend.capabilities?.().canExecuteLocally ?? false;
	const pins = useMemo(
		() =>
			conversation.members
				.filter((member) => member.kind === "bot")
				.map((member) => ({
					id: member.principal_id,
					pin: directory(member.principal_id).ttsModel ?? null,
					pending: directory(member.principal_id).isPending,
				})),
		[conversation.members, directory],
	);
	const modelPins = [...new Set(pins.flatMap(({ pin }) => (pin ? [pin] : [])))];
	const queries = useQueries({
		queries: modelPins.map((pin) => ({
			queryKey: ["call-bot-voice", pin, desktop],
			enabled: Boolean(callId && pin && backend.audioState),
			queryFn: async (): Promise<BotSpeechVoice | null> => {
				if (!pin) return null;
				const reference = speechBitReference(pin);
				const bit = await backend.bitState.getBit(reference.id, reference.hub);
				if (bit.type !== IBitTypes.Tts) return null;
				const provider = bit.parameters?.provider?.provider_name;
				const voice =
					typeof bit.parameters?.default_voice === "string"
						? bit.parameters.default_voice.trim() || undefined
						: undefined;
				const maxInputCharacters = speechCharacterLimit(
					bit.parameters?.max_input_characters,
				);
				if (isHostedAudioProvider(provider))
					return { bitId: bit.id, local: false, voice, maxInputCharacters };
				if (
					desktop &&
					typeof provider === "string" &&
					provider.trim().toLowerCase() === "local:any-tts" &&
					(await backend.bitState.isBitInstalled(bit))
				)
					return {
						bitId:
							reference.hub || bit.hub
								? `${reference.hub ?? bit.hub}:${bit.id}`
								: bit.id,
						local: true,
						voice,
						maxInputCharacters,
					};
				return null;
			},
			staleTime: 30_000,
			retry: false,
		})),
	});
	const voices = new Map<string, BotSpeechVoice | null>();
	for (const { id, pin, pending } of pins) {
		const index = pin ? modelPins.indexOf(pin) : -1;
		if (pending) continue;
		if (!pin || queries[index].isError) voices.set(id, null);
		else if (queries[index].data !== undefined)
			voices.set(id, queries[index].data ?? null);
	}
	const capable = [...voices]
		.filter(([, voice]) => voice !== null)
		.map(([id]) => id)
		.sort();
	const capabilityKey = JSON.stringify(capable);
	const latest = useRef({
		entries,
		items,
		voices,
		call,
	});
	latest.current = { entries, items, voices, call };
	const scheduler = useRef<ReturnType<typeof createBotCallSpeech> | null>(null);

	useEffect(() => {
		call.setBotVoiceCapabilities?.(callId ? JSON.parse(capabilityKey) : []);
	}, [callId, capabilityKey, call.setBotVoiceCapabilities]);
	// biome-ignore lint/correctness/useExhaustiveDependencies: voice capabilities belong to the joined call
	useEffect(
		() => () => call.setBotVoiceCapabilities?.([]),
		[callId, call.setBotVoiceCapabilities],
	);

	// biome-ignore lint/correctness/useExhaustiveDependencies: changing conversations must cancel the previous call's queue
	useEffect(() => {
		const audio = backend.audioState;
		if (!callId || !historyReady || !audio) return;
		let reported = false;
		const controller = createBotCallSpeech({
			callId,
			electionDelayMs: 350,
			initialEntries: latest.current.entries,
			synthesize: (voice, text) =>
				audio.speak({
					bit_id: voice.bitId,
					text,
					local: voice.local,
					...(voice.voice ? { voice: voice.voice } : {}),
					format: "wav",
				}),
			play: (blob, botId) => latest.current.call.playBotSpeech(blob, botId),
			stop: () => latest.current.call.stopBotSpeech?.(),
			canSpeak: (botId) => latest.current.call.canSpeakForBot?.(botId) ?? false,
			onError: (error) => {
				console.warn("Bot call speech failed", error);
				if (reported) return;
				reported = true;
				toast.error(
					t(
						"crew.call.voiceFailed",
						"The bot's voice could not play. Its reply is still in the conversation.",
					),
				);
			},
		});
		scheduler.current = controller;
		return () => {
			controller.dispose();
			if (scheduler.current === controller) scheduler.current = null;
		};
	}, [conversation.id, callId, historyReady, backend.audioState, t]);

	useEffect(() => {
		scheduler.current?.update({ entries, items, voices });
	});
}
