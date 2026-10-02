"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Check,
	ChevronLeft,
	ChevronRight,
	CircleCheck,
	CircleDashed,
	CircleDot,
	type LucideIcon,
	OctagonX,
	X,
} from "lucide-react";
import { type MouseEventHandler, type ReactNode, type Ref, useId } from "react";
import { DvButton } from "./dv-button";
import { type Gate, GateInline, GatedAction } from "./gate-notice";
import { ProgressBar } from "./meter";
import { cx } from "./tone";

type StepperState = "done" | "current" | "todo";

function StepperStep({
	index,
	step,
	state,
}: Readonly<{ index: number; step: string; state: StepperState }>) {
	const { t } = useTranslation("devices");
	return (
		<>
			<span
				className={cx(
					"inline-flex size-5.5 shrink-0 items-center justify-center rounded-full border font-mono text-xs font-medium",
					state === "current" &&
						"border-foreground bg-foreground text-background",
					state === "done" && "border-good-line bg-good-bg text-good",
					state === "todo" && "border-border bg-card text-muted-foreground",
				)}
			>
				{state === "done" ? (
					<Check aria-hidden className="size-3" />
				) : (
					index + 1
				)}
			</span>
			<span
				title={step}
				className={cx(
					"truncate text-ui max-[1200px]:leading-3.75 max-[1200px]:whitespace-normal @max-[1100px]/devices:leading-3.75 @max-[1100px]/devices:whitespace-normal",
					state === "current" && "font-semibold text-foreground",
					state === "done" && "text-ink-2",
					state === "todo" && "text-muted-foreground",
				)}
			>
				{step}
				{state === "done" ? (
					<span className="sr-only">{t("view.wizard.doneSr", " (done)")}</span>
				) : null}
			</span>
		</>
	);
}

/**
 * SPEC §4.22: numbered steps joined by hairlines. Labels wrap instead of
 * ending in "…" in a window up to 1200 px or an area up to 1100 px wide; below
 * 720 px of container width a "Step 3 of 8 · Device password" line with a bar
 * replaces the row.
 */
