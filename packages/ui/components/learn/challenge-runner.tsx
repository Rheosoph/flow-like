"use client";
import { Check, Loader2, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type {
	AttemptResult,
	BoardRiddlePayload,
	Challenge,
	ChallengeAttempt,
	ChoiceChallengePayload,
	ExecuteNodeChallengePayload,
} from "../../lib/learn/types";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "../ui/card";
import { Input } from "../ui/input";

interface ChallengeRunnerProps {
	readonly challenge: Challenge;
	readonly onSubmit: (submission: unknown) => Promise<AttemptResult>;
	readonly attempts?: ReadonlyArray<ChallengeAttempt>;
	readonly disabled?: boolean;
	/** Provide a board snapshot for board_riddle / execute_node challenges. */
	readonly buildBoardSubmission?: () => unknown | Promise<unknown>;
}

export function ChallengeRunner({
	challenge,
	onSubmit,
	attempts = [],
	disabled,
	buildBoardSubmission,
}: ChallengeRunnerProps) {
	const [submitting, setSubmitting] = useState(false);
	const [result, setResult] = useState<AttemptResult | null>(null);
	const [runId, setRunId] = useState("");

	if (
		challenge.kind === "SINGLE_CHOICE" ||
		challenge.kind === "MULTIPLE_CHOICE"
	) {
		return (
			<ChoiceChallenge
				challenge={challenge}
				disabled={disabled || submitting}
				submitting={submitting}
				result={result}
				attempts={attempts}
				onSubmit={async (selected) => {
					setSubmitting(true);
					try {
						const r = await onSubmit({ selected });
						setResult(r);
					} finally {
						setSubmitting(false);
					}
				}}
			/>
		);
	}

	if (challenge.kind === "BOARD_RIDDLE" || challenge.kind === "EXECUTE_NODE") {
		return (
			<Card>
				<CardHeader>
					<CardTitle className="flex items-center gap-2 text-base">
						{challenge.prompt}
						<Badge variant="outline" className="ml-auto">
							{challenge.kind === "BOARD_RIDDLE"
								? "Board riddle"
								: "Execute node"}
						</Badge>
					</CardTitle>
				</CardHeader>
				<CardContent className="space-y-3">
					<p className="text-sm text-muted-foreground">
						Edit the board on the right, then check your work.
					</p>
					<RiddleHints challenge={challenge} />
					<Button
						disabled={disabled || submitting || !buildBoardSubmission}
						onClick={async () => {
							if (!buildBoardSubmission) return;
							setSubmitting(true);
							try {
								const submission = await buildBoardSubmission();
								const submissionRecord = asRecord(submission) ?? {};
								const embeddedRunId =
									typeof submissionRecord.runId === "string"
										? submissionRecord.runId
										: typeof submissionRecord.run_id === "string"
											? submissionRecord.run_id
											: "";
								const proofRunId = runId.trim() || embeddedRunId.trim();
								const attemptSubmission =
									challenge.kind === "EXECUTE_NODE"
										? {
												...submissionRecord,
												...(proofRunId ? { runId: proofRunId } : {}),
											}
										: submission;
								const r = await onSubmit(attemptSubmission);
								setResult(r);
							} finally {
								setSubmitting(false);
							}
						}}
					>
						{submitting ? (
							<Loader2 className="h-4 w-4 mr-2 animate-spin" />
						) : null}
						Check my board
					</Button>
					{challenge.kind === "EXECUTE_NODE" ? (
						<Input
							value={runId}
							onChange={(event) => setRunId(event.target.value)}
							placeholder="Completed run ID"
							aria-label="Completed run ID"
						/>
					) : null}
					<ResultBanner result={result} />
				</CardContent>
			</Card>
		);
	}

	return null;
}

function ChoiceChallenge({
	challenge,
	onSubmit,
	disabled,
	submitting,
	result,
	attempts,
}: {
	readonly challenge: Challenge;
	readonly onSubmit: (selected: string[]) => Promise<void> | void;
	readonly disabled?: boolean;
	readonly submitting?: boolean;
	readonly result: AttemptResult | null;
	readonly attempts: ReadonlyArray<ChallengeAttempt>;
}) {
	const payload = challenge.payload as ChoiceChallengePayload;
	const isMulti = challenge.kind === "MULTIPLE_CHOICE";
	const [selected, setSelected] = useState<string[]>([]);
	const latestStoredSelected = useMemo(
		() => readSelectedSubmission(attempts[0]?.submission),
		[attempts],
	);

	useEffect(() => {
		setSelected(latestStoredSelected);
	}, [latestStoredSelected]);

	const toggle = (id: string) => {
		if (isMulti) {
			setSelected((s) =>
				s.includes(id) ? s.filter((x) => x !== id) : [...s, id],
			);
		} else {
			setSelected([id]);
		}
	};

	const options = useMemo(() => payload?.options ?? [], [payload]);

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base">{challenge.prompt}</CardTitle>
			</CardHeader>
			<CardContent className="space-y-3">
				<div className="flex flex-col gap-2">
					{options.map((opt) => {
						const isSelected = selected.includes(opt.id);
						return (
							<button
								key={opt.id}
								type="button"
								disabled={disabled}
								onClick={() => toggle(opt.id)}
								className={`text-left rounded-lg border p-3 transition-colors hover:bg-accent ${
									isSelected ? "border-primary bg-primary/5" : "border-border"
								}`}
							>
								<span className="text-sm">{opt.label}</span>
							</button>
						);
					})}
				</div>
				<div className="flex items-center gap-3">
					<Button
						disabled={disabled || selected.length === 0}
						onClick={() => onSubmit(selected)}
					>
						{submitting ? (
							<Loader2 className="h-4 w-4 mr-2 animate-spin" />
						) : null}
						Submit
					</Button>
					<Badge variant="outline">{challenge.points} pts</Badge>
				</div>
				<ResultBanner result={result} />
			</CardContent>
		</Card>
	);
}

