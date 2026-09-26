/**
 * Where a run spends its time before it starts.
 *
 * A trace opened around a page load collects every step timed while it is open — including
 * steps deep in backend states that know nothing about the page — and prints one report when
 * it finishes. Steps are only recorded while a trace is open, so runs nobody traces pay nothing.
 * The last reports stay on `globalThis.__flowLikeRunTimings` for `copy(__flowLikeRunTimings)`;
 * `__flowLikeRunTimingsOpen()` prints traces that have not finished, including the steps they
 * are still waiting on — the only view of a load that hangs.
 * Depth comes from time containment alone, so steps that run in parallel may print nested.
 */

export interface INativePreambleStep {
	readonly step: string;
	readonly ms: number;
}

/** The native `execute_event` breakdown the desktop sends on `run_initiated`. */
export interface INativePreamble {
	readonly totalMs: number;
	readonly steps: readonly INativePreambleStep[];
}

export interface IRunTimingReportStep {
	readonly name: string;
	readonly offsetMs: number;
	/** For a pending step, how long it has been waiting so far. */
	readonly ms: number;
	/** How many recorded steps enclose this one. */
	readonly depth: number;
	readonly pending?: boolean;
}

export interface IRunTimingReportMark {
	readonly name: string;
	readonly atMs: number;
}

export interface IRunTimingReport {
	readonly label: string;
	readonly totalMs: number;
	readonly marks: readonly IRunTimingReportMark[];
	readonly steps: readonly IRunTimingReportStep[];
	readonly native?: INativePreamble;
	readonly open?: boolean;
}

export interface IRunTrace {
	/** `runId` attaches the native breakdown already recorded for that run. */
	mark(name: string, runId?: string): void;
	/** Logs and returns the report; later calls return `undefined`. */
	finish(): IRunTimingReport | undefined;
}

interface IRecordedStep {
	readonly name: string;
	/**
	 * Breaks start ties, which a coarse clock makes routine: `timeRunStep` takes it when the
	 * step starts, `recordRunStep` when it is called.
	 */
	readonly seq: number;
	readonly start: number;
	readonly end: number;
	readonly pending?: boolean;
}

interface IRecordedMark {
	readonly name: string;
	readonly at: number;
}

interface IOpenTrace {
	readonly label: string;
	readonly start: number;
	readonly marks: IRecordedMark[];
	native?: INativePreamble;
}

const MAX_STEPS = 2000;
const MAX_NATIVE_PREAMBLES = 32;
const MAX_REPORTS = 10;
/** A trace whose owner unmounted mid-run must not keep every later step recording. */
const TRACE_TTL_MS = 5 * 60_000;
const REPORTS_GLOBAL = "__flowLikeRunTimings";
const OPEN_REPORTS_GLOBAL = "__flowLikeRunTimingsOpen";

const steps: IRecordedStep[] = [];
const inFlight = new Map<
	number,
	{ readonly name: string; readonly start: number }
>();
const openTraces = new Map<number, IOpenTrace>();
const nativePreambles = new Map<string, INativePreamble>();
let nextTraceId = 0;
let nextStepSeq = 0;

export function runTimingNow(): number {
	try {
		if (
			typeof performance !== "undefined" &&
			typeof performance.now === "function"
		)
			return performance.now();
	} catch {
		// Fall through to the wall clock below.
	}
	return Date.now();
}

function isRecording(now: number): boolean {
	for (const [id, trace] of openTraces) {
		if (now - trace.start > TRACE_TTL_MS) openTraces.delete(id);
	}
	return openTraces.size > 0;
}

function pushStep(step: IRecordedStep): void {
	if (!isRecording(step.end)) return;
	steps.push(step);
	if (steps.length > MAX_STEPS) steps.splice(0, steps.length - MAX_STEPS);
}

export function recordRunStep(
	name: string,
	start: number,
	end: number = runTimingNow(),
): void {
	pushStep({ name, seq: nextStepSeq++, start, end });
}

export async function timeRunStep<T>(
	name: string,
	run: () => Promise<T>,
): Promise<T> {
	if (openTraces.size === 0) return run();
	const seq = nextStepSeq++;
	const start = runTimingNow();
	inFlight.set(seq, { name, start });
	try {
		return await run();
	} finally {
		inFlight.delete(seq);
		pushStep({ name, seq, start, end: runTimingNow() });
	}
}

function parseNativePreamble(raw: unknown): INativePreamble | undefined {
	if (!raw || typeof raw !== "object") return undefined;
	const { total_ms: totalMs, steps: rawSteps } = raw as {
		total_ms?: unknown;
		steps?: unknown;
	};
	if (typeof totalMs !== "number" || !Array.isArray(rawSteps)) return undefined;
	const parsed: INativePreambleStep[] = [];
	for (const entry of rawSteps) {
		const { step, ms } = (entry ?? {}) as { step?: unknown; ms?: unknown };
		if (typeof step === "string" && typeof ms === "number")
			parsed.push({ step, ms });
	}
	return { totalMs, steps: parsed };
}

export function recordNativePreamble(runId: string, raw: unknown): void {
	if (!isRecording(runTimingNow())) return;
	const preamble = parseNativePreamble(raw);
	if (!preamble) return;
	nativePreambles.set(runId, preamble);
	if (nativePreambles.size > MAX_NATIVE_PREAMBLES) {
		const oldest = nativePreambles.keys().next().value;
		if (oldest !== undefined) nativePreambles.delete(oldest);
	}
}

