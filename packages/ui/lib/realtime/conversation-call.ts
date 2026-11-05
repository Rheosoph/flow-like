/**
 * Calls over a conversation's y-webrtc room. Media rides the room's existing peer
 * connections (simple-peer renegotiates through the room's own signaling); who is in the call
 * rides awareness. Peers are untrusted, so everything read from awareness is sanitized and
 * only active user members count.
 */

/** A mesh carries a call up to this many people; beyond that an SFU is needed. */
export const MAX_CALL_PARTICIPANTS = 8;
export const CALL_FIELD = "call";

const CALL_ID_PATTERN = /^[A-Za-z0-9_-]{1,80}$/;
const PEER_ID_PATTERN = /^[A-Za-z0-9_-]{1,80}$/;

export interface CallPresence {
	id: string;
	joined: boolean;
	muted: boolean;
	peer: string;
	listening: boolean;
	camera?: string;
	screen?: string;
	voiceBots?: readonly string[];
	speakingBot?: string;
}

export interface CallParticipant {
	sub: string;
	muted: boolean;
	listening: boolean;
	peer: string;
	clientId: number;
	local: boolean;
	camera?: string;
	screen?: string;
	voiceBots?: readonly string[];
	speakingBot?: string;
}

export interface LiveCall {
	callId: string;
	participants: readonly CallParticipant[];
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function streamId(value: unknown): value is string {
	return typeof value === "string" && /^[A-Za-z0-9_-]{1,128}$/.test(value);
}

export function sanitizeCallPresence(value: unknown): CallPresence | null {
	if (!isRecord(value)) return null;
	const { id, joined, muted, peer, listening } = value;
	if (typeof id !== "string" || !CALL_ID_PATTERN.test(id)) return null;
	if (typeof peer !== "string" || !PEER_ID_PATTERN.test(peer)) return null;
	const voiceBots = Array.isArray(value.voiceBots)
		? [
				...new Set(
					value.voiceBots.filter(
						(id): id is string =>
							typeof id === "string" && /^[A-Za-z0-9:_-]{1,160}$/.test(id),
					),
				),
			]
				.slice(0, 64)
				.sort()
		: [];
	return {
		id,
		joined: joined === true,
		muted: muted === true,
		peer,
		listening: listening === true,
		...(streamId(value.camera) ? { camera: value.camera as string } : {}),
		...(streamId(value.screen) ? { screen: value.screen as string } : {}),
		...(voiceBots.length ? { voiceBots } : {}),
		...(typeof value.speakingBot === "string" &&
		voiceBots.includes(value.speakingBot)
			? { speakingBot: value.speakingBot }
			: {}),
	};
}

interface Candidate extends CallParticipant {
	callId: string;
}

function candidatesOf(
	states: ReadonlyMap<number, unknown>,
	localClientId: number,
	members: ReadonlySet<string>,
): Candidate[] {
	const found: Candidate[] = [];
	for (const [clientId, state] of states) {
		if (!isRecord(state)) continue;
		const sub = state.sub;
		if (typeof sub !== "string" || !members.has(sub)) continue;
		const presence = sanitizeCallPresence(state[CALL_FIELD]);
		if (!presence?.joined) continue;
		found.push({
			callId: presence.id,
			sub,
			muted: presence.muted,
			listening: presence.listening,
			peer: presence.peer,
			clientId,
			local: clientId === localClientId,
			camera: presence.camera,
			screen: presence.screen,
			voiceBots: presence.voiceBots,
			speakingBot: presence.speakingBot,
		});
	}
	return found.sort((a, b) => a.clientId - b.clientId);
}

function admit(candidates: readonly Candidate[]): CallParticipant[] {
	const seen = new Set<string>();
	const admitted: CallParticipant[] = [];
	for (const { callId: _callId, ...participant } of candidates) {
		if (seen.has(participant.sub)) continue;
		if (admitted.length >= MAX_CALL_PARTICIPANTS) break;
		seen.add(participant.sub);
		admitted.push(participant);
	}
	return admitted;
}

/**
 * The call people are in, from awareness. With several ids (a call that was not wound up), the one
 * with the most people wins and the smaller id breaks a tie, so every peer picks the same one.
 */
export function deriveLiveCall(
	states: ReadonlyMap<number, unknown>,
	localClientId: number,
	members: ReadonlySet<string>,
): LiveCall | null {
	const candidates = candidatesOf(states, localClientId, members);
	if (candidates.length === 0) return null;
	const sizes = new Map<string, number>();
	for (const candidate of candidates) {
		sizes.set(candidate.callId, (sizes.get(candidate.callId) ?? 0) + 1);
	}
	let callId = "";
	let best = 0;
	for (const [id, size] of sizes) {
		if (size > best || (size === best && id < callId)) {
			callId = id;
			best = size;
		}
	}
	return {
		callId,
		participants: admit(candidates.filter((c) => c.callId === callId)),
	};
}

export function callLiveKey(live: LiveCall | null): string {
	if (!live) return "";
	return `${live.callId}|${live.participants
		.map(
			(p) =>
				`${p.sub}:${p.peer}:${p.muted ? 1 : 0}${p.listening ? 1 : 0}:${p.camera ?? ""}:${p.screen ?? ""}:${p.voiceBots?.join(";") ?? ""}:${p.speakingBot ?? ""}`,
		)
		.join(",")}`;
}

export function callIsFull(live: LiveCall | null, me: string): boolean {
	return (
		live !== null &&
		live.participants.length >= MAX_CALL_PARTICIPANTS &&
		!live.participants.some((p) => p.sub === me)
	);
}

/** Peer id to person. A peer id claimed by two people maps to nobody, so a stream cannot be spoofed onto someone else. */
export function peerSubs(
	participants: readonly CallParticipant[],
): Map<string, string> {
	const counts = new Map<string, number>();
	for (const p of participants)
		counts.set(p.peer, (counts.get(p.peer) ?? 0) + 1);
	const map = new Map<string, string>();
	for (const p of participants) {
		if (counts.get(p.peer) === 1) map.set(p.peer, p.sub);
	}
	return map;
}

/** One capable participant supplies a bot's voice to the call, including listeners without that model. */
export function botVoiceSource(
	live: LiveCall | null,
	botId: string,
): CallParticipant | null {
	if (!live) return null;
	const peers = peerSubs(live.participants);
	return (
		live.participants
			.filter((p) => peers.has(p.peer) && p.voiceBots?.includes(botId))
			.sort((a, b) => a.clientId - b.clientId)[0] ?? null
	);
}

/** "12 min" pieces for a duration: whole hours and minutes, with under a minute as 0 and 0. */
export function splitCallDuration(ms: number): {
	hours: number;
	minutes: number;
} {
	const total = Math.max(
		0,
		Math.floor((Number.isFinite(ms) ? ms : 0) / 60_000),
	);
	return { hours: Math.floor(total / 60), minutes: total % 60 };
}

export function formatClock(ms: number): string {
	const total = Math.max(0, Math.floor((Number.isFinite(ms) ? ms : 0) / 1000));
	const pad = (n: number) => String(n).padStart(2, "0");
	const hours = Math.floor(total / 3600);
	const minutes = Math.floor((total % 3600) / 60);
	const seconds = total % 60;
	return hours > 0
		? `${hours}:${pad(minutes)}:${pad(seconds)}`
		: `${minutes}:${pad(seconds)}`;
}

/** Turns a microphone level into a steady speaking flag: it rises at once and falls after a short hold. */
export function createSpeakingDetector(options?: {
	threshold?: number;
	holdMs?: number;
}) {
	const threshold = options?.threshold ?? 0.02;
	const holdMs = options?.holdMs ?? 300;
	let lastLoud = Number.NEGATIVE_INFINITY;
	return {
		update(level: number, now: number): boolean {
			if (level >= threshold) lastLoud = now;
			return now - lastLoud <= holdMs;
		},
		reset() {
			lastLoud = Number.NEGATIVE_INFINITY;
		},
	};
}

/** Root mean square of a time-domain byte buffer (128 is silence). */
export function byteLevel(samples: ArrayLike<number>): number {
	if (samples.length === 0) return 0;
	let sum = 0;
	for (let i = 0; i < samples.length; i++) {
		const v = (samples[i] - 128) / 128;
		sum += v * v;
	}
	return Math.sqrt(sum / samples.length);
}

export interface CallPeerLike {
	destroyed?: boolean;
	addStream(stream: MediaStream): void;
	removeStream(stream: MediaStream): void;
	replaceTrack?(
		oldTrack: MediaStreamTrack,
		newTrack: MediaStreamTrack | null,
		stream: MediaStream,
	): void;
	on(event: "stream" | "close", listener: (stream?: MediaStream) => void): void;
	off(
		event: "stream" | "close",
		listener: (stream?: MediaStream) => void,
	): void;
}

export interface CallProviderLike {
	room?: {
		webrtcConns: Map<string, { peer?: unknown }>;
	} | null;
	on(event: "peers", listener: (change: unknown) => void): void;
	off(event: "peers", listener: (change: unknown) => void): void;
}

export type RemoteStreams = ReadonlyMap<string, readonly MediaStream[]>;

export interface CallEngine {
	setLocalStream(stream: MediaStream | null): void;
	/** Microphone, camera and screen remain separate streams during renegotiation. */
	setLocalStreams(streams: readonly MediaStream[]): void;
	/** Only joined call peers receive local media. null retains the legacy unrestricted mode. */
	setAllowedPeers(peers: ReadonlySet<string> | null): void;
	onRemote(listener: (streams: RemoteStreams) => void): () => void;
	dispose(): void;
}

function isPeerLike(peer: unknown): peer is CallPeerLike {
	return (
		typeof peer === "object" &&
		peer !== null &&
		typeof (peer as CallPeerLike).addStream === "function" &&
		typeof (peer as CallPeerLike).on === "function"
	);
}

interface Tracked {
	remotePeerId: string;
	local: Set<MediaStream>;
	paused: Set<MediaStream>;
	remote: Map<MediaStream, () => void>;
	onStream: (stream?: MediaStream) => void;
	onClose: () => void;
}

/** Reattaches all active streams when ICE credentials replace a peer connection. */
export function createCallEngine(provider: CallProviderLike): CallEngine {
	const tracked = new Map<CallPeerLike, Tracked>();
	const listeners = new Set<(streams: RemoteStreams) => void>();
	let local = new Set<MediaStream>();
	let allowed: ReadonlySet<string> | null = null;
	let disposed = false;

	const snapshot = (): RemoteStreams => {
		const streams = new Map<string, readonly MediaStream[]>();
		for (const entry of tracked.values()) {
			if (entry.remote.size)
				streams.set(entry.remotePeerId, [...entry.remote.keys()]);
		}
		return streams;
	};
	const emit = () => {
		const streams = snapshot();
		for (const listener of listeners) listener(streams);
	};
	const swap = (peer: CallPeerLike, entry: Tracked) => {
		const admitted = allowed === null || allowed.has(entry.remotePeerId);
		for (const stream of entry.local) {
			if (local.has(stream) && admitted) {
				if (entry.paused.has(stream)) {
					try {
						for (const track of stream.getTracks())
							peer.replaceTrack?.(track, track, stream);
						entry.paused.delete(stream);
					} catch (cause) {
						console.warn("Resuming a call stream failed", cause);
					}
				}
				continue;
			}
			if (
				local.has(stream) &&
				!admitted &&
				peer.replaceTrack &&
				!peer.destroyed
			) {
				if (!entry.paused.has(stream)) {
					try {
						for (const track of stream.getTracks())
							peer.replaceTrack(track, null, stream);
						entry.paused.add(stream);
					} catch (cause) {
						console.warn("Pausing a call stream failed", cause);
					}
				}
				continue;
			}
			try {
				if (!peer.destroyed) peer.removeStream(stream);
			} catch (cause) {
				console.warn("Removing a call stream failed", cause);
			}
			entry.local.delete(stream);
			entry.paused.delete(stream);
		}
		if (!admitted) return;
		for (const stream of local) {
			if (entry.local.has(stream) || peer.destroyed) continue;
			try {
				peer.addStream(stream);
				entry.local.add(stream);
			} catch (cause) {
				console.warn("Adding a call stream failed", cause);
			}
		}
	};
	const detach = (peer: CallPeerLike, entry: Tracked) => {
		peer.off("stream", entry.onStream);
		peer.off("close", entry.onClose);
		for (const cleanup of entry.remote.values()) cleanup();
		entry.remote.clear();
		for (const stream of entry.local) {
			try {
				if (!peer.destroyed) peer.removeStream(stream);
			} catch {}
		}
		entry.local.clear();
		tracked.delete(peer);
	};
	const track = (peerId: string, peer: CallPeerLike) => {
		const existing = tracked.get(peer);
		if (existing) {
			swap(peer, existing);
			return;
		}
		const entry: Tracked = {
			remotePeerId: peerId,
			local: new Set(),
			paused: new Set(),
			remote: new Map(),
			onStream: (stream) => {
				if (!stream || disposed || entry.remote.has(stream)) return;
				// Each peer can publish microphone, camera and screen. Ignore an unbounded stream flood.
				if (entry.remote.size >= 3) return;
				const tracks = stream.getTracks();
				const prune = () => {
					if (stream.getTracks().some((t) => t.readyState !== "ended")) {
						emit();
						return;
					}
					entry.remote.get(stream)?.();
					entry.remote.delete(stream);
					emit();
				};
				const changed = () => emit();
				stream.addEventListener?.("removetrack", prune);
				stream.addEventListener?.("addtrack", changed);
				for (const t of tracks) t.addEventListener?.("ended", prune);
				entry.remote.set(stream, () => {
					stream.removeEventListener?.("removetrack", prune);
					stream.removeEventListener?.("addtrack", changed);
					for (const t of tracks) t.removeEventListener?.("ended", prune);
				});
				emit();
			},
			onClose: () => {
				detach(peer, entry);
				emit();
			},
		};
		tracked.set(peer, entry);
		peer.on("stream", entry.onStream);
		peer.on("close", entry.onClose);
		swap(peer, entry);
	};
	const sync = () => {
		if (disposed) return;
		const conns = provider.room?.webrtcConns;
		for (const [peer, entry] of tracked) {
			if (conns?.get(entry.remotePeerId)?.peer !== peer) {
				detach(peer, entry);
				emit();
			}
		}
		for (const [id, conn] of conns ?? [])
			if (isPeerLike(conn.peer)) track(id, conn.peer);
	};
	const setLocalStreams = (streams: readonly MediaStream[]) => {
		if (disposed) return;
		local = new Set(streams);
		sync();
		for (const [peer, entry] of tracked) swap(peer, entry);
	};
	provider.on("peers", sync);
	sync();
	return {
		setLocalStream: (stream) => setLocalStreams(stream ? [stream] : []),
		setLocalStreams,
		setAllowedPeers(peers) {
			allowed = peers;
			for (const [peer, entry] of tracked) swap(peer, entry);
		},
		onRemote(listener) {
			listeners.add(listener);
			listener(snapshot());
			return () => listeners.delete(listener);
		},
		dispose() {
			if (disposed) return;
			disposed = true;
			provider.off("peers", sync);
			for (const [peer, entry] of tracked) detach(peer, entry);
			local.clear();
			listeners.clear();
		},
	};
}
