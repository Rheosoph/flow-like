"use client";

import { useTranslation } from "@flow-like/locales";
import type { Ref } from "react";
import { cx } from "../../../settings/devices/primitives/tone";
import type { FormModel, RunEntry } from "../contracts";
import type { PaneEnv } from "./pane-env";
import type { PaneView } from "./pane-model";
import { stepsOf } from "./run-steps";
import { AnswerSection } from "./sections/answer-section";
import { FilesSection } from "./sections/files-section";
import { InputsSection } from "./sections/inputs-section";
import { NoteItem, NothingNote, PendingNote } from "./sections/notes";
import { ResultSection } from "./sections/result-section";
import { RoutesRow } from "./sections/routes-row";
import { StepsSection } from "./sections/steps-section";
import { WaitingSection } from "./sections/waiting-section";

const WORKING: ReadonlySet<RunEntry["status"]> = new Set<RunEntry["status"]>([
	"starting",
	"running",
	"streaming",
	"asking",
]);

/** A form without inputs and a run with nothing to read: its name, what it does and its state, centred (canvas `center`). */
function CenteredCard({
	pane,
	form,
	env,
}: Readonly<{
	pane: PaneView;
	form: Pick<FormModel, "name" | "description" | "routes">;
	env: PaneEnv;
}>) {
	const { t } = useTranslation("interfaces");
	const first = pane.notes[0];
	return (
		<div className="m-auto w-full max-w-120 pb-14">
			<div
				className={cx(
					"flex flex-col items-center gap-2.5 text-center",
					env.layout.split &&
						"rounded-[10px] border border-border bg-card px-8 pt-7 pb-8",
				)}
			>
				<h1 className="m-0 text-balance font-semibold text-2xl/[30px] tracking-[-0.015em]">
					{form.name}
				</h1>
				{form.description ? (
					<p className="m-0 text-balance text-[15px]/6 text-ink-2">
						{form.description}
					</p>
				) : null}
				{first?.kind === "pending" ? (
					<PendingNote slow={first.slow} centered />
				) : null}
				{first?.kind === "nothing" ? <NothingNote centered /> : null}
				{first?.kind === "stopped" ? (
					<p className="m-0 mt-3.5 w-full border-hairline border-t pt-4 font-medium text-sm/5">
						{t(
							"workbench.stage.note.stopped.centered",
							"You stopped this run before anything came back.",
						)}
					</p>
				) : null}
				{pane.offersRoutes ? (
					<RoutesRow
						routes={env.routes}
						list={form.routes}
						touch={env.touch}
						centered
					/>
				) : null}
			</div>
		</div>
	);
}

function OutputSections({
	pane,
	env,
	form,
}: Readonly<{
	pane: PaneView;
	env: PaneEnv;
	form: Pick<FormModel, "routes">;
}>) {
	const { run } = pane;
	const { output } = run;
	const steps = stepsOf(run);
	const answerLive = run.status === "streaming";
	const live = WORKING.has(run.status);
	const answer = output?.answer.trim() !== "" ? output?.answer : undefined;
	const incomplete = run.status === "failed" || run.status === "stopped";
	return (
		<>
			{steps.length > 0 ? (
				<StepsSection
					steps={steps}
					finished={run.status === "done" || run.status === "empty"}
					live={live && !(answerLive && answer)}
					touch={env.touch}
				/>
			) : null}
			{answer ? (
				<AnswerSection
					answer={answer}
					live={answerLive}
					incomplete={incomplete}
					touch={env.touch}
				/>
			) : null}
			{output?.result ? (
				<ResultSection
					value={output.result.value}
					format={env.resultFormat}
					touch={env.touch}
				/>
			) : null}
			{output && output.attachments.length > 0 ? (
				<FilesSection
					attachments={output.attachments}
					touch={env.touch}
					decimalSign={env.decimalSign}
				/>
			) : null}
			{pane.offersRoutes ? (
				<RoutesRow
					routes={env.routes}
					list={form.routes}
					touch={env.touch}
					centered={false}
				/>
			) : null}
		</>
	);
}

export interface RunBodyProps {
	readonly pane: PaneView;
	readonly env: PaneEnv;
	readonly form: Pick<FormModel, "name" | "description" | "routes">;
	readonly bodyRef: Ref<HTMLDivElement>;
	readonly onScroll: () => void;
}

/** The scrolling body of a run: waiting questions, notes, then Steps, Answer, Result, Files and Inputs (PLAN §7). */
export function RunBody({
	pane,
	env,
	form,
	bodyRef,
	onScroll,
}: Readonly<RunBodyProps>) {
	const { run } = pane;
	return (
		<div
			ref={bodyRef}
			onScroll={onScroll}
			className={cx(
				"relative flex min-h-0 flex-1 flex-col overflow-y-auto pt-6",
				env.pad,
				pane.centered ? "pb-6" : "pb-14",
			)}
		>
			{pane.centered ? (
				<CenteredCard pane={pane} form={form} env={env} />
			) : (
				<div className="flex w-full max-w-210 flex-col gap-8">
					{pane.waiting.length > 0 ? (
						<WaitingSection
							interactions={pane.waiting}
							onRespond={(id, value) =>
								env.actions.respondInteraction(run.id, id, value)
							}
						/>
					) : null}
					{pane.notes.map((note) => (
						<NoteItem key={note.kind} note={note} touch={env.touch} />
					))}
					<OutputSections pane={pane} env={env} form={form} />
					{pane.inputs ? (
						<InputsSection
							inputs={pane.inputs}
							touch={env.touch}
							onUse={() => env.actions.useInputs(run.id)}
						/>
					) : null}
				</div>
			)}
		</div>
	);
}