export function WizardStepper({
	steps,
	current,
	label,
	done,
	titles,
	reachable,
	onSelect,
	fit = false,
	className,
}: Readonly<{
	steps: readonly string[];
	/** Zero-based index of the current step. */
	current: number;
	label?: string;
	/** Per step: shows the check mark (a prefilled step ahead of the current one). Default: every earlier step. */
	done?: readonly boolean[];
	/** Per step: the full title for the phone line and the button names ("What to run"); the label otherwise. */
	titles?: readonly string[];
	/** Per step: may be opened from the stepper. Needs `onSelect`. */
	reachable?: readonly boolean[];
	onSelect?(index: number): void;
	/** Each step as wide as its label instead of equal widths (many steps on one line). */
	fit?: boolean;
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	const titleOf = (index: number) => titles?.[index] ?? steps[index] ?? "";
	const compact = t(
		"view.wizard.compact",
		"Step {{n, number}} of {{count, number}} · {{step}}",
		{
			n: current + 1,
			count: steps.length,
			step: titleOf(current),
		},
	);
	return (
		<div className={cx("@container/stepper min-w-0", className)}>
			<ol
				aria-label={label ?? t("view.wizard.steps", "Steps")}
				className="flex min-w-0 items-center @max-[720px]/stepper:hidden"
			>
				{steps.map((step, index) => {
					const state: StepperState =
						index === current
							? "current"
							: (done?.[index] ?? index < current)
								? "done"
								: "todo";
					const body = <StepperStep index={index} step={step} state={state} />;
					return (
						<li
							key={step}
							data-s={state}
							aria-current={state === "current" ? "step" : undefined}
							className={cx(
								"flex min-w-0 flex-1 items-center gap-2 last:flex-none data-[s=current]:min-w-fit [&:not(:last-child)]:after:mr-2 [&:not(:last-child)]:after:h-px [&:not(:last-child)]:after:min-w-2 [&:not(:last-child)]:after:flex-auto [&:not(:last-child)]:after:bg-border [&:not(:last-child)]:after:content-['']",
								fit && "flex-auto",
							)}
						>
							{onSelect && reachable?.[index] && index !== current ? (
								<button
									type="button"
									onClick={() => onSelect(index)}
									aria-label={t(
										"view.wizard.goTo",
										"Go to step {{n, number}}: {{step}}",
										{ n: index + 1, step: titleOf(index) },
									)}
									className="flex min-w-0 cursor-pointer items-center gap-2 rounded-md text-left hover:[&>span:last-child]:text-foreground hover:[&>span:last-child]:underline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
								>
									{body}
								</button>
							) : (
								body
							)}
						</li>
					);
				})}
			</ol>
			<div className="hidden flex-col gap-1.5 @max-[720px]/stepper:flex">
				<p className="text-ui font-medium">{compact}</p>
				<ProgressBar
					value={((current + 1) / Math.max(1, steps.length)) * 100}
					label={compact}
				/>
			</div>
		</div>
	);
}

/** The page title row: the exit left of the title, divided by a hairline (stacked below 720 px). */
export function WizardTitleRow({
	exitLabel,
	onExit,
	exitLink,
	exitTitle,
	title,
	sub,
	className,
}: Readonly<{
	/** "Exit setup" / "Exit deploy". */
	exitLabel: string;
	onExit?: () => void;
	/** An exit that leads to another page: a real link whose click the host router takes, so nothing reloads. */
	exitLink?: Readonly<{
		href: string;
		onClick?: MouseEventHandler<HTMLAnchorElement>;
	}>;
	/** Hover: "Your choices stay in this window, so you can come back and continue." */
	exitTitle?: string;
	title: ReactNode;
	sub?: ReactNode;
	className?: string;
}>) {
	return (
		<div className={cx("@container/wtitle min-w-0", className)}>
			<div className="flex min-w-0 items-center gap-3 @max-[720px]/wtitle:flex-col @max-[720px]/wtitle:items-start @max-[720px]/wtitle:gap-2">
				{exitLink ? (
					<DvButton
						size="sm"
						variant="ghost"
						icon={X}
						title={exitTitle}
						asChild
					>
						<a {...exitLink}>{exitLabel}</a>
					</DvButton>
				) : (
					<DvButton
						size="sm"
						variant="ghost"
						icon={X}
						title={exitTitle}
						onClick={onExit}
						className="shrink-0"
					>
						{exitLabel}
					</DvButton>
				)}
				<div className="min-w-0 border-l border-hairline pl-3 @max-[720px]/wtitle:border-l-0 @max-[720px]/wtitle:pl-0">
					<h1 className="text-2xl leading-7.5 font-semibold tracking-[-0.015em]">
						{title}
					</h1>
					{sub ? (
						<p className="mt-0.5 text-ui text-muted-foreground">{sub}</p>
					) : null}
				</div>
			</div>
		</div>
	);
}

/** Opens each step: h2, a muted lede, and "Step N of M" for screen readers only. */
export function WizardStepHeader({
	title,
	lede,
	step,
	total,
	headingRef,
	className,
}: Readonly<{
	title: ReactNode;
	lede?: ReactNode;
	/** One-based. */
	step?: number;
	total?: number;
	/** Focus target after a step change. */
	headingRef?: Ref<HTMLHeadingElement>;
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	return (
		<div className={cx("flex flex-col gap-1", className)}>
			<h2
				ref={headingRef}
				tabIndex={-1}
				className="text-base leading-5.5 font-semibold tracking-[-0.005em] outline-none"
			>
				{step !== undefined && total !== undefined ? (
					<span className="sr-only">
						{t(
							"view.wizard.stepOf",
							"Step {{n, number}} of {{count, number}}: ",
							{
								n: step,
								count: total,
							},
						)}
					</span>
				) : null}
				{title}
			</h2>
			{lede ? (
				<p className="max-w-[72ch] text-sm leading-5 text-muted-foreground">
					{lede}
				</p>
			) : null}
		</div>
	);
}

export interface WizardFootProps {
	/** One-based. */
	step: number;
	total: number;
	stepLabel: string;
	nextStepLabel?: string;
	/** Overrides the "Step N of M · step · next: step" note. */
	status?: ReactNode;
	/** "Cancel setup…" (danger-ghost) on the left. */
	cancel?: ReactNode;
	onBack?: () => void;
	backLabel?: string;
	onNext?: () => void;
	/** The last step names the verb ("Update nightly-sync"). */
	nextLabel?: string;
	nextIcon?: LucideIcon;
	nextBusy?: boolean;
	/** Why the primary is off, shown under it. Prefer `reason`, which keeps the row one line high. */
	nextGate?: Gate | null;
	/**
	 * Why the primary is off, shown in the note area at every width (R7): on a
	 * phone it takes its own row above the buttons instead of disappearing.
	 */
	reason?: Gate | null;
	/** Back stays visible but off, with this reason in the note area ("Earlier steps are locked: …"). */
	backGate?: Gate | null;
	/** An inline result on its own row above the buttons (a failed cancel). */
	result?: ReactNode;
	/** Replaces Back/Continue (finished or failed runs). */
	children?: ReactNode;
	className?: string;
}

/** The sticky wizard foot: cancel left, the step note or a gate reason, Back and the one primary. */
export function WizardFoot({
	step,
	total,
	stepLabel,
	nextStepLabel,
	status,
	cancel,
	onBack,
	backLabel,
	onNext,
	nextLabel,
	nextIcon,
	nextBusy = false,
	nextGate,
	reason,
	backGate,
	result,
	children,
	className,
}: Readonly<WizardFootProps>) {
	const { t } = useTranslation("devices");
	const reasonId = useId();
	const backReasonId = useId();
	const shown = reason ?? backGate ?? null;
	const note =
		status ??
		(nextStepLabel ? (
			<>
				<b className="font-medium text-ink-2">
					{t(
						"view.wizard.stepCount",
						"Step {{n, number}} of {{count, number}}",
						{
							n: step,
							count: total,
						},
					)}
				</b>
				{t("view.wizard.stepNext", " · {{step}} · next: {{next}}", {
					step: stepLabel,
					next: nextStepLabel,
				})}
			</>
		) : (
			<>
				<b className="font-medium text-ink-2">
					{t(
						"view.wizard.stepCount",
						"Step {{n, number}} of {{count, number}}",
						{
							n: step,
							count: total,
						},
					)}
				</b>
				{t("view.wizard.stepOnly", " · {{step}}", { step: stepLabel })}
			</>
		));
	const NextIcon = nextIcon;
	const nextBody = (
		<>
			{nextLabel ?? t("view.wizard.continue", "Continue")}
			{NextIcon ? (
				<NextIcon aria-hidden className="size-4" />
			) : (
				<ChevronRight aria-hidden className="size-4" />
			)}
		</>
	);
	return (
		<div
			data-wizard-foot=""
			className={cx(
				"@container/wfoot sticky bottom-2 z-5 flex flex-wrap items-center gap-2 rounded-lg border border-border bg-surface-sunken px-3 py-2.5",
				className,
			)}
		>
			{result ? <div className="basis-full">{result}</div> : null}
			{cancel}
			{shown ? (
				<span className="flex min-w-[16ch] flex-1 items-center @max-[720px]/wfoot:order-first @max-[720px]/wfoot:basis-full">
					<GateInline
						kind={shown.kind}
						id={reason ? reasonId : backReasonId}
						className="max-w-[52ch]"
					>
						{shown.reason}
					</GateInline>
				</span>
			) : (
				<output className="min-w-[16ch] flex-1 text-xs text-muted-foreground @max-[720px]/wfoot:hidden">
					{note}
				</output>
			)}
			{reason && backGate ? (
				<span id={backReasonId} className="sr-only">
					{backGate.reason}
				</span>
			) : null}
			{children ?? (
				<>
					{onBack ? (
						<DvButton
							icon={ChevronLeft}
							aria-disabled={backGate ? true : undefined}
							aria-describedby={backGate ? backReasonId : undefined}
							data-gated={backGate?.kind}
							onClick={onBack}
						>
							{backLabel ?? t("view.wizard.back", "Back")}
						</DvButton>
					) : null}
					{onNext && reason ? (
						<DvButton
							variant="primary"
							busy={nextBusy}
							aria-disabled
							aria-describedby={reasonId}
							data-gated={reason.kind}
							onClick={onNext}
						>
							{nextBody}
						</DvButton>
					) : onNext ? (
						<GatedAction gate={nextGate}>
							<DvButton variant="primary" busy={nextBusy} onClick={onNext}>
								{nextBody}
							</DvButton>
						</GatedAction>
					) : null}
				</>
			)}
		</div>
	);
}

export type WizardSummaryState = "done" | "current" | "todo" | "err";

export interface WizardSummaryItem {
	id: string;
	label: string;
	value?: ReactNode;
	state: WizardSummaryState;
	/** `err`: how many fields to fix. */
	errors?: number;
	/** Steps you may jump to. */
	onSelect?: () => void;
}

const SUMMARY_ICON: Record<
	WizardSummaryState,
	{ icon: LucideIcon; tone: string }
> = {
	done: { icon: CircleCheck, tone: "text-good" },
	current: { icon: CircleDot, tone: "text-foreground" },
	todo: { icon: CircleDashed, tone: "text-unknown" },
	err: { icon: OctagonX, tone: "text-critical" },
};

/** The side summary of choices per step; reachable steps are buttons. */
export function WizardSummary({
	items,
	title,
	foot,
	className,
}: Readonly<{
	items: readonly WizardSummaryItem[];
	title?: ReactNode;
	/** "Nothing changes on any device until you deploy." */
	foot?: ReactNode;
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	return (
		<section
			aria-label={
				typeof title === "string"
					? title
					: t("view.wizard.summary", "Your choices")
			}
			className={cx(
				"overflow-hidden rounded-lg border border-border bg-card",
				className,
			)}
		>
			<h2 className="border-b border-hairline px-4 py-3 text-[15px] leading-5 font-semibold">
				{title ?? t("view.wizard.summary", "Your choices")}
			</h2>
			<ol className="flex flex-col">
				{items.map((item) => {
					const { icon: Icon, tone } = SUMMARY_ICON[item.state];
					const inner = (
						<>
							<Icon aria-hidden className={cx("mt-px size-3.5", tone)} />
							<span className="text-label font-semibold tracking-[0.06em] text-muted-foreground uppercase">
								{item.state === "err" && item.errors
									? t(
											"view.wizard.toFix",
											"{{step}} · {{count, number}} to fix",
											{
												step: item.label,
												count: item.errors,
											},
										)
									: item.label}
							</span>
							<span className="col-start-2 min-w-0 text-ui wrap-break-word">
								{item.value ?? (
									<span className="text-muted-foreground">
										{t("view.wizard.notYet", "Not chosen yet")}
									</span>
								)}
							</span>
						</>
					);
					const cell =
						"grid w-full grid-cols-[16px_minmax(0,1fr)] gap-x-2.5 gap-y-0.5 px-4 py-2.25 text-left";
					return (
						<li
							key={item.id}
							data-s={item.state}
							aria-current={item.state === "current" ? "step" : undefined}
							className={cx(
								"border-t border-hairline first:border-t-0",
								item.state === "current" && "bg-row-selected",
							)}
						>
							{item.onSelect ? (
								<button
									type="button"
									onClick={item.onSelect}
									className={cx(
										cell,
										"cursor-pointer hover:bg-row-hover focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring",
									)}
								>
									{inner}
								</button>
							) : (
								<div className={cell}>{inner}</div>
							)}
						</li>
					);
				})}
			</ol>
			{foot ? (
				<p className="border-t border-hairline bg-surface-sunken px-4 py-2.5 text-xs text-muted-foreground">
					{foot}
				</p>
			) : null}
		</section>
	);
}

/**
 * `.wiz-grid`: stepper, body (max 760 px) and foot in the main column, the
 * summary sticky at the side; one column below 900 px of container width.
 * `top` spans both columns (a stepper with many steps).
 */
export function WizardLayout({
	top,
	stepper,
	side,
	foot,
	children,
	className,
}: Readonly<{
	top?: ReactNode;
	stepper?: ReactNode;
	side?: ReactNode;
	foot?: ReactNode;
	children: ReactNode;
	className?: string;
}>) {
	return (
		<div className={cx("@container/wiz min-w-0", className)}>
			<div
				className={cx(
					"grid max-w-[1124px] items-start gap-x-6 gap-y-4",
					side
						? "grid-cols-[minmax(0,760px)_minmax(280px,340px)] @max-[900px]/wiz:grid-cols-1"
						: "grid-cols-[minmax(0,760px)]",
				)}
			>
				{top ? <div className="col-span-full min-w-0">{top}</div> : null}
				<div className="flex min-w-0 flex-col gap-4">
					{stepper}
					{children}
					{foot}
				</div>
				{side ? (
					<aside className="sticky top-4 flex min-w-0 flex-col gap-3 @max-[900px]/wiz:static">
						{side}
					</aside>
				) : null}
			</div>
		</div>
	);
}
