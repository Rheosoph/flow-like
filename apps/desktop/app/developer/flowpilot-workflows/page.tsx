"use client";

import { Button } from "@flow-like/flow-like-ui/components/ui/button";
import { Input } from "@flow-like/flow-like-ui/components/ui/input";
import { Label } from "@flow-like/flow-like-ui/components/ui/label";
import { useTranslation } from "@flow-like/locales";
import { Channel, invoke } from "@tauri-apps/api/core";
import Link from "next/link";
import { useEffect, useRef, useState } from "react";

type BenchmarkBackend = "codex" | "claude-code" | "github-copilot";

interface PublicBenchmarkCase {
	id: string;
	prompt: string;
}

interface BenchmarkReport {
	schema: string;
	run_id: string;
	case_id: string;
	repeat_index: number;
	status: "succeeded" | "failed" | "cancelled";
	elapsed_ms: number;
	cohort: Record<string, string>;
	usage?: {
		input_tokens?: number | null;
		output_tokens?: number | null;
		cached_input_tokens?: number | null;
	} | null;
	attempts: readonly { attempt_index: number; source_fingerprint: string }[];
	graded_candidate?: { attempt_index: number } | null;
	checks: readonly {
		check_id: string;
		status: "passed" | "failed" | "blocked";
		message?: string | null;
	}[];
	grading_errors: readonly string[];
}

interface BenchmarkRun {
	requestId: string;
	caseId: string;
	repeatIndex: number;
	report?: BenchmarkReport;
	error?: string;
}

interface BenchmarkScorecard {
	case_id: string;
	scorecard: {
		cohort: Record<string, string>;
		runs_total: number;
		repair_attempts: number;
		verified_behavior_at_1: { numerator: number; denominator: number };
		verified_behavior_within_3: { numerator: number; denominator: number };
	};
}

interface BatchConfiguration {
	backend: BenchmarkBackend;
	modelId: string;
	reasoningEffort: string | null;
	caseIds: string[];
	repeats: number;
}

function errorMessage(error: unknown): string {
	return error instanceof Error ? error.message : String(error);
}

function tokenCount(value: number | null | undefined): string {
	return value == null ? "unknown" : value.toLocaleString();
}

function elapsed(value: number): string {
	return `${(value / 1000).toFixed(1)}s`;
}

