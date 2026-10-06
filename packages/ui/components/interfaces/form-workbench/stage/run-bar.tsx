"use client";

import { useTranslation } from "@flow-like/locales";
import { Copy, Pin, RotateCcw } from "lucide-react";
import type { ComponentProps } from "react";
import { DvButton } from "../../../settings/devices/primitives/dv-button";
import { cx } from "../../../settings/devices/primitives/tone";
import {
	FOCUS_ATTR,
	FOCUS_VALUE,
	type FormSessionState,
	type RunEntry,
} from "../contracts";
import { formatClock } from "../run/format";
import { ChangeChips } from "./change-chips";
import { detailText, leadText } from "./copy";
import { useCopyRun } from "./copy-run";
import { guardRepeat } from "./key-guards";
import type { PaneEnv } from "./pane-env";
import type { PaneView } from "./pane-model";
import { RunMenu } from "./run-menu";
import { StatusIcon } from "./status-icon";
import { StopGlyph } from "./stop-glyph";

const SEP = <span className="font-normal text-muted-foreground"> · </span>;

/** "Running · 0:31 · Step 4: Match purchase order", "Done in 48 s", "Queued · 1st in line". */
function BarLead({
	pane,
	named,
	pinned,
}: Readonly<{ pane: PaneView; named: boolean; pinned: boolean }>) {
	const { t } = useTranslation("interfaces");
	const { bar, run } = pane;
	return (
		<div className="flex min-w-0 flex-[1_1_200px] items-start gap-2.25 font-semibold text-[15px]/[22px]">
			<StatusIcon status={run.status} className="mt-0.5 size-4.5" />
			<span className="min-w-0">
				{named ? (
					<>
						<span className="whitespace-nowrap">
							{t("workbench.stage.tab.run", "Run {{n}}", { n: run.n })}
						</span>
						{pinned ? (
							<Pin
								role="img"
								aria-label={t("workbench.stage.bar.pinned", "pinned")}
								className="ml-1.25 inline size-3.25 align-[-1px] text-muted-foreground"
							/>
						) : null}
						{SEP}
					</>
				) : null}
				<span>{leadText(t, bar)}</span>
				{bar.clockMs !== null ? (
					<span aria-hidden>
						{SEP}
						<span className="font-medium font-mono text-sm tabular-nums">
							{formatClock(bar.clockMs)}
						</span>
					</span>
				) : null}
				{bar.detail ? (
					<>
						{SEP}
						<span className="font-normal text-ink-2">
							{detailText(t, bar.detail)}
						</span>
					</>
				) : null}
			</span>
		</div>
	);
}

type BarButtonProps = ComponentProps<typeof DvButton> & {
	readonly touch: boolean;
};

function BarButton({ touch, className, ...props }: Readonly<BarButtonProps>) {
	return (
		<DvButton
			size="md"
			className={cx(touch && "h-11 px-3", "whitespace-nowrap", className)}
			{...props}
		/>
	);
}

interface ButtonProps {
	readonly run: RunEntry;
	readonly env: PaneEnv;
}

function StopButton({ run, env }: Readonly<ButtonProps>) {
	const { t } = useTranslation("interfaces");
	const label = t("workbench.stage.action.stopRun", "Stop run {{n}}", {
		n: run.n,
	});
	const shortcut = env.mac ? "⌘." : "Ctrl+.";
	return (
		<BarButton
			touch={env.touch}
			aria-label={label}
			title={`${label} · ${shortcut}`}
			aria-keyshortcuts={env.mac ? "Meta+." : "Control+."}
			{...{ [FOCUS_ATTR]: FOCUS_VALUE.stop }}
			onClick={() => env.actions.stop(run.id)}
		>
			<StopGlyph />
			{t("workbench.stage.action.stop", "Stop")}
		</BarButton>
	);
}

function RemoveFromQueueButton({ run, env }: Readonly<ButtonProps>) {
	const { t } = useTranslation("interfaces");
	return (
		<BarButton
			touch={env.touch}
			onClick={() => env.actions.removeFromQueue(run.id)}
		>
			{t("workbench.stage.action.removeFromQueue", "Remove from queue")}
		</BarButton>
	);
}

