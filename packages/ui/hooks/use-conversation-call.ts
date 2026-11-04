"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
	type CallListener,
	startCallListener,
} from "../lib/realtime/call-listener";
import {
	END_SILENCE_MS,
	type SttChoice,
	type Utterance,
	bytesToBase64,
	createSerialQueue,
	encodeWav,
} from "../lib/realtime/call-listening";
import {
	type CallMediaError,
	type CallVideoKind,
	type VideoCaptureState,
	createVideoCapture,
} from "../lib/realtime/call-video";
import {
	CALL_FIELD,
	type CallEngine,
	type LiveCall,
	type RemoteStreams,
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
import type { ConversationRoom, RoomAwareness } from "./use-conversation-room";
import { newInboxMessageId, useSendInboxEntry } from "./use-inbox";

export type CallError = "denied" | "unavailable" | "full" | "failed";

export interface CallParticipantView {
	sub: string;
	muted: boolean;
	speaking: boolean;
	listening: boolean;
}

/** What letting bots listen needs: the speaker's STT bit and the bots a spoken name can wake. */
export interface CallListenOptions {
	stt: SttChoice | null;
	hasBots: boolean;
	/** The bot ids a spoken text names. */
	mentionsIn: (text: string) => string[];
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
	/** The speaker has a speech model to transcribe with. */
	canListen: boolean;
	/** The conversation has bot members, so listening has someone to serve. */
	botsPresent: boolean;
	setListening: (on: boolean) => void;
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
	localMeter: Meter | null;
	outputs: Map<string, Output>;
	poll: ReturnType<typeof setInterval> | null;
	joinedAt: number;
	muted: boolean;
	listening: boolean;
	listener: CallListener | null;
	transcripts: ReturnType<typeof createSerialQueue<SpokenUtterance>> | null;
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

function stopListening(session: Session) {
	session.listening = false;
	session.transcripts?.close();
	session.transcripts = null;
	session.listener?.stop();
	session.listener = null;
}

function releaseSession(session: Session) {
	stopListening(session);
	session.video?.dispose();
	if (session.poll !== null) clearInterval(session.poll);
	for (const output of session.outputs.values()) stopOutput(output);
	session.outputs.clear();
	dropMeter(session.localMeter);
	for (const track of session.stream.getTracks()) track.stop();
	void session.context?.close().catch(() => {});
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
		camera: session.video?.state.camera?.id,
		screen: session.video?.state.screen?.id,
	});
}

function localStreams(session: Session): MediaStream[] {
	return [
		session.stream,
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
	const [video, setVideo] = useState<VideoCaptureState>(NO_VIDEO);
	const joinAttempt = useRef(0);
	const joining = useRef(false);

	const sessionRef = useRef<Session | null>(null);
	const engineRef = useRef<CallEngine | null>(null);
	const updateCallRef = useRef<(() => void) | null>(null);
	const roomRef = useRef<ConversationRoom | undefined>(room);
	roomRef.current = room;
	const liveRef = useRef<LiveCall | null>(null);
	liveRef.current = live;
	const peopleRef = useRef(people);
	peopleRef.current = people;
	const startTimesRef = useRef(startTimes);
	startTimesRef.current = startTimes;

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
	}, [room]);

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
		async (callId: string): Promise<boolean> => {
			const target = roomRef.current;
			const peer = target?.provider.room?.peerId;
			if (
				!target ||
				!peer ||
				!me ||
				sessionRef.current ||
				joining.current ||
				!peopleRef.current.has(me)
			)
				return false;
			if (callIsFull(liveRef.current, me)) {
				setError("full");
				return false;
			}
			setError(null);
			const attempt = ++joinAttempt.current;
			joining.current = true;
			let stream: MediaStream;
			try {
				if (!navigator.mediaDevices?.getUserMedia) {
					setError("unavailable");
					joining.current = false;
					return false;
				}
				stream = await navigator.mediaDevices.getUserMedia({
					audio: {
						echoCancellation: true,
						noiseSuppression: true,
						autoGainControl: true,
					},
				});
			} catch (cause) {
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
				callIsFull(liveRef.current, me)
			) {
				for (const track of stream.getTracks()) track.stop();
				return false;
			}
			let context: AudioContext | null = null;
			try {
				context = new AudioContext();
				void context.resume().catch(() => {});
			} catch (cause) {
				console.warn("Audio analysis unavailable", cause);
			}
			const session: Session = {
				callId,
				conversationId,
				members: peopleRef.current,
				awareness: target.awareness,
				peer,
				stream,
				context,
				localMeter: makeMeter(context, stream, me),
				outputs: new Map(),
				poll: null,
				joinedAt: Date.now(),
				muted: false,
				listening: false,
				listener: null,
				transcripts: null,
				video: null,
			};
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
			publish(session);
			engineRef.current?.setLocalStream(stream);
			setJoined(true);
			return true;
		},
		[conversationId, me],
	);

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
		session.listener?.setPaused(session.muted);
		publish(session);
	}, []);

	const transcribe = useCallback(
		async (session: Session, utterance: SpokenUtterance) => {
			const { listen: options, audio: audioState } = listenRef.current;
			if (
				!options?.stt ||
				!audioState ||
				optionsRef.current?.persistCallEvents === false
			)
				return;
			const wav = encodeWav(utterance.samples);
			const result = await audioState.transcribe({
				bit_id: options.stt.bitId,
				audio_base64: bytesToBase64(wav),
				mime: "audio/wav",
				local: options.stt.route === "local",
			});
			const text = result.text.trim();
			if (!text || sessionRef.current !== session || !session.listening) return;
			const origin =
				startTimesRef.current.get(session.callId) ?? session.joinedAt;
			await sendEntry({
				conversationId: session.conversationId,
				request: {
					message_id: newInboxMessageId(),
					kind: "transcript",
					envelope: {
						v: 1,
						enc: "none",
						body: {
							call: session.callId,
							text,
							offset_ms: Math.max(0, Math.round(utterance.spokenAt - origin)),
							duration_ms: utterance.durationMs,
							mentions: options.mentionsIn(text),
						},
					},
				},
			});
		},
		[sendEntry],
	);

	const setListening = useCallback(
		(on: boolean) => {
			const session = sessionRef.current;
			if (!session) return;
			if (!on) {
				stopListening(session);
				publish(session);
				return;
			}
			if (
				session.listening ||
				!listenRef.current.listen?.stt ||
				optionsRef.current?.persistCallEvents === false
			)
				return;
			session.listening = true;
			session.transcripts = createSerialQueue<SpokenUtterance>((utterance) =>
				transcribe(session, utterance),
			);
			publish(session);
			startCallListener(session.stream, (utterance) =>
				session.transcripts?.push({
					...utterance,
					spokenAt: Date.now() - END_SILENCE_MS - utterance.durationMs,
				}),
			)
				.then((listener) => {
					if (sessionRef.current !== session || !session.listening) {
						listener.stop();
						return;
					}
					listener.setPaused(session.muted);
					session.listener = listener;
				})
				.catch((cause) => {
					console.warn("Listening to the microphone failed", cause);
					stopListening(session);
					publish(session);
				});
		},
		[transcribe],
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
			canListen: Boolean(
				listen?.stt && audio && options?.persistCallEvents !== false,
			),
			botsPresent: listen?.hasBots ?? false,
			setListening,
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
			listen?.stt,
			listen?.hasBots,
			audio,
			setListening,
			error,
			video,
			videos,
			toggleCamera,
			toggleScreenShare,
			options?.persistCallEvents,
		],
	);
}
