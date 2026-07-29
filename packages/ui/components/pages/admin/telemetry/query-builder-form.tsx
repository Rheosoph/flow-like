"use client";

import { Play, Plus, TriangleAlert, X } from "lucide-react";
import { useCallback, useMemo } from "react";
import {
	Button,
	Input,
	Label,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../../ui";
import {
	type ITelemetryQueryFilter,
	type ITelemetryQueryFilterOp,
	type ITelemetryQueryInterval,
	type ITelemetryQueryMetricType,
	type ITelemetryQueryRequest,
	TELEMETRY_QUERY_DATASETS,
	TELEMETRY_QUERY_HOUR_OPTIONS,
	TELEMETRY_QUERY_INTERVALS,
	TELEMETRY_QUERY_MAX_FILTERS,
	TELEMETRY_QUERY_METRIC_TYPES,
	telemetryBreakdownFields,
	telemetryDatasetSpec,
	telemetryFieldSpec,
	telemetryFilterOpsForKind,
	telemetryMetricNeedsField,
	telemetryMetricNeedsNumericField,
	telemetryNumericFields,
	telemetryQueryCapWarning,
	telemetryQueryFilterValueFromText,
	telemetryQueryFilterValueToText,
} from "./query-types";

const NONE = "__none__";

function FieldSection({
	label,
	hint,
	children,
}: {
	readonly label: string;
	readonly hint?: string;
	readonly children: React.ReactNode;
}) {
	return (
		<div className="space-y-1.5">
			<Label className="text-xs uppercase tracking-wide text-muted-foreground">
				{label}
			</Label>
			{children}
			{hint ? (
				<p className="text-[11px] text-muted-foreground">{hint}</p>
			) : null}
		</div>
	);
}

function FilterRow({
	dataset,
	filter,
	index,
	onChange,
	onRemove,
}: {
	readonly dataset: string;
	readonly filter: ITelemetryQueryFilter;
	readonly index: number;
	readonly onChange: (index: number, next: ITelemetryQueryFilter) => void;
	readonly onRemove: (index: number) => void;
}) {
	const fields = telemetryDatasetSpec(dataset)?.fields ?? [];
	const spec = telemetryFieldSpec(dataset, filter.field);
	const ops = telemetryFilterOpsForKind(spec?.kind);
	const text = telemetryQueryFilterValueToText(filter.value);

	const setField = (field: string) => {
		const next = telemetryFieldSpec(dataset, field);
		const nextOps = telemetryFilterOpsForKind(next?.kind);
		const op = nextOps[0]?.value ?? "eq";
		onChange(index, {
			field,
			op,
			value: telemetryQueryFilterValueFromText(next?.kind, op, ""),
		});
	};

	const setOp = (op: ITelemetryQueryFilterOp) => {
		onChange(index, {
			...filter,
			op,
			value: telemetryQueryFilterValueFromText(spec?.kind, op, text),
		});
	};

	const setValue = (raw: string) => {
		onChange(index, {
			...filter,
			value: telemetryQueryFilterValueFromText(spec?.kind, filter.op, raw),
		});
	};

	return (
		<div className="flex flex-wrap items-center gap-1.5">
			<Select value={filter.field} onValueChange={setField}>
				<SelectTrigger className="h-8 min-w-[8rem] flex-1 text-xs">
					<SelectValue placeholder="Field" />
				</SelectTrigger>
				<SelectContent>
					{fields.map((field) => (
						<SelectItem key={field.field} value={field.field}>
							{field.label}
						</SelectItem>
					))}
				</SelectContent>
			</Select>
			<Select
				value={filter.op}
				onValueChange={(v) => setOp(v as ITelemetryQueryFilterOp)}
			>
				<SelectTrigger className="h-8 w-24 text-xs">
					<SelectValue placeholder="Operator" />
				</SelectTrigger>
				<SelectContent>
					{ops.map((op) => (
						<SelectItem key={op.value} value={op.value}>
							{op.label}
						</SelectItem>
					))}
				</SelectContent>
			</Select>
			{spec?.kind === "bool" ? (
				<Select
					value={text === "true" ? "true" : "false"}
					onValueChange={setValue}
				>
					<SelectTrigger className="h-8 min-w-[7rem] flex-1 text-xs">
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="true">true</SelectItem>
						<SelectItem value="false">false</SelectItem>
					</SelectContent>
				</Select>
			) : (
				<Input
					value={text}
					onChange={(e) => setValue(e.target.value)}
					inputMode={spec?.kind === "number" ? "decimal" : "text"}
					placeholder={
						filter.op === "in" ? "value, value, …" : (spec?.label ?? "Value")
					}
					className="h-8 min-w-[7rem] flex-1 text-xs"
				/>
			)}
			<Button
				variant="ghost"
				size="icon"
				className="h-8 w-8 shrink-0"
				onClick={() => onRemove(index)}
				aria-label={`Remove filter ${index + 1}`}
			>
				<X className="h-3.5 w-3.5" />
			</Button>
		</div>
	);
}

export interface TelemetryQueryBuilderFormProps {
	value: ITelemetryQueryRequest;
	onChange: (next: ITelemetryQueryRequest) => void;
	onRun: () => void;
	running?: boolean;
	errors?: readonly string[];
}

export function TelemetryQueryBuilderForm({
	value,
	onChange,
	onRun,
	running = false,
	errors = [],
}: Readonly<TelemetryQueryBuilderFormProps>) {
	const spec = telemetryDatasetSpec(value.dataset);
	const fields = spec?.fields ?? [];
	const numeric = useMemo(
		() => telemetryNumericFields(value.dataset),
		[value.dataset],
	);
	const breakdownFields = useMemo(
		() => telemetryBreakdownFields(value.dataset),
		[value.dataset],
	);
	const metricFields = telemetryMetricNeedsNumericField(value.metric.type)
		? numeric
		: fields;
	const needsField = telemetryMetricNeedsField(value.metric.type);
	const filters = value.filters ?? [];
	const capWarning = useMemo(() => telemetryQueryCapWarning(value), [value]);

	const setDataset = useCallback(
		(dataset: string) => {
			const next = telemetryDatasetSpec(dataset);
			if (!next) return;
			onChange({
				...value,
				dataset: next.dataset,
				metric: { type: value.metric.type, field: null },
				filters: [],
				breakdown: null,
			});
		},
		[onChange, value],
	);

	const setMetricType = useCallback(
		(type: ITelemetryQueryMetricType) => {
			const allowed = telemetryMetricNeedsNumericField(type)
				? telemetryNumericFields(value.dataset)
				: (telemetryDatasetSpec(value.dataset)?.fields ?? []);
			const keep =
				value.metric.field &&
				allowed.some((field) => field.field === value.metric.field)
					? value.metric.field
					: (allowed[0]?.field ?? null);
			onChange({
				...value,
				metric: {
					type,
					field: telemetryMetricNeedsField(type) ? keep : null,
				},
			});
		},
		[onChange, value],
	);

	const updateFilter = useCallback(
		(index: number, next: ITelemetryQueryFilter) => {
			const copy = [...filters];
			copy[index] = next;
			onChange({ ...value, filters: copy });
		},
		[filters, onChange, value],
	);

	const removeFilter = useCallback(
		(index: number) => {
			onChange({ ...value, filters: filters.filter((_, i) => i !== index) });
		},
		[filters, onChange, value],
	);

	const addFilter = useCallback(() => {
		const first = fields[0];
		if (!first) return;
		const op = telemetryFilterOpsForKind(first.kind)[0]?.value ?? "eq";
		onChange({
			...value,
			filters: [
				...filters,
				{
					field: first.field,
					op,
					value: telemetryQueryFilterValueFromText(first.kind, op, ""),
				},
			],
		});
	}, [fields, filters, onChange, value]);

	return (
		<div className="space-y-4">
			<FieldSection label="Dataset" hint={spec?.description}>
				<Select value={value.dataset} onValueChange={setDataset}>
					<SelectTrigger className="w-full">
						<SelectValue placeholder="Dataset" />
					</SelectTrigger>
					<SelectContent>
						{TELEMETRY_QUERY_DATASETS.map((dataset) => (
							<SelectItem key={dataset.dataset} value={dataset.dataset}>
								{dataset.label}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			</FieldSection>

			<FieldSection label="Metric">
				<div className="flex flex-wrap gap-1.5">
					<Select
						value={value.metric.type}
						onValueChange={(v) => setMetricType(v as ITelemetryQueryMetricType)}
					>
						<SelectTrigger className="min-w-[9rem] flex-1">
							<SelectValue placeholder="Metric" />
						</SelectTrigger>
						<SelectContent>
							{TELEMETRY_QUERY_METRIC_TYPES.map((metric) => (
								<SelectItem key={metric.value} value={metric.value}>
									{metric.label}
								</SelectItem>
							))}
						</SelectContent>
					</Select>
					{needsField ? (
						<Select
							value={value.metric.field ?? NONE}
							onValueChange={(v) =>
								onChange({
									...value,
									metric: {
										type: value.metric.type,
										field: v === NONE ? null : v,
									},
								})
							}
						>
							<SelectTrigger className="min-w-[9rem] flex-1">
								<SelectValue placeholder="Field" />
							</SelectTrigger>
							<SelectContent>
								{metricFields.length === 0 ? (
									<SelectItem value={NONE} disabled>
										No numeric field on this dataset
									</SelectItem>
								) : (
									metricFields.map((field) => (
										<SelectItem key={field.field} value={field.field}>
											{field.label}
										</SelectItem>
									))
								)}
							</SelectContent>
						</Select>
					) : null}
				</div>
			</FieldSection>

			<div className="space-y-1.5">
				<div className="flex items-center justify-between">
					<Label className="text-xs uppercase tracking-wide text-muted-foreground">
						Filters
					</Label>
					<Button
						variant="ghost"
						size="sm"
						className="h-7 px-2 text-xs"
						onClick={addFilter}
						disabled={filters.length >= TELEMETRY_QUERY_MAX_FILTERS}
					>
						<Plus className="mr-1 h-3.5 w-3.5" />
						Add
					</Button>
				</div>
				{filters.length === 0 ? (
					<div className="flex items-center justify-center rounded-lg border border-dashed py-4 text-[11px] text-muted-foreground">
						No filters — the whole dataset is scanned.
					</div>
				) : (
					<div className="space-y-1.5">
						{filters.map((filter, index) => (
							<FilterRow
								key={`${filter.field}-${index}`}
								dataset={value.dataset}
								filter={filter}
								index={index}
								onChange={updateFilter}
								onRemove={removeFilter}
							/>
						))}
					</div>
				)}
			</div>

			<FieldSection label="Breakdown" hint="Top 50 groups are returned.">
				<Select
					value={value.breakdown ?? NONE}
					onValueChange={(v) =>
						onChange({ ...value, breakdown: v === NONE ? null : v })
					}
				>
					<SelectTrigger className="w-full">
						<SelectValue placeholder="No breakdown" />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value={NONE}>No breakdown</SelectItem>
						{breakdownFields.map((field) => (
							<SelectItem key={field.field} value={field.field}>
								{field.label}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			</FieldSection>

			<div className="grid gap-3 sm:grid-cols-2">
				<FieldSection label="Interval">
					<Select
						value={value.interval ?? "none"}
						onValueChange={(v) =>
							onChange({ ...value, interval: v as ITelemetryQueryInterval })
						}
					>
						<SelectTrigger className="w-full">
							<SelectValue placeholder="Interval" />
						</SelectTrigger>
						<SelectContent>
							{TELEMETRY_QUERY_INTERVALS.map((interval) => (
								<SelectItem key={interval.value} value={interval.value}>
									{interval.label}
								</SelectItem>
							))}
						</SelectContent>
					</Select>
				</FieldSection>
				<FieldSection label="Time range">
					<Select
						value={String(value.hours)}
						onValueChange={(v) =>
							onChange({ ...value, hours: Number.parseInt(v, 10) })
						}
					>
						<SelectTrigger className="w-full">
							<SelectValue placeholder="Time range" />
						</SelectTrigger>
						<SelectContent>
							{TELEMETRY_QUERY_HOUR_OPTIONS.map((option) => (
								<SelectItem key={option.value} value={String(option.value)}>
									{option.label}
								</SelectItem>
							))}
						</SelectContent>
					</Select>
				</FieldSection>
			</div>

			{capWarning ? (
				<div className="flex items-start gap-1.5 rounded-lg border border-amber-500/50 bg-amber-500/10 p-2.5 text-[11px] text-amber-700 dark:text-amber-400">
					<TriangleAlert className="mt-0.5 h-3 w-3 shrink-0" />
					<span>{capWarning}</span>
				</div>
			) : null}

			{errors.length > 0 ? (
				<ul className="space-y-1 rounded-lg border border-dashed border-destructive/50 p-2.5">
					{errors.map((error) => (
						<li
							key={error}
							className="flex items-start gap-1.5 text-[11px] text-destructive"
						>
							<TriangleAlert className="mt-0.5 h-3 w-3 shrink-0" />
							<span>{error}</span>
						</li>
					))}
				</ul>
			) : null}

			<Button
				className="w-full"
				onClick={onRun}
				disabled={running || errors.length > 0}
			>
				<Play className="mr-1 h-3.5 w-3.5" />
				{running ? "Running…" : "Run query"}
			</Button>
		</div>
	);
}