export default function FlowPilotWorkflowBenchmarksPage() {
	const { t } = useTranslation("common");
	const [cases, setCases] = useState<PublicBenchmarkCase[]>([]);
	const [loading, setLoading] = useState(true);
	const [selectedCaseIds, setSelectedCaseIds] = useState<string[]>([]);
	const [backend, setBackend] = useState<BenchmarkBackend>("codex");
	const [modelId, setModelId] = useState("");
	const [reasoningEffort, setReasoningEffort] = useState("");
	const [repeats, setRepeats] = useState(1);
	const [running, setRunning] = useState(false);
	const [stopping, setStopping] = useState(false);
	const [error, setError] = useState<string>();
	const [results, setResults] = useState<BenchmarkRun[]>([]);
	const [scorecards, setScorecards] = useState<BenchmarkScorecard[]>([]);
	const [scorecardError, setScorecardError] = useState<string>();
	useEffect(() => {
		let active = true;
		invoke<BenchmarkScorecard[]>("flowpilot_workflow_benchmark_scorecards", {
			reports: results.flatMap((result) =>
				result.report ? [result.report] : [],
			),
		})
			.then((value) => {
				if (active) {
					setScorecards(value);
					setScorecardError(undefined);
				}
			})
			.catch((failure) => {
				if (active) {
					setScorecards([]);
					setScorecardError(errorMessage(failure));
				}
			});
		return () => {
			active = false;
		};
	}, [results]);
	const [configuration, setConfiguration] = useState<BatchConfiguration>();
	const [activeCase, setActiveCase] = useState<string>();
	const [activeStartedAt, setActiveStartedAt] = useState<number>();
	const [now, setNow] = useState(Date.now());
	const [streamUpdates, setStreamUpdates] = useState(0);
	const activeRequest = useRef<string | undefined>(undefined);
	const stopRequested = useRef(false);
	const mounted = useRef(true);
	const batchRunning = useRef(false);

	useEffect(() => {
		mounted.current = true;
		let active = true;
		invoke<PublicBenchmarkCase[]>("flowpilot_workflow_benchmark_cases")
			.then((available) => {
				if (!active) return;
				setCases(available);
				setSelectedCaseIds(available.map((fixture) => fixture.id));
			})
			.catch((failure: unknown) => {
				if (active) setError(errorMessage(failure));
			})
			.finally(() => {
				if (active) setLoading(false);
			});
		return () => {
			active = false;
			mounted.current = false;
			stopRequested.current = true;
			if (activeRequest.current) {
				void invoke("cancel_copilot_chat", {
					requestId: activeRequest.current,
				}).catch(() => undefined);
			}
		};
	}, []);

	useEffect(() => {
		if (!running) return;
		const timer = setInterval(() => setNow(Date.now()), 1000);
		return () => clearInterval(timer);
	}, [running]);

	const start = async () => {
		if (batchRunning.current) return;
		const requested: BatchConfiguration = {
			backend,
			modelId: modelId.trim(),
			reasoningEffort: reasoningEffort.trim() || null,
			caseIds: cases
				.filter((fixture) => selectedCaseIds.includes(fixture.id))
				.map((fixture) => fixture.id),
			repeats,
		};
		if (
			!requested.modelId ||
			requested.caseIds.length === 0 ||
			!Number.isInteger(repeats) ||
			repeats < 1 ||
			repeats > 5
		) {
			setError(
				"Enter a model ID, select a case, and choose one to five repetitions.",
			);
			return;
		}
		batchRunning.current = true;
		stopRequested.current = false;
		setRunning(true);
		setStopping(false);
		setError(undefined);
		setResults([]);
		setConfiguration(requested);
		try {
			for (let repeatIndex = 0; repeatIndex < repeats; repeatIndex += 1) {
				for (const caseId of requested.caseIds) {
					if (stopRequested.current) return;
					const requestId = crypto.randomUUID();
					activeRequest.current = requestId;
					setActiveCase(`${caseId}, repetition ${repeatIndex + 1}`);
					setActiveStartedAt(Date.now());
					setNow(Date.now());
					setStreamUpdates(0);
					const channel = new Channel<string>();
					channel.onmessage = () => {
						if (mounted.current && activeRequest.current === requestId) {
							setStreamUpdates((count) => count + 1);
						}
					};
					const result: BenchmarkRun = { requestId, caseId, repeatIndex };
					try {
						result.report = await invoke<BenchmarkReport>(
							"flowpilot_run_workflow_benchmark",
							{
								caseId,
								backend: requested.backend,
								modelId: requested.modelId,
								reasoningEffort: requested.reasoningEffort,
								repeatIndex,
								requestId,
								channel,
							},
						);
					} catch (failure) {
						result.error = errorMessage(failure);
					} finally {
						activeRequest.current = undefined;
					}
					if (mounted.current) setResults((previous) => [...previous, result]);
				}
			}
		} finally {
			batchRunning.current = false;
			if (mounted.current) {
				setRunning(false);
				setStopping(false);
				setActiveCase(undefined);
				setActiveStartedAt(undefined);
			}
		}
	};

	const cancel = async () => {
		stopRequested.current = true;
		setStopping(true);
		const requestId = activeRequest.current;
		if (!requestId) return;
		try {
			await invoke("cancel_copilot_chat", { requestId });
		} catch (failure) {
			setError(
				`Cancellation failed: ${errorMessage(failure)}. Remaining runs will not start.`,
			);
		}
	};

	const exportResults = async () => {
		let exportedScorecards: BenchmarkScorecard[] = [];
		let exportedScorecardError: string | undefined;
		try {
			exportedScorecards = await invoke<BenchmarkScorecard[]>(
				"flowpilot_workflow_benchmark_scorecards",
				{
					reports: results.flatMap((result) =>
						result.report ? [result.report] : [],
					),
				},
			);
		} catch (failure) {
			exportedScorecardError = errorMessage(failure);
		}
		const content = {
			schema: "flowpilot.workflow-benchmark-batch/v1",
			exported_at: new Date().toISOString(),
			configuration,
			in_progress: running,
			results,
			scorecards: exportedScorecards,
			scorecard_error: exportedScorecardError,
		};
		const url = URL.createObjectURL(
			new Blob([JSON.stringify(content, null, 2)], {
				type: "application/json",
			}),
		);
		const anchor = document.createElement("a");
		anchor.href = url;
		anchor.download = `flowpilot-workflows-${Date.now()}.json`;
		anchor.click();
		setTimeout(() => URL.revokeObjectURL(url), 1000);
	};

	const passed = results.filter(
		(result) => result.report?.status === "succeeded",
	).length;
	const planned = configuration
		? configuration.caseIds.length * configuration.repeats
		: 0;

	return (
		<div className="h-full overflow-auto p-6">
			<div className="mx-auto max-w-5xl space-y-6">
				<header className="space-y-2">
					<Link
						href="/store/packages?tab=mine"
						className="text-sm text-muted-foreground underline"
					>
						{t("backToPackages", "Back to packages")}
					</Link>
					<h1 className="text-2xl font-semibold">
						Workflow behavior benchmarks
					</h1>
					<p className="text-sm text-muted-foreground">
						Generate workflows through FlowPilot and check their outputs against
						fixed host fixtures. Each run uses an isolated draft. Reports cover
						the tested transformations.
					</p>
				</header>
				{error && (
					<p
						role="alert"
						className="rounded-md border border-destructive p-3 text-sm"
					>
						{error}
					</p>
				)}
				<fieldset
					disabled={running || loading}
					className="space-y-5 rounded-lg border p-5"
				>
					<legend className="px-2 font-medium">Generation settings</legend>
					<div className="grid gap-4 sm:grid-cols-2">
						<div className="space-y-2">
							<Label htmlFor="benchmark-backend">Backend</Label>
							<select
								id="benchmark-backend"
								value={backend}
								onChange={(event) =>
									setBackend(event.target.value as BenchmarkBackend)
								}
								className="h-9 w-full rounded-md border bg-background px-3 text-sm"
							>
								<option value="codex">Codex</option>
								<option value="claude-code">Claude Code</option>
								<option value="github-copilot">GitHub Copilot</option>
							</select>
						</div>
						<div className="space-y-2">
							<Label htmlFor="benchmark-model">Model ID</Label>
							<Input
								id="benchmark-model"
								value={modelId}
								onChange={(event) => setModelId(event.target.value)}
								placeholder="Exact model ID available in this backend"
							/>
						</div>
						<div className="space-y-2">
							<Label htmlFor="benchmark-reasoning">
								Reasoning effort (optional)
							</Label>
							<Input
								id="benchmark-reasoning"
								value={reasoningEffort}
								onChange={(event) => setReasoningEffort(event.target.value)}
								placeholder="Backend default when empty"
							/>
						</div>
						<div className="space-y-2">
							<Label htmlFor="benchmark-repeats">Repetitions per case</Label>
							<Input
								id="benchmark-repeats"
								type="number"
								min={1}
								max={5}
								step={1}
								value={Number.isNaN(repeats) ? "" : repeats}
								onChange={(event) => setRepeats(event.target.valueAsNumber)}
							/>
						</div>
					</div>
					<div className="space-y-3">
						<div className="flex items-center justify-between gap-3">
							<p className="font-medium">Cases</p>
							<Button
								type="button"
								variant="outline"
								size="sm"
								onClick={() =>
									setSelectedCaseIds(cases.map((fixture) => fixture.id))
								}
							>
								Select all
							</Button>
						</div>
						{loading && (
							<p className="text-sm text-muted-foreground">
								Loading available cases...
							</p>
						)}
						{!loading && cases.length === 0 && (
							<p className="text-sm text-muted-foreground">
								No benchmark cases are available.
							</p>
						)}
						{cases.map((fixture) => (
							<label
								key={fixture.id}
								className="flex cursor-pointer items-start gap-3 rounded-md border p-3"
							>
								<input
									type="checkbox"
									className="mt-1"
									checked={selectedCaseIds.includes(fixture.id)}
									onChange={(event) =>
										setSelectedCaseIds((previous) =>
											event.target.checked
												? [...previous, fixture.id]
												: previous.filter((id) => id !== fixture.id),
										)
									}
								/>
								<span className="space-y-1">
									<span className="block text-sm font-medium">
										{fixture.id}
									</span>
									<span className="block whitespace-pre-wrap text-sm text-muted-foreground">
										{fixture.prompt}
									</span>
								</span>
							</label>
						))}
					</div>
				</fieldset>
				<div className="flex flex-wrap gap-3">
					<Button
						type="button"
						disabled={
							running ||
							loading ||
							!modelId.trim() ||
							selectedCaseIds.length === 0
						}
						onClick={start}
					>
						Run selected cases
					</Button>
					{running && (
						<Button
							type="button"
							variant="outline"
							disabled={stopping}
							onClick={cancel}
						>
							{stopping ? "Stopping..." : "Cancel batch"}
						</Button>
					)}
					<Button
						type="button"
						variant="outline"
						disabled={results.length === 0}
						onClick={exportResults}
					>
						Export JSON
					</Button>
				</div>
				{running && (
					<output className="block text-sm text-muted-foreground">
						{activeCase}: {elapsed(now - (activeStartedAt ?? now))},{" "}
						{streamUpdates.toLocaleString()} stream updates. Cases run
						sequentially.
					</output>
				)}
				{configuration && (
					<section className="space-y-3" aria-label="Benchmark results">
						<h2 className="text-lg font-semibold">
							{passed} passed, {results.length} completed of {planned} requested
						</h2>
						{scorecardError && (
							<p className="text-sm text-destructive">
								Scorecards unavailable: {scorecardError}
							</p>
						)}
						{scorecards.map(({ case_id, scorecard }) => (
							<p
								className="text-sm text-muted-foreground"
								key={`${case_id}:${JSON.stringify(scorecard.cohort)}`}
							>
								{case_id}: verified at attempt 1,{" "}
								{scorecard.verified_behavior_at_1.numerator}/
								{scorecard.verified_behavior_at_1.denominator}. Within 3,{" "}
								{scorecard.verified_behavior_within_3.numerator}/
								{scorecard.verified_behavior_within_3.denominator}.{" "}
								{scorecard.repair_attempts} repairs. Earlier uncommitted
								candidates were not graded.
							</p>
						))}
						{results.map((result) => (
							<article
								key={result.requestId}
								className="space-y-2 rounded-lg border p-4"
							>
								<h3 className="font-medium">
									{result.caseId}, repetition {result.repeatIndex + 1}:{" "}
									{result.report?.status ?? "runner error"}
								</h3>
								{result.error && (
									<p className="text-sm text-destructive">{result.error}</p>
								)}
								{result.report && (
									<>
										<p className="text-sm text-muted-foreground">
											{elapsed(result.report.elapsed_ms)} ·{" "}
											{result.report.attempts.length} source attempts ·{" "}
											{Math.max(0, result.report.attempts.length - 1)} repairs ·{" "}
											{
												result.report.checks.filter(
													(check) => check.status === "passed",
												).length
											}
											/{result.report.checks.length} checks passed
										</p>
										<p className="text-sm text-muted-foreground">
											Input tokens:{" "}
											{tokenCount(result.report.usage?.input_tokens)} · output:{" "}
											{tokenCount(result.report.usage?.output_tokens)} · cached
											input:{" "}
											{tokenCount(result.report.usage?.cached_input_tokens)}
										</p>
										{result.report.grading_errors.length > 0 && (
											<p className="whitespace-pre-wrap text-sm text-destructive">
												{result.report.grading_errors.join("\n")}
											</p>
										)}
										<details className="text-sm">
											<summary className="cursor-pointer">
												Full report and cohort identity
											</summary>
											<pre className="mt-2 max-h-96 overflow-auto rounded-md bg-muted p-3 text-xs">
												{JSON.stringify(result.report, null, 2)}
											</pre>
										</details>
									</>
								)}
							</article>
						))}
					</section>
				)}
			</div>
		</div>
	);
}