function asRecord(value: unknown): Record<string, unknown> | null {
	if (!value || typeof value !== "object" || Array.isArray(value)) return null;
	return value as Record<string, unknown>;
}

function readSelectedSubmission(submission: unknown): string[] {
	const data = asRecord(submission);
	return Array.isArray(data?.selected)
		? data.selected.filter(
				(value): value is string => typeof value === "string",
			)
		: [];
}

function RiddleHints({ challenge }: { readonly challenge: Challenge }) {
	if (challenge.kind === "BOARD_RIDDLE") {
		const payload = challenge.payload as BoardRiddlePayload;
		return (
			<ul className="text-xs text-muted-foreground space-y-0.5 list-disc pl-4">
				{(payload.predicates ?? []).map((p, i) => (
					<li key={i}>
						<code className="text-[11px]">{p.op}</code>:{" "}
						{JSON.stringify(p.args)}
					</li>
				))}
			</ul>
		);
	}
	if (challenge.kind === "EXECUTE_NODE") {
		const payload = challenge.payload as ExecuteNodeChallengePayload;
		const packages =
			payload.requiredPackages ??
			payload.required_packages ??
			payload.packages ??
			[];
		return (
			<p className="text-xs text-muted-foreground">
				Run node <code>{payload.nodeId}</code> on board{" "}
				<code>{payload.boardId}</code>. Required package proof:{" "}
				{packages.length ? packages.join(", ") : "not configured"}.
			</p>
		);
	}
	return null;
}

function ResultBanner({ result }: { readonly result: AttemptResult | null }) {
	if (!result) return null;
	if (result.is_correct) {
		return (
			<div className="rounded-md border border-green-500/30 bg-green-500/10 p-3 text-sm text-green-700 dark:text-green-400 flex items-center gap-2">
				<Check className="h-4 w-4" />
				<span>
					{result.points_awarded > 0
						? `Correct! +${result.points_awarded} pts.`
						: "Correct. Already scored."}{" "}
					{result.explanation ? <em>{result.explanation}</em> : null}
				</span>
			</div>
		);
	}
	return (
		<div className="rounded-md border border-destructive/30 bg-destructive/10 p-3 text-sm text-destructive flex items-center gap-2">
			<X className="h-4 w-4" />
			<span>Not quite. {result.explanation ?? "Try again."}</span>
		</div>
	);
}
