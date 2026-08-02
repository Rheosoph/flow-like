"use client";
import { useMemo, useState } from "react";
import type { Challenge, ChallengeKind } from "../../../lib/learn/types";
import { Button } from "../../ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "../../ui/card";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";
import { Textarea } from "../../ui/textarea";

const kinds: ReadonlyArray<{
	readonly value: ChallengeKind;
	readonly label: string;
}> = [
	{ value: "SINGLE_CHOICE", label: "Single choice" },
	{ value: "MULTIPLE_CHOICE", label: "Multiple choice" },
	{ value: "BOARD_RIDDLE", label: "Board riddle" },
	{ value: "EXECUTE_NODE", label: "Execute node" },
];

export interface ChallengeFormValue {
	readonly kind: ChallengeKind;
	readonly prompt: string;
	readonly explanation: string | null;
	readonly points: number;
	readonly position: number;
	readonly payload: Record<string, unknown>;
}

interface ChallengeEditorProps {
	readonly initial?: Challenge | null;
	readonly onSubmit: (value: ChallengeFormValue) => Promise<void> | void;
	readonly onDelete?: () => Promise<void> | void;
	readonly submitting?: boolean;
}

function fromInitial(c?: Challenge | null): ChallengeFormValue {
	return {
		kind: (c?.kind as ChallengeKind) ?? "SINGLE_CHOICE",
		prompt: c?.prompt ?? "",
		explanation: c?.explanation ?? null,
		points: c?.points ?? 10,
		position: c?.position ?? 0,
		payload: (c?.payload as Record<string, unknown>) ?? {},
	};
}

