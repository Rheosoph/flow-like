"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { joinContentText } from "../components/interfaces/chat-default/inline-segments";
import {
	type InterruptedSpeech,
	type StreamedReply,
	isBotEcho,
} from "../lib/realtime/bot-call-speech";
import { callLatency } from "../lib/realtime/call-latency";
import {
	type CallListener,
	startCallListener,
} from "../lib/realtime/call-listener";
import { warmCallListening } from "../lib/realtime/call-listener";
import {
	END_SILENCE_MS,
	type SttChoice,
	type Utterance,
	bytesToBase64,
	encodeWav,
} from "../lib/realtime/call-listening";
import {
	type CallSpeechOutput,
	type SpeechProgress,
	createCallSpeechOutput,
} from "../lib/realtime/call-speech-output";
import { createCallTranscription } from "../lib/realtime/call-transcription";
import {
	type CallMediaError,
	type CallVideoKind,
	type VideoCaptureState,
	createVideoCapture,
} from "../lib/realtime/call-video";
import {
	CALL_FIELD,
	type CallEngine,
	type InterruptRequest,
	type InterruptedSpeechRecord,
	type LiveCall,
	type RemoteStreams,
	botVoiceSource,
	byteLevel,
	callIsFull,
	callLiveKey,
	createCallEngine,
	createSpeakingDetector,
	deriveLiveCall,
	peerSubs,
} from "../lib/realtime/conversation-call";
import { useBackend } from "../state/backend-state";
import type { IAudioState } from "../state/backend-state/audio-state";
import type { IConversation } from "../state/backend-state/inbox-state";
import { useGlobalChatStore } from "../state/global-chat/global-chat-store";
import { cancelGlobalChatRun } from "../state/global-chat/global-chat-stream";
import {
	type VoiceReplyFollow,
	type VoiceRunSource,
	type VoiceTranscript,
	type VoiceTurnRequest,
	followVoiceReply,
	voiceTurnSender,
} from "../state/global-chat/voice-turn";
import type { ConversationRoom, RoomAwareness } from "./use-conversation-room";
import { newInboxMessageId, useSendInboxEntry } from "./use-inbox";
import { useIncomingCallJoin } from "./use-incoming-call-join";

export type CallError =
	| "denied"
	| "unavailable"
	| "full"
	| "failed"
	| "transcription";

export interface CallParticipantView {
	sub: string;
	muted: boolean;
	speaking: boolean;
	listening: boolean;
}

/** What the bot speech scheduler offers the call: cutting a reply off, and what is being said. */
export interface BotSpeechControl {
	interrupt: (filter?: {
		botId?: string;
		since?: number;
	}) => InterruptedSpeech | null;
	echoText: (botId: string) => string | null;
	/** Speaks a reply while it streams in; null when this device has no voice for the bot. */
	stream?: (transcriptId: string, botId: string) => StreamedReply | null;
	/** Holds the playing reply in place; false when nothing was playing. */
	pause?: () => boolean;
	resume?: () => void;
}

/** What letting bots listen needs: the speaker's STT bit and the bots a spoken name can wake. */
export interface CallListenOptions {
	stt: SttChoice | null;
	hasBots: boolean;
	/** The bot ids a spoken text names. */
	mentionsIn: (text: string) => string[];
}

interface VoiceRun {
	runId: string;
	transcriptId: string;
	finished: boolean;
	follow: VoiceReplyFollow | null;
}

interface SpokenUtterance extends Utterance {
	/** Wall-clock time the utterance began. */
	spokenAt: number;
}

export interface LiveCallView {
	callId: string;
	participants: readonly CallParticipantView[];
	/** When the call began, if its entry is known; otherwise null. */
	startedAt: number | null;
}

export interface CallVideoView {
	id: string;
	sub: string;
	kind: CallVideoKind;
	stream: MediaStream;
	local: boolean;
}

export interface ConversationCallOptions {
	/** Isolated meetings manage their own history and admission. */
	persistCallEvents?: boolean;
	stableCallId?: string;
}

export interface ConversationCallState {
	/** The room is up, so a call can be started or joined. */
	available: boolean;
	live: LiveCallView | null;
	joined: boolean;
	start: () => Promise<void>;
	join: () => Promise<void>;
	leave: () => Promise<void>;
	toggleMute: () => void;
	/** Bots are given a transcript of your microphone while you are in the call. */
	listening: boolean;
	/** A provisional transcript of your current utterance, visible only on this device. */
	transcriptDraft: string;
	/** The speaker has a speech model to transcribe with. */
	canListen: boolean;
	/** The conversation has bot members, so listening has someone to serve. */
	botsPresent: boolean;
	setListening: (on: boolean) => void;
	setBotVoiceCapabilities: (botIds: readonly string[]) => void;
	canSpeakForBot: (botId: string) => boolean;
	playBotSpeech: (blob: Blob, botId: string) => Promise<void>;
	stopBotSpeech: () => void;
	/** The bot whose voice is playing in the call, on this device or another. */
	speakingBot: string | null;
	/** Stops the speaking bot, wherever its voice plays; what was heard rides on your next transcript. */
	interruptSpeech: () => void;
	registerSpeechControl: (control: BotSpeechControl | null) => void;
	speechOutput: () => CallSpeechOutput | null;
	speechProgress: () => SpeechProgress | null;
	error: CallError | null;
	cameraOn: boolean;
	screenSharing: boolean;
	canUseCamera: boolean;
	canShareScreen: boolean;
	mediaPending: CallVideoKind | null;
	mediaError: CallMediaError | null;
	toggleCamera: () => Promise<void>;
	toggleScreenShare: () => Promise<void>;
	videos: readonly CallVideoView[];
}

