"use client";

import * as SliderPrimitive from "@radix-ui/react-slider";
import { GripVertical, Loader2, Play } from "lucide-react";
import { useId } from "react";
import { Button } from "../../../ui/button";
import { Input } from "../../../ui/input";
import { Label } from "../../../ui/label";
import type { forecastBacktestRange } from "./forecast-backtest";
import { formatForecastTime } from "./forecast-data";

export function ForecastBacktestControls({
	range,
	offset,
	horizon,
	disabled,
	reason,
	running,
	onChange,
	onCommit,
	onHorizonChange,
}: {
	range: ReturnType<typeof forecastBacktestRange>;
	offset: number;
	horizon: number;
	disabled: boolean;
	reason?: string | null;
	running: boolean;
	onChange: (offset: number) => void;
	onCommit: (offset: number) => void;
	onHorizonChange?: (horizon: number) => void;
}) {
	const labelId = useId();
	const descriptionId = useId();
	const horizonId = useId();
	const percent = (count: number) =>
		range.totalCount > 0 ? (count / range.totalCount) * 100 : 0;
	const contextPercent = percent(range.contextCount);
	const holdoutPercent = range.valid ? percent(horizon) : 0;
	const earliestCutoffPercent = percent(
		range.totalCount - horizon - range.maxOffset,
	);
	const time = (timestamp?: number) =>
		timestamp === undefined
			? "Unavailable"
			: `${formatForecastTime(timestamp, true)} UTC`;
	return (
		<section
			className="space-y-3 rounded-xl border bg-card p-4"
			aria-label="Historical forecast cutoff"
		>
			<div className="flex flex-wrap items-start justify-between gap-4">
				<div className="space-y-1">
					<p id={labelId} className="text-xs font-medium">
						Holdout forecast as of
					</p>
					<p className="text-sm font-semibold tabular-nums" aria-live="polite">
						{time(range.cutoff)}
					</p>
				</div>
				<div className="flex flex-wrap items-end gap-2">
					{onHorizonChange && (
						<div className="space-y-1.5">
							<Label htmlFor={horizonId} className="text-xs">
								Holdout steps
							</Label>
							<Input
								id={horizonId}
								type="number"
								min={1}
								max={Math.min(65536, range.maxHorizon || 65536)}
								step={1}
								value={horizon || ""}
								disabled={running}
								onChange={(event) =>
									onHorizonChange(Number(event.currentTarget.value))
								}
								className="h-8 w-24 tabular-nums"
							/>
						</div>
					)}
					<Button
						type="button"
						size="sm"
						disabled={disabled || !range.valid}
						onClick={() => onCommit(offset)}
					>
						{running ? (
							<Loader2 className="size-3.5 animate-spin" />
						) : (
							<Play className="size-3.5" />
						)}
						{running ? "Evaluating…" : "Evaluate window"}
					</Button>
					<Button
						type="button"
						size="sm"
						variant="outline"
						disabled={
							disabled || !range.available || (range.valid && offset === 0)
						}
						onClick={() => {
							onChange(0);
							onCommit(0);
						}}
					>
						Latest window
					</Button>
				</div>
			</div>
			<div className="relative h-11">
				<div
					aria-hidden="true"
					className="absolute inset-x-0 top-3 flex h-5 overflow-hidden rounded-md border bg-muted"
				>
					<div
						className="h-full bg-muted-foreground/30"
						style={{ width: `${contextPercent}%` }}
					/>
					<div
						className="h-full border-x border-primary bg-primary/35"
						style={{ width: `${holdoutPercent}%` }}
					/>
				</div>
				<SliderPrimitive.Root
					className="absolute inset-y-0 flex touch-none select-none items-center data-[disabled]:opacity-50"
					style={
						range.available
							? {
									left: `calc(${earliestCutoffPercent}% - 10px)`,
									right: `calc(${percent(horizon)}% - 10px)`,
								}
							: { left: 0, right: 0 }
					}
					min={0}
					max={Math.max(1, range.maxOffset)}
					step={1}
					value={[
						Math.max(0, Math.min(range.maxOffset, range.maxOffset - offset)),
					]}
					disabled={disabled || !range.available || range.maxOffset === 0}
					onValueChange={([value]) => onChange(range.maxOffset - value)}
					onValueCommit={([value]) => onCommit(range.maxOffset - value)}
				>
					<SliderPrimitive.Track className="relative h-5 grow" />
					<SliderPrimitive.Thumb
						aria-labelledby={labelId}
						aria-describedby={descriptionId}
						aria-valuetext={
							range.valid
								? `${time(range.cutoff)}, evaluating the next ${horizon} steps`
								: "Choose a valid historical cutoff"
						}
						className="flex h-9 w-5 cursor-grab items-center justify-center rounded-md border-2 border-primary bg-background text-primary shadow-sm focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-ring/30 active:cursor-grabbing"
					>
						<GripVertical className="size-3.5" aria-hidden="true" />
					</SliderPrimitive.Thumb>
				</SliderPrimitive.Root>
			</div>
			{range.valid && (
				<div
					className="flex flex-wrap gap-x-5 gap-y-2 text-xs tabular-nums"
					aria-live="polite"
				>
					<span className="flex items-center gap-1.5">
						<span className="size-2.5 rounded-sm bg-muted-foreground/50" />
						History: {range.contextCount.toLocaleString()}{" "}
						{range.contextCount === 1 ? "step" : "steps"}
					</span>
					<span className="flex items-center gap-1.5">
						<span className="size-2.5 rounded-sm bg-primary/50" />
						Holdout: {horizon.toLocaleString()}{" "}
						{horizon === 1 ? "step" : "steps"}
					</span>
					{offset > 0 && (
						<span className="flex items-center gap-1.5 text-muted-foreground">
							<span className="size-2.5 rounded-sm border bg-muted" />
							Later: {offset.toLocaleString()} {offset === 1 ? "step" : "steps"}
						</span>
					)}
				</div>
			)}
			<div className="flex flex-wrap justify-between gap-x-6 gap-y-1 text-xs text-muted-foreground">
				<span>Earlier · {time(range.oldest)}</span>
				<span>Latest · {time(range.latest)}</span>
			</div>
			<div
				id={descriptionId}
				className="space-y-1 text-xs leading-relaxed text-muted-foreground"
			>
				{range.valid && (
					<p>
						Compare {time(range.start)} to {time(range.end)} · {horizon} steps.
					</p>
				)}
				<p>
					{running
						? "Evaluating the selected window…"
						: reason ||
							(!range.available
								? range.maxHorizon > 0
									? `Choose up to ${range.maxHorizon.toLocaleString()} holdout steps, leaving at least two earlier observations per series.`
									: "Run a forecast with enough history for the evaluation window and at least two earlier observations."
								: !range.valid
									? "The selected cutoff is outside the available history. Move the slider or choose Latest window."
									: "Drag the cutoff and release to evaluate. Arrow keys move one step; Home and End jump to the earliest and latest cutoff.")}
				</p>
				<details className="pt-1">
					<summary className="cursor-pointer text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
						How holdout works
					</summary>
					<div className="mt-2 space-y-1">
						<p>
							The holdout uses history through the cutoff. Known future inputs
							keep their configured availability. Outlook continues from the
							latest observation.
						</p>
						{range.seriesCount > 1 && (
							<p>
								Each series uses the same step offset from its own end. Dates
								above are for the selected series.
							</p>
						)}
					</div>
				</details>
			</div>
		</section>
	);
}