export function ChallengeEditor({
	initial,
	onSubmit,
	onDelete,
	submitting,
}: ChallengeEditorProps) {
	const [value, setValue] = useState<ChallengeFormValue>(fromInitial(initial));

	function patch<K extends keyof ChallengeFormValue>(
		key: K,
		v: ChallengeFormValue[K],
	) {
		setValue((prev) => ({ ...prev, [key]: v }));
	}

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base flex items-center justify-between gap-2">
					<span>{initial ? "Edit challenge" : "New challenge"}</span>
					{onDelete && (
						<Button
							type="button"
							variant="ghost"
							size="sm"
							onClick={() => void onDelete()}
						>
							Delete
						</Button>
					)}
				</CardTitle>
			</CardHeader>
			<CardContent>
				<form
					className="space-y-4"
					onSubmit={(e) => {
						e.preventDefault();
						void onSubmit(value);
					}}
				>
					<div className="grid grid-cols-1 md:grid-cols-3 gap-3">
						<div className="space-y-2 md:col-span-1">
							<Label>Kind</Label>
							<Select
								value={value.kind}
								onValueChange={(v) => patch("kind", v as ChallengeKind)}
							>
								<SelectTrigger>
									<SelectValue />
								</SelectTrigger>
								<SelectContent>
									{kinds.map((k) => (
										<SelectItem key={k.value} value={k.value}>
											{k.label}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
						</div>
						<div className="space-y-2">
							<Label>Points</Label>
							<Input
								type="number"
								value={value.points}
								onChange={(e) => patch("points", Number(e.target.value) || 0)}
							/>
						</div>
						<div className="space-y-2">
							<Label>Position</Label>
							<Input
								type="number"
								value={value.position}
								onChange={(e) => patch("position", Number(e.target.value) || 0)}
							/>
						</div>
					</div>
					<div className="space-y-2">
						<Label>Prompt</Label>
						<Textarea
							value={value.prompt}
							onChange={(e) => patch("prompt", e.target.value)}
							rows={2}
							placeholder="Ask the user something."
							required
						/>
					</div>
					<PayloadEditor
						kind={value.kind}
						payload={value.payload}
						onChange={(p) => patch("payload", p)}
					/>
					<div className="space-y-2">
						<Label>Explanation (shown after a correct answer)</Label>
						<Textarea
							value={value.explanation ?? ""}
							onChange={(e) => patch("explanation", e.target.value || null)}
							rows={2}
						/>
					</div>
					<Button type="submit" disabled={submitting}>
						Save challenge
					</Button>
				</form>
			</CardContent>
		</Card>
	);
}

interface PayloadEditorProps {
	readonly kind: ChallengeKind;
	readonly payload: Record<string, unknown>;
	readonly onChange: (next: Record<string, unknown>) => void;
}

function PayloadEditor({ kind, payload, onChange }: PayloadEditorProps) {
	if (kind === "SINGLE_CHOICE" || kind === "MULTIPLE_CHOICE") {
		return (
			<ChoicePayloadEditor
				multi={kind === "MULTIPLE_CHOICE"}
				payload={payload}
				onChange={onChange}
			/>
		);
	}
	if (kind === "EXECUTE_NODE") {
		return <ExecuteNodePayloadEditor payload={payload} onChange={onChange} />;
	}
	if (kind === "BOARD_RIDDLE") {
		return <BoardRiddlePayloadEditor payload={payload} onChange={onChange} />;
	}
	return null;
}

function ChoicePayloadEditor({
	multi,
	payload,
	onChange,
}: {
	readonly multi: boolean;
	readonly payload: Record<string, unknown>;
	readonly onChange: (next: Record<string, unknown>) => void;
}) {
	const options = useMemo(
		() =>
			(payload.options as Array<{ id: string; label: string }> | undefined) ??
			[],
		[payload.options],
	);
	const correct = useMemo(
		() => (payload.correct as string[] | undefined) ?? [],
		[payload.correct],
	);

	const setOptions = (next: Array<{ id: string; label: string }>) =>
		onChange({ ...payload, options: next });
	const setCorrect = (next: string[]) =>
		onChange({ ...payload, correct: next });

	return (
		<div className="rounded-md border p-3 space-y-3 bg-muted/20">
			<div className="flex items-center justify-between">
				<Label>Options</Label>
				<Button
					type="button"
					size="sm"
					variant="outline"
					onClick={() =>
						setOptions([
							...options,
							{ id: `opt_${options.length + 1}`, label: "" },
						])
					}
				>
					Add option
				</Button>
			</div>
			<ul className="space-y-2">
				{options.map((opt, i) => {
					const isCorrect = correct.includes(opt.id);
					return (
						<li key={opt.id ?? i} className="flex items-center gap-2">
							<Input
								className="w-32"
								value={opt.id}
								onChange={(e) => {
									const newId = e.target.value;
									const next = options.slice();
									next[i] = { ...opt, id: newId };
									setOptions(next);
								}}
								placeholder="id"
							/>
							<Input
								value={opt.label}
								onChange={(e) => {
									const next = options.slice();
									next[i] = { ...opt, label: e.target.value };
									setOptions(next);
								}}
								placeholder="Option text"
							/>
							<Button
								type="button"
								variant={isCorrect ? "default" : "outline"}
								size="sm"
								onClick={() => {
									if (multi) {
										setCorrect(
											isCorrect
												? correct.filter((c) => c !== opt.id)
												: [...correct, opt.id],
										);
									} else {
										setCorrect(isCorrect ? [] : [opt.id]);
									}
								}}
							>
								{isCorrect ? "Correct" : "Mark correct"}
							</Button>
							<Button
								type="button"
								variant="ghost"
								size="sm"
								onClick={() => {
									setOptions(options.filter((_, j) => j !== i));
									setCorrect(correct.filter((c) => c !== opt.id));
								}}
							>
								Remove
							</Button>
						</li>
					);
				})}
			</ul>
		</div>
	);
}

function ExecuteNodePayloadEditor({
	payload,
	onChange,
}: {
	readonly payload: Record<string, unknown>;
	readonly onChange: (next: Record<string, unknown>) => void;
}) {
	const requiredPackages = JSON.stringify(
		payload.requiredPackages ??
			payload.required_packages ??
			payload.packages ??
			[],
		null,
		2,
	);
	return (
		<div className="rounded-md border p-3 space-y-3 bg-muted/20">
			<div className="grid grid-cols-1 md:grid-cols-2 gap-3">
				<div className="space-y-2">
					<Label>App alias</Label>
					<Input
						value={(payload.appAlias as string) ?? ""}
						onChange={(e) =>
							onChange({ ...payload, appAlias: e.target.value || undefined })
						}
						placeholder="starter"
					/>
				</div>
				<div className="space-y-2">
					<Label>Source board ID</Label>
					<Input
						value={(payload.boardId as string) ?? ""}
						onChange={(e) =>
							onChange({ ...payload, boardId: e.target.value || undefined })
						}
						placeholder="board id from the source/template app"
					/>
				</div>
				<div className="space-y-2">
					<Label>Source node ID</Label>
					<Input
						value={(payload.nodeId as string) ?? ""}
						onChange={(e) =>
							onChange({ ...payload, nodeId: e.target.value || undefined })
						}
					/>
				</div>
			</div>
			<div className="space-y-2">
				<Label>Required streamed packages (JSON array)</Label>
				<Textarea
					value={requiredPackages}
					rows={6}
					className="font-mono text-xs"
					placeholder={'["package-id"]'}
					onChange={(e) => {
						try {
							const next = JSON.parse(e.target.value || "[]");
							const nextPayload: Record<string, unknown> = {
								...payload,
								requiredPackages: next,
							};
							delete nextPayload.expectedOutputs;
							delete nextPayload.expected_outputs;
							onChange(nextPayload);
						} catch {
							/* keep as-is, user is mid-typing */
						}
					}}
				/>
			</div>
		</div>
	);
}

function BoardRiddlePayloadEditor({
	payload,
	onChange,
}: {
	readonly payload: Record<string, unknown>;
	readonly onChange: (next: Record<string, unknown>) => void;
}) {
	const predicates =
		(payload.predicates as
			| Array<{ op: string; args: unknown[] }>
			| undefined) ?? [];
	const setPredicates = (next: typeof predicates) =>
		onChange({ ...payload, predicates: next });

	return (
		<div className="rounded-md border p-3 space-y-3 bg-muted/20">
			<div className="grid grid-cols-1 md:grid-cols-2 gap-3">
				<div className="space-y-2">
					<Label>App alias</Label>
					<Input
						value={(payload.appAlias as string) ?? ""}
						onChange={(e) =>
							onChange({ ...payload, appAlias: e.target.value || undefined })
						}
					/>
				</div>
				<div className="space-y-2">
					<Label>Source board ID (optional)</Label>
					<Input
						value={(payload.boardId as string) ?? ""}
						onChange={(e) =>
							onChange({ ...payload, boardId: e.target.value || undefined })
						}
					/>
				</div>
			</div>
			<div className="flex items-center justify-between">
				<Label>Predicates</Label>
				<Button
					type="button"
					size="sm"
					variant="outline"
					onClick={() =>
						setPredicates([...predicates, { op: "requires_nodes", args: [] }])
					}
				>
					Add predicate
				</Button>
			</div>
			<ul className="space-y-2">
				{predicates.map((p, i) => (
					<li key={i} className="flex items-start gap-2">
						<Select
							value={p.op}
							onValueChange={(op) => {
								const next = predicates.slice();
								next[i] = { ...p, op };
								setPredicates(next);
							}}
						>
							<SelectTrigger className="w-44">
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="requires_nodes">requires_nodes</SelectItem>
								<SelectItem value="forbids_nodes">forbids_nodes</SelectItem>
								<SelectItem value="max_nodes">max_nodes</SelectItem>
								<SelectItem value="min_nodes">min_nodes</SelectItem>
								<SelectItem value="has_connection">has_connection</SelectItem>
								<SelectItem value="pin_value_equals">
									pin_value_equals
								</SelectItem>
							</SelectContent>
						</Select>
						<Input
							className="flex-1 font-mono text-xs"
							value={JSON.stringify(p.args)}
							onChange={(e) => {
								try {
									const args = JSON.parse(e.target.value);
									const next = predicates.slice();
									next[i] = { ...p, args };
									setPredicates(next);
								} catch {
									/* mid-typing */
								}
							}}
						/>
						<Button
							type="button"
							variant="ghost"
							size="sm"
							onClick={() =>
								setPredicates(predicates.filter((_, j) => j !== i))
							}
						>
							Remove
						</Button>
					</li>
				))}
			</ul>
			<p className="text-xs text-muted-foreground">
				Args are a JSON array. e.g. <code>["nodeTypeId.add"]</code> for
				requires_nodes, or <code>[5]</code> for max_nodes.
			</p>
		</div>
	);
}