const round = (ms: number) => Math.round(ms * 10) / 10;

function encloses(enclosingEnd: number, step: IRecordedStep): boolean {
	return step.start < enclosingEnd && step.end <= enclosingEnd;
}

function reportSteps(start: number, end: number): IRunTimingReportStep[] {
	const pending: IRecordedStep[] = [...inFlight].map(([seq, step]) => ({
		...step,
		seq,
		end,
		pending: true,
	}));
	const traced = [...steps, ...pending]
		.filter((step) => step.start >= start && step.start <= end)
		.sort((a, b) => a.start - b.start || a.seq - b.seq);
	const enclosingEnds: number[] = [];
	return traced.map((step) => {
		while (
			enclosingEnds.length > 0 &&
			!encloses(enclosingEnds[enclosingEnds.length - 1], step)
		)
			enclosingEnds.pop();
		const depth = enclosingEnds.length;
		enclosingEnds.push(step.end);
		return {
			name: step.name,
			offsetMs: round(step.start - start),
			ms: round(step.end - step.start),
			depth,
			...(step.pending ? { pending: true } : {}),
		};
	});
}

function buildReport(
	trace: IOpenTrace,
	end: number,
	open: boolean,
): IRunTimingReport {
	const { native } = trace;
	return {
		label: trace.label,
		totalMs: round(end - trace.start),
		marks: trace.marks.map((mark) => ({
			name: mark.name,
			atMs: round(mark.at - trace.start),
		})),
		steps: reportSteps(trace.start, end),
		...(native ? { native } : {}),
		...(open ? { open: true } : {}),
	};
}

function pruneSteps(): void {
	if (openTraces.size === 0) {
		steps.length = 0;
		nativePreambles.clear();
		return;
	}
	const oldestOpen = Math.min(
		...[...openTraces.values()].map((trace) => trace.start),
	);
	const keepFrom = steps.findIndex((step) => step.start >= oldestOpen);
	steps.splice(0, keepFrom === -1 ? steps.length : keepFrom);
}

const pad = (value: string, width: number) => value.padStart(width, " ");

export function formatRunTimingReport(report: IRunTimingReport): string {
	const marks = report.marks
		.map((mark) => `${mark.name} +${Math.round(mark.atMs)} ms`)
		.join(", ");
	const lines = [
		`[run-timing] ${report.label}${report.open ? " (still open)" : ""}: ${Math.round(report.totalMs)} ms${marks ? ` (${marks})` : ""}`,
	];
	for (const step of report.steps) {
		lines.push(
			`${pad(`+${Math.round(step.offsetMs)} ms`, 10)} ${pad(`${Math.round(step.ms)} ms`, 9)}  ${"  ".repeat(step.depth)}${step.name}${step.pending ? " (pending)" : ""}`,
		);
	}
	if (report.native) {
		const native = report.native.steps
			.map((step) => `${step.step} ${step.ms}`)
			.join(" · ");
		lines.push(
			`native execute_event ${Math.round(report.native.totalMs)} ms: ${native}`,
		);
	}
	return lines.join("\n");
}

export function openRunTimingReports(): IRunTimingReport[] {
	const now = runTimingNow();
	isRecording(now);
	return [...openTraces.values()].map((trace) => buildReport(trace, now, true));
}

function installOpenReportsHandle(): void {
	try {
		const holder = globalThis as { [OPEN_REPORTS_GLOBAL]?: () => string };
		if (holder[OPEN_REPORTS_GLOBAL]) return;
		holder[OPEN_REPORTS_GLOBAL] = () => {
			const text =
				openRunTimingReports().map(formatRunTimingReport).join("\n\n") ||
				"[run-timing] no open traces";
			console.info(text);
			return text;
		};
	} catch {
		// A frozen global only costs the console handle.
	}
}

function publish(report: IRunTimingReport): void {
	try {
		const holder = globalThis as { [REPORTS_GLOBAL]?: IRunTimingReport[] };
		const reports = holder[REPORTS_GLOBAL] ?? [];
		reports.push(report);
		if (reports.length > MAX_REPORTS)
			reports.splice(0, reports.length - MAX_REPORTS);
		holder[REPORTS_GLOBAL] = reports;
	} catch {
		// A frozen global only costs the copy handle; the log below still lands.
	}
	console.info(formatRunTimingReport(report));
}

export function startRunTrace(label: string): IRunTrace {
	installOpenReportsHandle();
	const id = nextTraceId++;
	const trace: IOpenTrace = { label, start: runTimingNow(), marks: [] };
	openTraces.set(id, trace);
	let finished = false;

	return {
		mark(name, runId) {
			if (finished) return;
			trace.marks.push({ name, at: runTimingNow() });
			if (runId === undefined) return;
			const native = nativePreambles.get(runId);
			if (!native) return;
			trace.native = native;
			nativePreambles.delete(runId);
		},
		finish() {
			if (finished) return undefined;
			finished = true;
			const end = runTimingNow();
			openTraces.delete(id);
			const report = buildReport(trace, end, false);
			pruneSteps();
			publish(report);
			return report;
		},
	};
}

export function resetRunTiming(): void {
	steps.length = 0;
	inFlight.clear();
	openTraces.clear();
	nativePreambles.clear();
}
