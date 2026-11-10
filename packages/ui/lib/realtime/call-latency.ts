const MAX_AGE_MS = 120_000;
const MAX_ENTRIES = 64;

interface Trace {
	startedAt: number;
	speechEndAt?: number;
	sttStartAt?: number;
	sttDoneAt?: number;
	postedAt?: number;
	replySeenAt?: number;
	blobReadyAt?: number;
	firstTextAt?: number;
	route?: CallRoute;
	turnWaitMs?: number;
	turnSource?: string;
}

export type CallRoute = "stream" | "queued";

export interface TranscriptTiming {
	speechEndAt: number;
	sttStartAt: number;
	sttDoneAt: number;
	postedAt: number;
	route?: CallRoute;
	turnWaitMs?: number;
	turnSource?: string;
}

const delta = (from?: number, to?: number) =>
	from === undefined || to === undefined ? undefined : Math.round(to - from);

/** Where the time between the end of someone's speech and the bot's first audio goes, logged once per reply. */
export function createCallLatencyTrace({
	now = Date.now,
	log = (line: Record<string, unknown>) => console.info("[call-latency]", line),
}: {
	now?: () => number;
	log?: (line: Record<string, unknown>) => void;
} = {}) {
	const traces = new Map<string, Trace>();
	const replies = new Map<string, string>();

	const prune = () => {
		const cutoff = now() - MAX_AGE_MS;
		for (const [id, trace] of traces) {
			if (trace.startedAt >= cutoff && traces.size <= MAX_ENTRIES) break;
			traces.delete(id);
		}
		for (const reply of replies.keys()) {
			if (replies.size <= MAX_ENTRIES) break;
			replies.delete(reply);
		}
	};
	const traceOf = (id: string): Trace => {
		let trace = traces.get(id);
		if (!trace) {
			trace = { startedAt: now() };
			traces.set(id, trace);
			prune();
		}
		return trace;
	};
	const finish = (replyId: string, outcome: string) => {
		const id = replies.get(replyId);
		replies.delete(replyId);
		if (id === undefined) return;
		const trace = traces.get(id);
		traces.delete(id);
		if (!trace) return;
		const at = now();
		log({
			transcript: id,
			reply: replyId,
			outcome,
			route: trace.route ?? "queued",
			turnWaitMs: trace.turnWaitMs,
			turnSource: trace.turnSource,
			speechToSttMs: delta(trace.speechEndAt, trace.sttStartAt),
			sttMs: delta(trace.sttStartAt, trace.sttDoneAt),
			postMs: delta(trace.sttDoneAt, trace.postedAt),
			firstTextMs: delta(trace.postedAt, trace.firstTextAt),
			botMs: delta(trace.postedAt, trace.replySeenAt),
			synthesisMs: delta(trace.replySeenAt, trace.blobReadyAt),
			playMs: delta(trace.blobReadyAt, at),
			totalMs: delta(trace.speechEndAt ?? trace.postedAt, at),
		});
	};

	return {
		transcriptPosted(id: string, timing: TranscriptTiming) {
			Object.assign(traceOf(id), timing);
		},
		firstText(transcriptId: string) {
			const trace = traces.get(transcriptId);
			if (trace) trace.firstTextAt ??= now();
		},
		replySeen(transcriptId: string, replyId: string) {
			const trace = traceOf(transcriptId);
			trace.replySeenAt ??= now();
			replies.set(replyId, transcriptId);
		},
		speechReady(replyId: string) {
			const id = replies.get(replyId);
			const trace = id === undefined ? undefined : traces.get(id);
			if (trace) trace.blobReadyAt ??= now();
		},
		playbackStarted(replyId: string) {
			finish(replyId, "played");
		},
		notVoiced(replyId: string, reason: string) {
			finish(replyId, reason);
		},
	};
}

export type CallLatencyTrace = ReturnType<typeof createCallLatencyTrace>;

export const callLatency = createCallLatencyTrace();
