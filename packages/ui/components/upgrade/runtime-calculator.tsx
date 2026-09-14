"use client";

import { Calculator, ChevronDown } from "lucide-react";
import { useId, useState } from "react";
import {
	estimateCloudRuns,
	formatRuntimeSeconds,
	type RuntimeSample,
} from "../../lib/runtime-estimate";
import { Button } from "../ui/button";

export interface RuntimePlan {
	id: string;
	name: string;
	runtimeMs?: number;
	cloudStarts?: number;
}

export function RuntimeCalculator({
	plans,
	sample,
	remaining,
}: {
	plans: RuntimePlan[];
	sample?: RuntimeSample | null;
	remaining?: RuntimePlan;
}) {
	const inputId = useId();
	const [customSeconds, setCustomSeconds] = useState<string | null>(null);
	const personal = customSeconds === null && !!sample;
	const seconds =
		customSeconds ?? (sample ? String(sample.averageRuntimeMs / 1000) : "30");
	const averageMs = Number(seconds) * 1000;
	const valid =
		seconds.trim() !== "" && Number.isFinite(averageMs) && averageMs > 0;
	const examples = remaining ? [remaining, ...plans] : plans;
	return (
		<details
			className="group rounded-xl border bg-card"
			data-runtime-calculator
		>
			<summary className="flex cursor-pointer list-none items-center justify-between gap-3 p-4 font-medium [&::-webkit-details-marker]:hidden">
				<span className="flex items-center gap-2">
					<Calculator
						className="size-4 text-muted-foreground"
						aria-hidden="true"
					/>
					How many cloud runs does this cover?
				</span>
				<ChevronDown
					className="size-4 shrink-0 transition-transform group-open:rotate-180"
					aria-hidden="true"
				/>
			</summary>
			<div className="space-y-5 border-t p-4 sm:p-5">
				<p className="text-sm text-muted-foreground">
					{sample
						? `Your recent average is ${formatRuntimeSeconds(sample.averageRuntimeMs)} seconds across ${sample.cloudStarts.toLocaleString()} settled cloud runs. Try a different duration to compare.`
						: "Start with a 30-second example, or enter the average runtime of your workflow."}
				</p>
				<div className="flex flex-wrap items-end gap-3">
					<div className="space-y-2">
						<label htmlFor={inputId} className="block text-sm font-medium">
							Average cloud runtime per run
						</label>
						<div className="flex items-center gap-2">
							<input
								id={inputId}
								type="number"
								inputMode="decimal"
								min="0"
								step="any"
								value={
									personal
										? String(Number(Number(seconds).toPrecision(3)))
										: seconds
								}
								onChange={(event) => setCustomSeconds(event.target.value)}
								aria-invalid={!valid}
								aria-describedby={`${inputId}-help`}
								className="w-32 min-w-0 rounded-lg border bg-background px-3 py-2 text-sm tabular-nums focus-visible:outline-2 focus-visible:outline-ring"
							/>
							<span className="text-sm text-muted-foreground">seconds</span>
						</div>
					</div>
					<div
						className="flex flex-wrap gap-2"
						role="group"
						aria-label="Example run durations"
					>
						{[5, 30, 120].map((value) => (
							<Button
								key={value}
								variant={
									!personal && Number(seconds) === value
										? "secondary"
										: "outline"
								}
								size="sm"
								aria-pressed={!personal && Number(seconds) === value}
								onClick={() => setCustomSeconds(String(value))}
							>
								{value === 120 ? "2 min" : `${value} sec`}
							</Button>
						))}
						{sample && (
							<Button
								variant={personal ? "secondary" : "outline"}
								size="sm"
								aria-pressed={personal}
								onClick={() => setCustomSeconds(null)}
							>
								Use my average
							</Button>
						)}
					</div>
				</div>
				<p id={`${inputId}-help`} className="text-xs text-muted-foreground">
					Cloud runtime includes time waiting for models and other services.
					Local runs are always free.
				</p>
				<div aria-live="polite" aria-atomic="true">
					{!valid ? (
						<p className="text-sm text-destructive">
							Enter a duration greater than zero.
						</p>
					) : (
						<>
							<p className="mb-3 text-xs font-medium text-muted-foreground">
								{personal
									? "Estimate based on your recent usage"
									: `Example at ${formatRuntimeSeconds(averageMs)} seconds per run`}
							</p>
							<dl
								className={`grid grid-cols-2 gap-3 ${examples.length > 2 ? "lg:grid-cols-4" : ""}`}
							>
								{examples.map((plan) => {
									const estimate = estimateCloudRuns(
										plan.runtimeMs,
										plan.cloudStarts,
										averageMs,
									);
									return (
										<div
											key={plan.id}
											className="min-w-0 rounded-lg border bg-muted/20 p-4"
										>
											<dt className="text-sm font-medium">{plan.name}</dt>
											<dd className="mt-2">
												<span className="text-xl font-semibold tabular-nums">
													{!estimate
														? "Unavailable"
														: estimate.runs === null
															? "No runtime or start limit"
															: `≈ ${estimate.runs.toLocaleString()}`}
												</span>
												<p className="mt-1 text-xs text-muted-foreground">
													{plan.id === remaining?.id
														? "cloud runs available"
														: "cloud runs per month"}
												</p>
												{estimate?.startLimited && (
													<p className="mt-2 text-xs text-muted-foreground">
														Cloud-start limit reached first.
													</p>
												)}
											</dd>
										</div>
									);
								})}
							</dl>
						</>
					)}
				</div>
				<div className="space-y-1 text-xs leading-relaxed text-muted-foreground">
					<p>
						Estimates use both runtime and cloud-start allowances. Hosted AI and
						other plan limits still apply. Workflow duration can vary.
					</p>
					{sample && (
						<p>
							Your average uses the past 30 days, excluding today. It includes
							failed or cancelled cloud runs that started. Late usage reports
							can change it.
						</p>
					)}
					{!sample && (
						<p>
							Personal estimates appear after at least 10 settled cloud runs
							from earlier days, when a complete usage sample is available.
						</p>
					)}
					{remaining && (
						<p>
							Available allowance excludes usage already recorded and capacity
							reserved for pending work.
						</p>
					)}
				</div>
			</div>
		</details>
	);
}
