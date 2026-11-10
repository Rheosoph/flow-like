import { MAX_QUEUED_UTTERANCES, type Utterance } from "./call-listening";

interface Work<T> {
	utterance: T;
	generation: number;
	final: boolean;
}

interface Settled {
	offsetMs: number;
	length: number;
	text: Promise<string>;
}

/**
 * Runs speech model calls in two independent lanes, one final and one replaceable preview at a time,
 * so a final never waits behind a preview. A settled preview is reused by the final it predicted.
 */
export function createCallTranscription<T extends Utterance>({
	transcribe,
	onPartial,
	onFinal,
	onError,
	limit = MAX_QUEUED_UTTERANCES,
}: {
	transcribe: (utterance: T) => Promise<string>;
	onPartial: (text: string) => void;
	onFinal: (text: string, utterance: T) => Promise<void>;
	onError: (error: unknown) => void;
	limit?: number;
}) {
	const maximum =
		Number.isSafeInteger(limit) && limit > 0 ? limit : MAX_QUEUED_UTTERANCES;
	const finals: Work<T>[] = [];
	let preview: Work<T> | null = null;
	let settled: Settled | null = null;
	let draft: { offsetMs: number; generation: number } | null = null;
	let draftText = "";
	let generation = 0;
	let latestOffset = Number.NEGATIVE_INFINITY;
	let finalOffset = Number.NEGATIVE_INFINITY;
	let finalBusy = false;
	let previewBusy = false;
	let closed = false;

	const show = (text: string) => {
		if (text === draftText) return;
		draftText = text;
		onPartial(text);
	};
	const current = (work: Work<T>) =>
		!closed &&
		draft?.offsetMs === work.utterance.offsetMs &&
		draft.generation === work.generation;
	const clear = (work: Work<T>) => {
		if (!current(work)) return;
		draft = null;
		show("");
	};
	const sameUtterance = (memo: Settled | null, utterance: T): memo is Settled =>
		memo?.offsetMs === utterance.offsetMs &&
		memo.length === utterance.samples.length;

	const textOf = async (work: Work<T>): Promise<string> => {
		if (work.final && sameUtterance(settled, work.utterance)) {
			const reused = settled;
			try {
				return (await reused.text).trim();
			} catch {
				// A failed preview must not fail the final; transcribe it again.
			}
		}
		return (await transcribe(work.utterance)).trim();
	};

	const drainFinals = async () => {
		if (finalBusy || closed) return;
		finalBusy = true;
		try {
			while (!closed) {
				const work = finals.shift();
				if (!work) break;
				try {
					const text = await textOf(work);
					if (closed) return;
					if (text) await onFinal(text, work.utterance);
				} catch (error) {
					if (!closed) onError(error);
				} finally {
					clear(work);
				}
			}
		} finally {
			finalBusy = false;
		}
	};

	const drainPreviews = async () => {
		if (previewBusy || closed) return;
		previewBusy = true;
		try {
			while (!closed) {
				const work = preview;
				if (!work) break;
				preview = null;
				const request = transcribe(work.utterance).then((text) => text.trim());
				if (work.utterance.settled) {
					settled = {
						offsetMs: work.utterance.offsetMs,
						length: work.utterance.samples.length,
						text: request,
					};
					request.catch(() => {});
				}
				try {
					const text = await request;
					if (closed) return;
					if (text && current(work)) show(text);
				} catch (error) {
					if (!closed && current(work)) onError(error);
				}
			}
		} finally {
			previewBusy = false;
		}
	};

	return {
		partial(utterance: T) {
			if (
				closed ||
				utterance.offsetMs <= finalOffset ||
				utterance.offsetMs < latestOffset
			)
				return;
			latestOffset = utterance.offsetMs;
			if (draft?.offsetMs !== utterance.offsetMs) {
				draft = { offsetMs: utterance.offsetMs, generation };
				show("");
			}
			if (!utterance.settled && settled?.offsetMs === utterance.offsetMs)
				settled = null;
			preview = { utterance, generation, final: false };
			void drainPreviews();
		},
		final(utterance: T) {
			if (closed || utterance.offsetMs <= finalOffset) return;
			finalOffset = utterance.offsetMs;
			if (preview && preview.utterance.offsetMs <= utterance.offsetMs)
				preview = null;
			finals.push({ utterance, generation, final: true });
			while (finals.length > maximum) {
				const dropped = finals.shift();
				if (dropped) clear(dropped);
			}
			void drainFinals();
		},
		/** Transcribes audio outside both lanes, for a one-off check such as confirming a barge-in. */
		async probe(utterance: T): Promise<string> {
			if (closed) return "";
			return (await transcribe(utterance)).trim();
		},
		resetPartial() {
			if (closed) return;
			generation++;
			preview = null;
			settled = null;
			draft = null;
			show("");
		},
		close() {
			if (closed) return;
			closed = true;
			finals.length = 0;
			preview = null;
			settled = null;
			draft = null;
			show("");
		},
	};
}
