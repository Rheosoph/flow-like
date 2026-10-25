"use client";

import { timestampFormatOptions } from "./forecast-timestamps";

import { ArrowLeft, ArrowRight, Loader2, Play } from "lucide-react";
import { type ReactNode, useEffect, useId, useRef } from "react";
import { cn } from "../../../../lib/utils";
import type { StudioForecastPayload } from "../../../../state/backend-state/query-state";
import { Button } from "../../../ui/button";
import { forecastOutputLabel } from "./forecast-data";

const steps = [
	{
		title: "Source",
		description: "Choose the historical data and load its columns.",
	},
	{
		title: "Columns",
		description:
			"Choose what to predict, its timestamps, and any related inputs.",
	},
	{
		title: "Model",
		description: "Choose a forecasting model and how many steps to predict.",
	},
	{
		title: "Review",
		description: "Check the inputs before running your prediction.",
	},
] as const;

export type ForecastSetupStep = 0 | 1 | 2 | 3;

export function ForecastSetupWizard({
	step,
	onStepChange,
	errors,
	busy,
	running,
	onRun,
	onClose,
	children,
}: {
	step: ForecastSetupStep;
	onStepChange: (step: ForecastSetupStep) => void;
	errors: readonly (string | null)[];
	busy: boolean;
	running: boolean;
	onRun: () => void;
	onClose?: () => void;
	children: ReactNode;
}) {
	const id = useId();
	const heading = useRef<HTMLHeadingElement>(null);
	const previousStep = useRef(step);
	useEffect(() => {
		if (previousStep.current !== step) heading.current?.focus();
		previousStep.current = step;
	}, [step]);
	const reason = errors.slice(0, step + 1).find(Boolean);
	return (
		<section
			aria-label="Forecast setup"
			aria-busy={busy}
			className="space-y-5 rounded-xl border bg-card p-4 sm:p-6"
		>
			<div className="flex flex-wrap items-center justify-between gap-3">
				<p className="text-sm font-medium">Set up a prediction</p>
				{onClose && (
					<Button
						type="button"
						size="sm"
						variant="ghost"
						disabled={busy}
						onClick={onClose}
					>
						Back to prediction
					</Button>
				)}
			</div>
			<ol
				aria-label="Forecast setup steps"
				className="grid grid-cols-2 gap-2 sm:grid-cols-4"
			>
				{steps.map((item, index) => (
					<li key={item.title}>
						<button
							type="button"
							aria-current={index === step ? "step" : undefined}
							aria-label={`Step ${index + 1}: ${item.title}`}
							disabled={
								busy || (index > step && errors.slice(0, index).some(Boolean))
							}
							onClick={() => onStepChange(index as ForecastSetupStep)}
							className={cn(
								"flex w-full items-center gap-2 rounded-lg border px-3 py-2 text-left text-xs focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-40",
								index === step
									? "border-primary/40 bg-primary/10 font-medium"
									: "hover:bg-muted",
							)}
						>
							<span
								aria-hidden="true"
								className="flex size-5 shrink-0 items-center justify-center rounded-full border text-[10px]"
							>
								{index + 1}
							</span>
							{item.title}
						</button>
					</li>
				))}
			</ol>
			<div>
				<h3
					ref={heading}
					tabIndex={-1}
					className="text-base font-semibold outline-none"
				>
					Step {step + 1} of 4: {steps[step].title}
				</h3>
				<p className="mt-1 text-sm leading-relaxed text-muted-foreground">
					{steps[step].description}
				</p>
			</div>
			<fieldset
				disabled={busy}
				className="min-w-0 space-y-4 disabled:opacity-70"
			>
				{children}
			</fieldset>
			<div className="flex flex-wrap items-center justify-between gap-3 border-t pt-4">
				<p
					id={`${id}-reason`}
					aria-live="polite"
					className="min-w-0 flex-1 text-xs leading-relaxed text-muted-foreground"
				>
					{reason ??
						(step === 3
							? "Running reads your data. Save the completed prediction to keep or share it."
							: "Your choices stay here as you move between steps.")}
				</p>
				<div className="flex items-center gap-2">
					{step > 0 && (
						<Button
							type="button"
							variant="outline"
							disabled={busy}
							onClick={() => onStepChange((step - 1) as ForecastSetupStep)}
						>
							<ArrowLeft className="size-3.5" />
							Back
						</Button>
					)}
					<Button
						type="button"
						disabled={busy || Boolean(reason)}
						aria-describedby={`${id}-reason`}
						onClick={
							step === 3
								? onRun
								: () => onStepChange((step + 1) as ForecastSetupStep)
						}
						className="gap-2"
					>
						{step === 3 ? (
							<>
								{running ? (
									<Loader2 className="size-3.5 animate-spin" />
								) : (
									<Play className="size-3.5" />
								)}
								{running ? "Forecasting…" : "Run forecast"}
							</>
						) : (
							<>
								Next: {steps[step + 1].title}
								<ArrowRight className="size-3.5" />
							</>
						)}
					</Button>
				</div>
			</div>
		</section>
	);
}