const SPEAKING_POLL_MS = 150;
const BOT_ECHO_TAIL_MS = 350;
const INTERRUPT_CARRY_MS = 30_000;
const BARGE_HOLD_MS = 5000;
const BARGE_IN_PROBE_TIMEOUT_MS = 2000;
const MAX_TIMINGS = 16;
const NO_SPEAKERS: ReadonlySet<string> = new Set();
const NO_STARTS: ReadonlyMap<string, number> = new Map();
const NO_STREAMS: RemoteStreams = new Map();
const NO_VIDEO: VideoCaptureState = {
	camera: null,
	screen: null,
	pending: null,
	error: null,
};

interface Meter {
	sub: string;
	analyser: AnalyserNode;
	source: MediaStreamAudioSourceNode;
	buffer: Uint8Array<ArrayBuffer>;
	detector: ReturnType<typeof createSpeakingDetector>;
}

interface Output {
	stream: MediaStream;
	audio: HTMLAudioElement;
	meter: Meter | null;
}

interface Session {
	callId: string;
	conversationId: string;
	members: ReadonlySet<string>;
	awareness: RoomAwareness;
	peer: string;
	stream: MediaStream;
	context: AudioContext | null;
	contextChanged: (() => void) | null;
	localMeter: Meter | null;
	outputs: Map<string, Output>;
	poll: ReturnType<typeof setInterval> | null;
	joinedAt: number;
	muted: boolean;
	listening: boolean;
	botSpeaking: boolean;
	botEchoTimer: ReturnType<typeof setTimeout> | null;
	guardBot: string | null;
	bargeHold: number;
	bargeToken: number;
	speechControl: BotSpeechControl | null;
	interruptBot: InterruptRequest | undefined;
	interruptedSpeech: InterruptedSpeechRecord | undefined;
	pendingInterrupt: { record: InterruptedSpeech; at: number } | null;
	awaitingInterrupt: { at: number } | null;
	handledInterrupts: Map<number, number>;
	voiceBots: readonly string[];
	speakingBot: string | null;
	speechOutput: CallSpeechOutput | null;
	voiceRun: VoiceRun | null;
	bargePaused: boolean;
	listener: CallListener | null;
	listeningGeneration: number;
	transcripts: ReturnType<
		typeof createCallTranscription<SpokenUtterance>
	> | null;
	video: ReturnType<typeof createVideoCapture> | null;
}

function errorOf(cause: unknown): CallError {
	const name = cause instanceof DOMException ? cause.name : "";
	if (name === "NotAllowedError" || name === "SecurityError") return "denied";
	if (
		name === "NotFoundError" ||
		name === "NotReadableError" ||
		name === "OverconstrainedError" ||
		name === "TypeError"
	)
		return "unavailable";
	return "failed";
}

function makeMeter(
	context: AudioContext | null,
	stream: MediaStream,
	sub: string,
): Meter | null {
	if (!context) return null;
	try {
		const source = context.createMediaStreamSource(stream);
		const analyser = context.createAnalyser();
		analyser.fftSize = 512;
		source.connect(analyser);
		return {
			sub,
			analyser,
			source,
			buffer: new Uint8Array(analyser.fftSize),
			detector: createSpeakingDetector(),
		};
	} catch (cause) {
		console.warn("Speaking detection unavailable", cause);
		return null;
	}
}

function dropMeter(meter: Meter | null) {
	if (!meter) return;
	try {
		meter.source.disconnect();
		meter.analyser.disconnect();
	} catch {}
}

function playRemote(stream: MediaStream): HTMLAudioElement {
	const audio = new Audio();
	audio.autoplay = true;
	audio.srcObject = stream;
	void audio.play().catch((cause) => {
		console.warn("Playing call audio failed", cause);
	});
	return audio;
}

function stopOutput(output: Output) {
	dropMeter(output.meter);
	output.audio.pause();
	output.audio.srcObject = null;
}

function releaseBargeIn(session: Session) {
	session.bargeToken++;
	const paused = session.bargePaused;
	session.bargePaused = false;
	if (paused) session.speechControl?.resume?.();
	session.speechOutput?.duck(false);
}

function stopListening(session: Session) {
	session.listeningGeneration++;
	session.listening = false;
	releaseBargeIn(session);
	session.transcripts?.close();
	session.transcripts = null;
	session.listener?.stop();
	session.listener = null;
}

function cancelVoiceRun(session: Session): Promise<void> {
	const run = session.voiceRun;
	session.voiceRun = null;
	if (!run || run.finished) return Promise.resolve();
	run.finished = true;
	run.follow?.cancel();
	return cancelGlobalChatRun(run.runId).then(
		() => {},
		() => {},
	);
}

const voiceRunSource = (runId: string): VoiceRunSource => ({
	subscribe: (listener) => useGlobalChatStore.subscribe(listener),
	read: () => {
		const message = useGlobalChatStore.getState().runs[runId]?.message;
		return message
			? {
					text: joinContentText(message.inner.content),
					toolRunning: Boolean(message.current_step_id),
				}
			: null;
	},
});

function releaseSession(session: Session) {
	void cancelVoiceRun(session);
	if (session.contextChanged)
		session.context?.removeEventListener("statechange", session.contextChanged);
	if (session.botEchoTimer !== null) clearTimeout(session.botEchoTimer);
	session.speechOutput?.dispose();
	stopListening(session);
	session.video?.dispose();
	if (session.poll !== null) clearInterval(session.poll);
	for (const output of session.outputs.values()) stopOutput(output);
	session.outputs.clear();
	dropMeter(session.localMeter);
	for (const track of session.stream.getTracks()) track.stop();
	void session.context?.close().catch(() => {});
}