/** "Run again" / "Try again": the run's own copy; a held key never presses it twice. */
function RepeatButton({
	run,
	env,
	kind,
}: Readonly<ButtonProps & { kind: "runAgain" | "tryAgain" }>) {
	const { t } = useTranslation("interfaces");
	return (
		<BarButton
			touch={env.touch}
			{...{ [FOCUS_ATTR]: FOCUS_VALUE.runAgain }}
			onKeyDown={guardRepeat}
			onClick={() => env.actions.runAgain(run.id)}
		>
			<RotateCcw aria-hidden className="size-3.25" />
			{kind === "tryAgain"
				? t("workbench.stage.action.tryAgain", "Try again")
				: t("workbench.stage.action.runAgain", "Run again")}
		</BarButton>
	);
}

function UseButton({
	run,
	env,
	first,
}: Readonly<ButtonProps & { first: boolean }>) {
	const { t } = useTranslation("interfaces");
	return (
		<BarButton
			touch={env.touch}
			{...(first ? { [FOCUS_ATTR]: FOCUS_VALUE.runAgain } : {})}
			onKeyDown={guardRepeat}
			onClick={() => env.actions.useInputs(run.id)}
		>
			{t("workbench.stage.action.use", "Use these inputs")}
		</BarButton>
	);
}

function CopyButton({
	env,
	copied,
	kind,
	onCopy,
}: Readonly<{
	env: PaneEnv;
	copied: boolean;
	kind: "answer" | "result";
	onCopy: () => void;
}>) {
	const { t } = useTranslation("interfaces");
	let label = t("workbench.stage.action.copyResult", "Copy result");
	if (kind === "answer")
		label = t("workbench.stage.action.copyAnswer", "Copy answer");
	if (copied) label = t("workbench.stage.action.copied", "Copied");
	return (
		<BarButton
			touch={env.touch}
			{...{ [FOCUS_ATTR]: FOCUS_VALUE.copy }}
			onClick={onCopy}
		>
			<Copy aria-hidden className="size-3.25" />
			{label}
		</BarButton>
	);
}

export interface RunBarProps {
	readonly pane: PaneView;
	readonly env: PaneEnv;
	readonly overlay: FormSessionState["view"]["overlay"];
	/** Two panes: the lead names its run and the pin shows on the pinned one. */
	readonly named: boolean;
	readonly pinned: boolean;
	/** The "Inputs edited since this run" chip scrolls the pane to its Inputs. */
	readonly onGoInputs: () => void;
}

/** The lead line with the run's buttons, and the change chips under it (spec M5 anatomy, canvas run bar). */
export function RunBar({
	pane,
	env,
	overlay,
	named,
	pinned,
	onGoInputs,
}: Readonly<RunBarProps>) {
	const { run, actions } = pane;
	const { copied, copyNow } = useCopyRun(run, pane.copy, env.resultFormat);
	return (
		<div className={cx("shrink-0 pt-3.5", env.pad)}>
			<div className="flex min-h-8 flex-wrap items-center gap-x-3 gap-y-2">
				<BarLead pane={pane} named={named} pinned={pinned} />
				<div className="flex shrink-0 flex-wrap items-center gap-1.5">
					{actions.stop ? <StopButton run={run} env={env} /> : null}
					{actions.removeFromQueue ? (
						<RemoveFromQueueButton run={run} env={env} />
					) : null}
					{actions.repeat ? (
						<RepeatButton run={run} env={env} kind={actions.repeat} />
					) : null}
					{actions.use ? (
						<UseButton run={run} env={env} first={actions.repeat === null} />
					) : null}
					{actions.copy ? (
						<CopyButton
							env={env}
							copied={copied}
							kind={actions.copy}
							onCopy={copyNow}
						/>
					) : null}
					{actions.menu.length > 0 ? (
						<RunMenu
							run={run}
							items={actions.menu}
							overlay={overlay}
							env={env}
							onCopy={copyNow}
						/>
					) : null}
				</div>
			</div>
			{pane.chips ? (
				<ChangeChips
					chips={pane.chips}
					touch={env.touch}
					onGoInputs={onGoInputs}
				/>
			) : null}
		</div>
	);
}