export function ForecastSetupReview({
	request,
	modelName,
	userScoped,
	pointOnly = false,
}: {
	request: StudioForecastPayload;
	modelName: string;
	userScoped: boolean;
	pointOnly?: boolean;
}) {
	const mapping = request.mapping;
	const rows = [
		["Data scope", userScoped ? "Personal data" : "Project data"],
		["Timestamp", mapping.timestamp],
		[
			"Timestamp format",
			timestampFormatOptions.find(
				(option) => option.value === mapping.timestamp_format,
			)?.label ?? "Saved interpretation (numeric milliseconds)",
		],
		[
			"Forecast grouping",
			mapping.series_id
				? `Each ${mapping.series_id} separately`
				: "Single series from source rows",
		],
		["Metrics", mapping.targets.join(", ")],
		["Past covariates", mapping.past_covariates?.join(", ") || "None"],
		["Known future covariates", mapping.known_covariates?.join(", ") || "None"],
		[
			"Time interval",
			mapping.frequency_ms
				? `${mapping.frequency_ms} milliseconds`
				: "Infer from timestamps",
		],
		["Model", modelName],
		["Future steps", String(request.horizon)],
		[
			"Evaluation",
			request.backtest_horizon
				? `${request.backtest_horizon} holdout steps · ${request.backtest_offset ?? 0} steps after the window`
				: "Off",
		],
		[
			"Forecast output",
			forecastOutputLabel(request.quantile_levels, pointOnly),
		],
	];
	return (
		<div className="space-y-4">
			<p className="rounded-lg border bg-muted/30 p-3 text-xs leading-relaxed text-muted-foreground">
				{mapping.series_id
					? `Every ${mapping.series_id} gets its own forecast. The chart shows one group at a time; saving keeps all groups.`
					: "This run forecasts the source rows as one series. An overall total or average must already be calculated in the source SQL."}
			</p>
			<dl className="grid gap-x-8 gap-y-3 sm:grid-cols-2">
				{rows.map(([label, value]) => (
					<div key={label} className="min-w-0">
						<dt className="text-xs text-muted-foreground">{label}</dt>
						<dd className="mt-1 break-words text-sm font-medium">{value}</dd>
					</div>
				))}
			</dl>
			{[
				["Historical query", request.sql, request.params],
				...(request.future_sql
					? [["Future values query", request.future_sql, request.future_params]]
					: []),
			].map(([label, sql, params]) => (
				<details key={String(label)} className="rounded-lg border p-3 text-xs">
					<summary className="cursor-pointer font-medium">
						{String(label)}
					</summary>
					<pre className="mt-3 max-h-48 overflow-auto whitespace-pre-wrap break-words text-muted-foreground">
						{String(sql)}
					</pre>
					{params && Object.keys(params).length > 0 && (
						<pre className="mt-2 max-h-32 overflow-auto whitespace-pre-wrap break-words">
							{JSON.stringify(params, null, 2)}
						</pre>
					)}
				</details>
			))}
		</div>
	);
}