function holdingBargeIn(session: Session): boolean {
	return Date.now() < session.bargeHold;
}

function guardWhileBotSpeaks(session: Session, bot: string) {
	session.guardBot = bot;
	const wasInEchoTail = session.botEchoTimer !== null;
	if (session.botEchoTimer !== null) clearTimeout(session.botEchoTimer);
	session.botEchoTimer = null;
	if (!session.botSpeaking) {
		session.botSpeaking = true;
		session.transcripts?.resetPartial();
	}
	if (!holdingBargeIn(session)) session.listener?.setGuarded(true);
	// A new speech chunk cancels the echo tail and must restore microphone capture.
	if (wasInEchoTail) session.listener?.setPaused(session.muted);
}

/** What the guarded bot is saying, to tell its echo from a person talking over it. */
function guardedSpeech(session: Session) {
	return session.guardBot
		? (session.speechControl?.echoText(session.guardBot) ?? "")
		: "";
}

function othersIn(session: Session): number {
	const live = deriveLiveCall(
		session.awareness.getStates(),
		session.awareness.clientID,
		session.members,
	);
	return (
		live?.participants.filter((p) => !p.local && live.callId === session.callId)
			.length ?? 0
	);
}

function sameSet(a: ReadonlySet<string>, b: ReadonlySet<string>): boolean {
	if (a.size !== b.size) return false;
	for (const value of a) if (!b.has(value)) return false;
	return true;
}

function publish(session: Session) {
	session.awareness.setLocalStateField(CALL_FIELD, {
		id: session.callId,
		joined: true,
		muted: session.muted,
		peer: session.peer,
		listening: session.listening,
		voiceBots: session.voiceBots,
		speakingBot: session.speakingBot,
		interruptBot: session.interruptBot,
		interruptedSpeech: session.interruptedSpeech,
		camera: session.video?.state.camera?.id,
		screen: session.video?.state.screen?.id,
	});
}

function localStreams(session: Session): MediaStream[] {
	return [
		session.context?.state === "running"
			? (session.speechOutput?.stream ?? session.stream)
			: session.stream,
		session.video?.state.camera,
		session.video?.state.screen,
	].filter((stream): stream is MediaStream => Boolean(stream));
}

/** `room` is the conversation's peer room from `useConversationRoom`, shared with typing. */
export function useConversationCall(
	conversation: IConversation,
	me: string | null | undefined,
	room: ConversationRoom | undefined,
	startTimes: ReadonlyMap<string, number> = NO_STARTS,
	listen?: CallListenOptions,
	options?: ConversationCallOptions,
): ConversationCallState {
	const { mutateAsync: sendEntry } = useSendInboxEntry();
	const audio = useBackend().audioState;
	const optionsRef = useRef(options);
	optionsRef.current = options;
	const listenRef = useRef<{
		listen?: CallListenOptions;
		audio?: IAudioState;
	}>({});
	listenRef.current = { listen, audio };
	const peopleKey = conversation.members
		.filter((member) => member.kind === "user")
		.map((member) => member.principal_id)
		.sort()
		.join("\0");
	const people = useMemo(
		() => new Set(peopleKey.split("\0").filter(Boolean)),
		[peopleKey],
	);

	const [live, setLive] = useState<LiveCall | null>(null);
	const [remote, setRemote] = useState<RemoteStreams>(NO_STREAMS);
	const [joined, setJoined] = useState(false);
	const [speaking, setSpeaking] = useState<ReadonlySet<string>>(NO_SPEAKERS);
	const [error, setError] = useState<CallError | null>(null);
	const [transcriptDraft, setTranscriptDraft] = useState("");
	const [video, setVideo] = useState<VideoCaptureState>(NO_VIDEO);
	const joinAttempt = useRef(0);
	const joining = useRef(false);

	const sessionRef = useRef<Session | null>(null);
	const pendingControl = useRef<BotSpeechControl | null>(null);
	const engineRef = useRef<CallEngine | null>(null);
	const updateCallRef = useRef<(() => void) | null>(null);
	const roomRef = useRef<ConversationRoom | undefined>(room);
	roomRef.current = room;
	const liveRef = useRef<LiveCall | null>(null);
	liveRef.current = live;
	const peopleRef = useRef(people);
	peopleRef.current = people;
	const botsRef = useRef(new Set<string>());
	botsRef.current = new Set(
		conversation.members
			.filter((member) => member.kind === "bot")
			.map((member) => member.principal_id),
	);
	const directBotRef = useRef<string | null>(null);
	directBotRef.current =
		conversation.kind === "direct" && botsRef.current.size === 1
			? ([...botsRef.current][0] ?? null)
			: null;
	const startTimesRef = useRef(startTimes);
	startTimesRef.current = startTimes;

	const remoteSpeakingBot = useCallback(
		(session: Session, current: LiveCall | null): string | null =>
			(current?.callId === session.callId
				? current.participants.find(
						(p) =>
							!p.local && p.speakingBot && botsRef.current.has(p.speakingBot),
					)?.speakingBot
				: null) ?? null,
		[],
	);

	const refreshSpeechPause = useCallback(
		(session: Session, current: LiveCall | null) => {
			const bot = session.speakingBot ?? remoteSpeakingBot(session, current);
			if (bot) {
				guardWhileBotSpeaks(session, bot);
			} else if (session.botSpeaking && session.botEchoTimer === null) {
				session.bargeToken++;
				session.speechOutput?.duck(false);
				if (session.bargeHold) {
					session.bargeHold = 0;
					session.botSpeaking = false;
					session.guardBot = null;
					session.listener?.setGuarded(false);
					return;
				}
				// Allow playback already buffered by WebRTC to finish before listening again.
				session.listener?.setPaused(true);
				session.botEchoTimer = setTimeout(() => {
					session.botEchoTimer = null;
					if (sessionRef.current !== session) return;
					session.botSpeaking = false;
					session.guardBot = null;
					session.listener?.setGuarded(false);
					session.listener?.setPaused(session.muted);
				}, BOT_ECHO_TAIL_MS);
			}
		},
		[remoteSpeakingBot],
	);

	const claimRemoteInterrupt = useCallback(
		(session: Session, current: LiveCall | null) => {
			const awaiting = session.awaitingInterrupt;
			if (!awaiting || current?.callId !== session.callId) return;
			const report = current.participants.find(
				(p) => !p.local && (p.interruptedSpeech?.at ?? 0) >= awaiting.at,
			)?.interruptedSpeech;
			if (!report) return;
			session.awaitingInterrupt = null;
			session.pendingInterrupt = {
				record: {
					message_id: report.message_id,
					heard_chars: report.heard_chars,
					spoken_chars: report.spoken_chars,
				},
				at: Date.now(),
			};
		},
		[],
	);

	const serveInterrupts = useCallback(
		(session: Session, current: LiveCall | null) => {
			if (current?.callId !== session.callId) return;
			for (const p of current.participants) {
				const request = p.interruptBot;
				if (
					p.local ||
					!request ||
					!botsRef.current.has(request.botId) ||
					(session.handledInterrupts.get(p.clientId) ?? 0) >= request.at
				)
					continue;
				session.handledInterrupts.set(p.clientId, request.at);
				const record = session.speechControl?.interrupt({
					botId: request.botId,
					since: request.at,
				});
				if (!record) continue;
				session.interruptedSpeech = { ...record, at: Date.now() };
				publish(session);
			}
		},
		[],
	);

	const interruptSession = useCallback(
		(session: Session) => {
			const record = session.speechControl?.interrupt();
			if (record) session.pendingInterrupt = { record, at: Date.now() };
			session.speechOutput?.duck(false);
			const remoteBot = remoteSpeakingBot(session, liveRef.current);
			if (!remoteBot) return;
			const at = Date.now();
			session.interruptBot = { botId: remoteBot, at };
			session.awaitingInterrupt = { at };
			publish(session);
		},
		[remoteSpeakingBot],
	);

	const takeInterrupt = useCallback(
		(session: Session): InterruptedSpeech | undefined => {
			claimRemoteInterrupt(
				session,
				deriveLiveCall(
					session.awareness.getStates(),
					session.awareness.clientID,
					session.members,
				),
			);
			const pending = session.pendingInterrupt;
			session.pendingInterrupt = null;
			return pending && Date.now() - pending.at <= INTERRUPT_CARRY_MS
				? pending.record
				: undefined;
		},
		[claimRemoteInterrupt],
	);

	useEffect(() => {
		if (!room) {
			setLive(null);
			setRemote(NO_STREAMS);
			return;
		}
		const { awareness, provider } = room;
		const engine = createCallEngine(provider);
		engine.setAllowedPeers(new Set());
		engineRef.current = engine;
		const stopRemote = engine.onRemote(setRemote);
		let lastKey = "";
		const update = () => {
			const next = deriveLiveCall(
				awareness.getStates(),
				awareness.clientID,
				peopleRef.current,
			);
			if (sessionRef.current) {
				refreshSpeechPause(sessionRef.current, next);
				serveInterrupts(sessionRef.current, next);
				claimRemoteInterrupt(sessionRef.current, next);
			}
			engine.setAllowedPeers(
				new Set(peerSubs(next?.participants ?? []).keys()),
			);
			const key = callLiveKey(next);
			if (key === lastKey) return;
			lastKey = key;
			setLive(next);
		};
		updateCallRef.current = update;
		awareness.on("change", update);
		update();
		const session = sessionRef.current;
		if (session) {
			session.awareness = awareness;
			session.peer = provider.room?.peerId ?? session.peer;
			publish(session);
			engine.setLocalStreams(localStreams(session));
		}
		return () => {
			awareness.off("change", update);
			stopRemote();
			engine.dispose();
			if (engineRef.current === engine) engineRef.current = null;
			if (updateCallRef.current === update) updateCallRef.current = null;
		};
	}, [room, refreshSpeechPause, serveInterrupts, claimRemoteInterrupt]);

	useEffect(() => {
		if (sessionRef.current) sessionRef.current.members = people;
		updateCallRef.current?.();
	}, [people]);

	const reconcileOutputs = useCallback(
		(session: Session, streams: RemoteStreams, current: LiveCall | null) => {
			const subs = peerSubs(current?.participants ?? []);
			const wanted = new Map<string, { sub: string; stream: MediaStream }>();
			for (const [peerId, peerStreams] of streams) {
				const sub = subs.get(peerId);
				if (!sub || sub === me) continue;
				for (const stream of peerStreams) {
					if (
						stream
							.getAudioTracks()
							.some((track) => track.readyState !== "ended")
					)
						wanted.set(`${peerId}:${stream.id}`, { sub, stream });
				}
			}
			for (const [peerId, output] of session.outputs) {
				const next = wanted.get(peerId);
				if (next?.stream === output.stream) continue;
				stopOutput(output);
				session.outputs.delete(peerId);
			}
			for (const [peerId, { sub, stream }] of wanted) {
				if (session.outputs.has(peerId)) continue;
				session.outputs.set(peerId, {
					stream,
					audio: playRemote(stream),
					meter: makeMeter(session.context, stream, sub),
				});
			}
		},
		[me],
	);

	useEffect(() => {
		const session = sessionRef.current;
		if (!session || !joined) return;
		reconcileOutputs(session, remote, live);
	}, [joined, remote, live, reconcileOutputs]);

	const release = useCallback(() => {
		const session = sessionRef.current;
		sessionRef.current = null;
		if (!session) return;
		releaseSession(session);
		setJoined(false);
		setVideo(NO_VIDEO);
		setSpeaking(NO_SPEAKERS);
		setTranscriptDraft("");
	}, []);

	const leave = useCallback(async () => {
		joinAttempt.current++;
		joining.current = false;
		const session = sessionRef.current;
		if (!session) return;
		const last = othersIn(session) === 0;
		try {
			session.awareness.setLocalStateField(CALL_FIELD, undefined);
		} catch {}
		engineRef.current?.setLocalStream(null);
		release();
		if (!last || optionsRef.current?.persistCallEvents === false) return;
		const started = startTimesRef.current.get(session.callId);
		try {
			await sendEntry({
				conversationId: session.conversationId,
				request: {
					message_id: newInboxMessageId(),
					kind: "call",
					envelope: {
						v: 1,
						enc: "none",
						body: {
							op: "ended",
							call: session.callId,
							duration_ms: Math.max(
								0,
								Date.now() - (started ?? session.joinedAt),
							),
						},
					},
				},
			});
		} catch (cause) {
			console.warn("Ending the call line failed", cause);
		}
	}, [release, sendEntry]);

	const leaveRef = useRef(leave);
	leaveRef.current = leave;
	useEffect(() => {
		if (!room || !me || !people.has(me)) void leaveRef.current();
	}, [room, me, people]);
	const conversationId = conversation.id;
	// biome-ignore lint/correctness/useExhaustiveDependencies: leaving belongs to the conversation the call began in
	useEffect(() => () => void leaveRef.current(), [conversationId]);

	const joinCall = useCallback(
		async (
			callId: string,
			preparedContext?: AudioContext | null,
		): Promise<boolean> => {
			const target = roomRef.current;
			const peer = target?.provider.room?.peerId;
			if (
				!target ||
				!peer ||
				!me ||
				sessionRef.current ||
				joining.current ||
				!peopleRef.current.has(me) ||
				(preparedContext !== undefined && liveRef.current?.callId !== callId)
			) {
				void preparedContext?.close().catch(() => {});
				return false;
			}
			if (callIsFull(liveRef.current, me)) {
				void preparedContext?.close().catch(() => {});
				setError("full");
				return false;
			}
			setError(null);
			const attempt = ++joinAttempt.current;
			joining.current = true;
			let stream: MediaStream;
			let context: AudioContext | null = preparedContext ?? null;
			try {
				if (!navigator.mediaDevices?.getUserMedia) {
					void context?.close().catch(() => {});
					setError("unavailable");
					joining.current = false;
					return false;
				}
				try {
					context ??= new AudioContext();
					void context.resume().catch(() => {});
				} catch (cause) {
					console.warn("Audio analysis unavailable", cause);
				}
				stream = await navigator.mediaDevices.getUserMedia({
					audio: {
						echoCancellation: true,
						noiseSuppression: true,
						autoGainControl: true,
					},
				});
			} catch (cause) {
				void context?.close().catch(() => {});
				console.warn("Microphone unavailable", cause);
				if (attempt === joinAttempt.current) {
					joining.current = false;
					setError(errorOf(cause));
				}
				return false;
			}
			if (attempt === joinAttempt.current) joining.current = false;
			if (
				attempt !== joinAttempt.current ||
				sessionRef.current ||
				roomRef.current !== target ||
				!peopleRef.current.has(me) ||
				(preparedContext !== undefined && liveRef.current?.callId !== callId) ||
				callIsFull(liveRef.current, me)
			) {
				for (const track of stream.getTracks()) track.stop();
				void context?.close().catch(() => {});
				return false;
			}
			const session: Session = {
				callId,
				conversationId,
				members: peopleRef.current,
				awareness: target.awareness,
				peer,
				stream,
				context,
				contextChanged: null,
				localMeter: makeMeter(context, stream, me),
				outputs: new Map(),
				poll: null,
				joinedAt: Date.now(),
				muted: false,
				listening: false,
				botSpeaking: false,
				botEchoTimer: null,
				guardBot: null,
				bargeHold: 0,
				bargeToken: 0,
				speechControl: null,
				interruptBot: undefined,
				interruptedSpeech: undefined,
				pendingInterrupt: null,
				awaitingInterrupt: null,
				handledInterrupts: new Map(),
				voiceBots: [],
				speakingBot: null,
				speechOutput: null,
				voiceRun: null,
				bargePaused: false,
				listener: null,
				listeningGeneration: 0,
				transcripts: null,
				video: null,
			};
			session.speechControl = pendingControl.current;
			if (context && optionsRef.current?.persistCallEvents !== false) {
				try {
					session.speechOutput = createCallSpeechOutput(
						context,
						stream,
						(botId) => {
							if (sessionRef.current !== session) return;
							session.speakingBot = botId;
							refreshSpeechPause(session, liveRef.current);
							publish(session);
						},
					);
				} catch (cause) {
					console.warn("Bot voice playback unavailable", cause);
				}
			}
			session.video = createVideoCapture((state) => {
				if (sessionRef.current !== session) return;
				setVideo(state);
				publish(session);
				engineRef.current?.setLocalStreams(localStreams(session));
			});
			session.poll = setInterval(() => {
				const now = performance.now();
				const next = new Set<string>();
				const meters = [
					session.localMeter,
					...[...session.outputs.values()].map((o) => o.meter),
				];
				for (const meter of meters) {
					if (!meter) continue;
					meter.analyser.getByteTimeDomainData(meter.buffer);
					if (meter.detector.update(byteLevel(meter.buffer), now)) {
						next.add(meter.sub);
					}
				}
				if (session.muted) next.delete(me);
				setSpeaking((current) => (sameSet(current, next) ? current : next));
			}, SPEAKING_POLL_MS);
			sessionRef.current = session;
			session.contextChanged = () => {
				if (sessionRef.current === session)
					engineRef.current?.setLocalStreams(localStreams(session));
			};
			context?.addEventListener("statechange", session.contextChanged);
			publish(session);
			engineRef.current?.setLocalStreams(localStreams(session));
			setJoined(true);
			return true;
		},
		[conversationId, me, refreshSpeechPause],
	);

	useIncomingCallJoin({
		userId: me,
		conversationId,
		liveCallId: live?.callId,
		ready: Boolean(room?.provider.room?.peerId) && !joined,
		join: joinCall,
	});

	const join = useCallback(async () => {
		const target = optionsRef.current?.stableCallId ?? liveRef.current?.callId;
		if (target) await joinCall(target);
	}, [joinCall]);

	const start = useCallback(async () => {
		const existing =
			optionsRef.current?.stableCallId ?? liveRef.current?.callId;
		if (existing) {
			await joinCall(existing);
			return;
		}
		const callId = newInboxMessageId();
		if (!(await joinCall(callId))) return;
		if (optionsRef.current?.persistCallEvents === false) return;
		try {
			await sendEntry({
				conversationId,
				request: {
					message_id: callId,
					kind: "call",
					envelope: { v: 1, enc: "none", body: { op: "started" } },
				},
			});
		} catch (cause) {
			console.warn("Starting the call failed", cause);
			setError("failed");
			const session = sessionRef.current;
			if (session) {
				try {
					session.awareness.setLocalStateField(CALL_FIELD, undefined);
				} catch {}
				engineRef.current?.setLocalStream(null);
				release();
			}
		}
	}, [conversationId, joinCall, release, sendEntry]);

	const toggleCamera = useCallback(async () => {
		await sessionRef.current?.video?.toggle("camera");
	}, []);
	const toggleScreenShare = useCallback(async () => {
		await sessionRef.current?.video?.toggle("screen");
	}, []);

	const toggleMute = useCallback(() => {
		const session = sessionRef.current;
		if (!session) return;
		session.muted = !session.muted;
		for (const track of session.stream.getAudioTracks()) {
			track.enabled = !session.muted;
		}
		const tail = session.botEchoTimer !== null;
		session.listener?.setPaused(session.muted || tail);
		if (session.muted) releaseBargeIn(session);
		if (session.muted || tail) session.transcripts?.resetPartial();
		publish(session);
	}, []);

	const setBotVoiceCapabilities = useCallback((botIds: readonly string[]) => {
		const session = sessionRef.current;
		if (!session) return;
		const next = session.speechOutput
			? [...new Set(botIds)].filter((id) => botsRef.current.has(id)).sort()
			: [];
		if (session.voiceBots.join("\0") === next.join("\0")) return;
		session.voiceBots = next;
		if (session.speakingBot && !next.includes(session.speakingBot))
			session.speechOutput?.stop();
		publish(session);
	}, []);

	const canSpeakForBot = useCallback((botId: string) => {
		const session = sessionRef.current;
		if (!session?.speechOutput || !botsRef.current.has(botId)) return false;
		const current = deriveLiveCall(
			session.awareness.getStates(),
			session.awareness.clientID,
			session.members,
		);
		return (
			current?.callId === session.callId &&
			botVoiceSource(current, botId)?.local === true
		);
	}, []);

	const playBotSpeech = useCallback(
		async (blob: Blob, botId: string) => {
			const session = sessionRef.current;
			if (!session || !canSpeakForBot(botId)) return;
			await session.speechOutput?.play(blob, botId);
		},
		[canSpeakForBot],
	);

	const stopBotSpeech = useCallback(() => {
		sessionRef.current?.speechOutput?.stop();
	}, []);

	const interruptSpeech = useCallback(() => {
		const session = sessionRef.current;
		if (!session) return;
		interruptSession(session);
		void cancelVoiceRun(session);
	}, [interruptSession]);

	const registerSpeechControl = useCallback(
		(control: BotSpeechControl | null) => {
			const session = sessionRef.current;
			if (session) session.speechControl = control;
			pendingControl.current = control;
		},
		[],
	);

	const speechOutput = useCallback(
		() => sessionRef.current?.speechOutput ?? null,
		[],
	);

	const speechProgress = useCallback(
		() => sessionRef.current?.speechOutput?.progress() ?? null,
		[],
	);

	const startVoiceTurn = useCallback(
		async (
			session: Session,
			request: VoiceTurnRequest,
		): Promise<{ start: (transcriptId: string) => void } | null> => {
			const botId = directBotRef.current;
			const sender = botId ? voiceTurnSender(botId) : null;
			const stream = session.speechControl?.stream;
			if (!botId || !sender || !stream) return null;
			try {
				await cancelVoiceRun(session);
				const handle = await sender(request);
				if (!handle) return null;
				const run: VoiceRun = {
					runId: handle.runId,
					transcriptId: request.messageId,
					finished: false,
					follow: null,
				};
				void handle.done.then(
					() => {
						run.finished = true;
					},
					() => {
						run.finished = true;
					},
				);
				if (sessionRef.current !== session) {
					void cancelGlobalChatRun(handle.runId).catch(() => {});
					return { start: () => {} };
				}
				session.voiceRun = run;
				return {
					start: (transcriptId) => {
						if (session.voiceRun !== run) return;
						const reply = stream(transcriptId, botId) ?? null;
						run.follow = followVoiceReply(
							reply,
							voiceRunSource(run.runId),
							handle.done,
							() => callLatency.firstText(transcriptId),
						);
					},
				};
			} catch (cause) {
				console.warn("Starting the voice turn failed", cause);
				return null;
			}
		},
		[],
	);

	const setListening = useCallback(
		(on: boolean) => {
			const session = sessionRef.current;
			if (!session) return;
			if (!on) {
				stopListening(session);
				setTranscriptDraft("");
				publish(session);
				return;
			}
			const { listen: listenOptions, audio: audioState } = listenRef.current;
			const stt = listenOptions?.stt;
			if (
				session.listening ||
				!stt ||
				!audioState ||
				optionsRef.current?.persistCallEvents === false
			)
				return;
			session.listening = true;
			const generation = ++session.listeningGeneration;
			const isCurrent = () =>
				sessionRef.current === session &&
				session.listening &&
				session.listeningGeneration === generation &&
				optionsRef.current?.persistCallEvents !== false;
			const timings = new Map<string, { startAt: number; doneAt: number }>();
			const transcripts = createCallTranscription<SpokenUtterance>({
				transcribe: async (utterance) => {
					const { listen: currentListen, audio: currentAudio } =
						listenRef.current;
					const currentStt = currentListen?.stt;
					if (!isCurrent() || !currentStt || !currentAudio) return "";
					const startAt = Date.now();
					const wav = encodeWav(utterance.samples);
					const local = currentStt.route === "local";
					const result = await currentAudio.transcribe({
						bit_id: currentStt.bitId,
						audio_base64: local ? "" : bytesToBase64(wav),
						...(local ? { audio_bytes: wav } : {}),
						mime: "audio/wav",
						local,
					});
					timings.set(`${utterance.offsetMs}:${utterance.samples.length}`, {
						startAt,
						doneAt: Date.now(),
					});
					if (timings.size > MAX_TIMINGS)
						timings.delete(timings.keys().next().value as string);
					return result.text.trim();
				},
				onPartial: (text) => {
					if (isCurrent()) setTranscriptDraft(text);
				},
				onFinal: async (text, utterance) => {
					if (!isCurrent()) return;
					const origin =
						startTimesRef.current.get(session.callId) ?? session.joinedAt;
					const messageId = newInboxMessageId();
					const postedAt = Date.now();
					const timing = timings.get(
						`${utterance.offsetMs}:${utterance.samples.length}`,
					);
					const interrupted = takeInterrupt(session);
					const transcript: VoiceTranscript = {
						call: session.callId,
						offset_ms: Math.max(0, Math.round(utterance.spokenAt - origin)),
						duration_ms: utterance.durationMs,
						mentions: listenRef.current.listen?.mentionsIn(text) ?? [],
						...(interrupted ? { interrupted } : {}),
					};
					const streamed = await startVoiceTurn(session, {
						messageId,
						text,
						transcript,
					});
					callLatency.transcriptPosted(messageId, {
						speechEndAt:
							utterance.endOfSpeechAt ??
							utterance.spokenAt + utterance.durationMs,
						sttStartAt: timing?.startAt ?? postedAt,
						sttDoneAt: timing?.doneAt ?? postedAt,
						postedAt,
						route: streamed ? "stream" : "queued",
						turnWaitMs: utterance.turn?.waitedMs,
						turnSource: utterance.turn?.source,
					});
					if (streamed) {
						streamed.start(messageId);
						return;
					}
					await sendEntry({
						conversationId: session.conversationId,
						request: {
							message_id: messageId,
							kind: "transcript",
							envelope: {
								v: 1,
								enc: "none",
								body: { ...transcript, text },
							},
						},
					});
				},
				onError: (cause) => {
					if (!isCurrent()) return;
					console.warn("Transcribing call audio failed", cause);
					setError("transcription");
				},
			});
			session.transcripts = transcripts;
			setTranscriptDraft("");
			setError(null);
			publish(session);
			const captured = (
				utterance: Utterance,
				final: boolean,
			): SpokenUtterance => ({
				...utterance,
				spokenAt:
					utterance.capturedAt ??
					Date.now() - (final ? END_SILENCE_MS : 0) - utterance.durationMs,
			});
			const heardInProbe = async (probe: Utterance) => {
				let timeout: ReturnType<typeof setTimeout> | undefined;
				try {
					return await Promise.race([
						transcripts.probe(captured(probe, false)),
						new Promise<null>((resolve) => {
							timeout = setTimeout(
								() => resolve(null),
								BARGE_IN_PROBE_TIMEOUT_MS,
							);
						}),
					]);
				} catch (cause) {
					console.warn("Checking a barge-in failed", cause);
					return null;
				} finally {
					clearTimeout(timeout);
				}
			};
			const confirmBargeIn = async (probe: Utterance, token: number) => {
				const text = await heardInProbe(probe);
				if (!isCurrent() || session.bargeToken !== token || session.muted)
					return;
				if (text === "" && session.listener?.retryBargeIn()) return;
				const confirmed =
					text !== null &&
					text !== "" &&
					!isBotEcho(text, guardedSpeech(session));
				if (confirmed) {
					session.bargeHold = Date.now() + BARGE_HOLD_MS;
					interruptSession(session);
					void cancelVoiceRun(session);
				} else if (session.bargePaused) session.speechControl?.resume?.();
				else session.speechOutput?.duck(false);
				session.bargePaused = false;
				session.listener?.endBargeIn(confirmed);
			};
			startCallListener(
				session.stream,
				(utterance) => transcripts.final(captured(utterance, true)),
				{
					context: session.context ?? undefined,
					paused: session.muted || session.botEchoTimer !== null,
					guarded: session.botSpeaking && !holdingBargeIn(session),
					onPartial: (utterance) => {
						if (isCurrent() && !session.muted)
							transcripts.partial(captured(utterance, false));
					},
					onReset: () => transcripts.resetPartial(),
					onBargeInStart: () => {
						if (!isCurrent()) return;
						session.bargeToken++;
						session.bargePaused = session.speechControl?.pause?.() ?? false;
						if (!session.bargePaused) session.speechOutput?.duck(true);
					},
					onBargeInProbe: (probe) =>
						void confirmBargeIn(probe, session.bargeToken),
				},
			)
				.then((listener) => {
					if (!isCurrent()) {
						listener.stop();
						return;
					}
					listener.setGuarded(session.botSpeaking && !holdingBargeIn(session));
					listener.setPaused(session.muted || session.botEchoTimer !== null);
					session.listener = listener;
				})
				.catch((cause) => {
					console.warn("Listening to the microphone failed", cause);
					if (!isCurrent()) return;
					stopListening(session);
					setTranscriptDraft("");
					const failure = errorOf(cause);
					setError(failure === "failed" ? "transcription" : failure);
					publish(session);
				});
		},
		[sendEntry, interruptSession, takeInterrupt, startVoiceTurn],
	);

	const speakingBot = useMemo(
		() =>
			live?.participants.find(
				(p) => p.speakingBot && botsRef.current.has(p.speakingBot),
			)?.speakingBot ?? null,
		[live],
	);

	const view = useMemo<LiveCallView | null>(
		() =>
			live
				? {
						callId: live.callId,
						startedAt: startTimes.get(live.callId) ?? null,
						participants: live.participants.map((p) => ({
							sub: p.sub,
							muted: p.muted,
							speaking: speaking.has(p.sub) && !p.muted,
							listening: p.listening,
						})),
					}
				: null,
		[live, speaking, startTimes],
	);

	const videos = useMemo<readonly CallVideoView[]>(() => {
		if (!joined || !me) return [];
		const result: CallVideoView[] = [];
		for (const kind of ["screen", "camera"] as const) {
			const stream = video[kind];
			if (stream)
				result.push({
					id: `${me}:${kind}:${stream.id}`,
					sub: me,
					kind,
					stream,
					local: true,
				});
		}
		const peers = peerSubs(live?.participants ?? []);
		for (const participant of live?.participants ?? []) {
			if (participant.sub === me || !peers.has(participant.peer)) continue;
			for (const kind of ["screen", "camera"] as const) {
				const id = participant[kind];
				if (!id) continue;
				const stream = remote
					.get(participant.peer)
					?.find((candidate) => candidate.id === id);
				if (
					stream?.getVideoTracks().some((track) => track.readyState !== "ended")
				)
					result.push({
						id: `${participant.sub}:${kind}:${id}`,
						sub: participant.sub,
						kind,
						stream,
						local: false,
					});
			}
		}
		return result;
	}, [joined, me, video, live, remote]);

	const sttBitId = listen?.stt?.bitId;
	const sttRoute = listen?.stt?.route;
	useEffect(() => {
		if (!joined) return;
		void warmCallListening().catch(() => {});
	}, [joined]);
	useEffect(() => {
		if (!joined || !sttBitId) return;
		void listenRef.current.audio
			?.warm?.({ stt: { bit_id: sttBitId, local: sttRoute === "local" } })
			?.catch(() => {});
	}, [joined, sttBitId, sttRoute]);

	const listeningNow =
		view?.participants.find((p) => p.sub === me)?.listening ?? false;

	return useMemo(
		() => ({
			available: room !== undefined,
			live: view,
			joined,
			start,
			join,
			leave,
			toggleMute,
			listening: listeningNow,
			transcriptDraft,
			canListen: Boolean(
				listen?.stt && audio && options?.persistCallEvents !== false,
			),
			botsPresent: listen?.hasBots ?? false,
			setListening,
			setBotVoiceCapabilities,
			canSpeakForBot,
			playBotSpeech,
			stopBotSpeech,
			speakingBot,
			interruptSpeech,
			registerSpeechControl,
			speechOutput,
			speechProgress,
			error,
			cameraOn: video.camera !== null,
			screenSharing: video.screen !== null,
			canUseCamera:
				typeof navigator !== "undefined" &&
				typeof navigator.mediaDevices?.getUserMedia === "function",
			canShareScreen:
				typeof navigator !== "undefined" &&
				typeof navigator.mediaDevices?.getDisplayMedia === "function",
			mediaPending: video.pending,
			mediaError: video.error,
			toggleCamera,
			toggleScreenShare,
			videos,
		}),
		[
			room,
			view,
			joined,
			start,
			join,
			leave,
			toggleMute,
			listeningNow,
			transcriptDraft,
			listen?.stt,
			listen?.hasBots,
			audio,
			setListening,
			setBotVoiceCapabilities,
			canSpeakForBot,
			playBotSpeech,
			stopBotSpeech,
			speakingBot,
			interruptSpeech,
			registerSpeechControl,
			speechOutput,
			speechProgress,
			error,
			video,
			videos,
			toggleCamera,
			toggleScreenShare,
			options?.persistCallEvents,
		],
	);
}
