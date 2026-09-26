"use client";

import { useTranslation } from "@flow-like/locales";
import { Clock, X } from "lucide-react";
import type * as React from "react";
import { useCallback, useMemo } from "react";
import {
	formatAbsoluteDateTime,
	formatCalendarDate,
	formatRelativeTime,
	fromDateInputValue,
	fromDateTimeInputValue,
	localTimeZoneLabel,
	parseTemporalValue,
	toDateInputValue,
	toDateTimeInputValue,
	toEpochNumber,
} from "../../lib/date";
import { Button } from "./button";
import { Input } from "./input";
import type { LanceTemporalUnit } from "./lance-viewer";

export interface TemporalCell {
	/** The unit the stored number counts in, and the one an edit writes back. */
	unit: LanceTemporalUnit;
	/** The storage shape of the column, which an edit has to keep. */
	wire: "number" | "string";
}

/**
 * Edits an instant as a wall-clock date and time instead of as the epoch integer
 * on disk, and writes it back in the column's own shape and unit.
 */
export const TemporalValueEditor: React.FC<{
	value: string;
	temporal: TemporalCell;
	nullable?: boolean;
	onChange: (value: string) => void;
	disabled?: boolean;
	invalid?: boolean;
	"aria-label"?: string;
	"aria-describedby"?: string;
}> = ({
	value,
	temporal,
	nullable,
	onChange,
	disabled,
	invalid,
	"aria-label": ariaLabel,
	"aria-describedby": ariaDescribedBy,
}) => {
	const { t } = useTranslation("common");
	const timeZone = useMemo(() => localTimeZoneLabel(), []);
	const dayPrecision = temporal.unit === "day";

	const date = useMemo(() => {
		let parsed: unknown;
		try {
			parsed = JSON.parse(value);
		} catch {
			parsed = value;
		}
		return parseTemporalValue(parsed, temporal.unit);
	}, [value, temporal.unit]);

	const emit = useCallback(
		(next: Date | null) => {
			if (!next) {
				onChange("null");
				return;
			}
			onChange(
				JSON.stringify(
					temporal.wire === "string"
						? next.toISOString()
						: toEpochNumber(next, temporal.unit),
				),
			);
		},
		[onChange, temporal.unit, temporal.wire],
	);

	return (
		<div className="space-y-3">
			<div className="flex flex-wrap items-center gap-2">
				<Input
					type={dayPrecision ? "date" : "datetime-local"}
					step={dayPrecision ? undefined : 1}
					className="w-auto"
					disabled={disabled}
					aria-label={ariaLabel}
					aria-invalid={invalid || undefined}
					aria-describedby={ariaDescribedBy}
					value={
						date
							? dayPrecision
								? toDateInputValue(date)
								: toDateTimeInputValue(date)
							: ""
					}
					onChange={(e) => {
						const next = dayPrecision
							? fromDateInputValue(e.target.value)
							: fromDateTimeInputValue(e.target.value);
						if (next || !e.target.value) emit(next);
					}}
				/>
				<Button
					variant="outline"
					size="sm"
					disabled={disabled}
					onClick={() => {
						const now = new Date();
						// A day column stores the calendar day the viewer is in, not the
						// instant, which would round to tomorrow past midday in the east.
						emit(
							dayPrecision
								? (fromDateInputValue(toDateTimeInputValue(now).slice(0, 10)) ??
										now)
								: now,
						);
					}}
				>
					<Clock className="h-3.5 w-3.5 mr-2" /> {t("now", "Now")}
				</Button>
				{nullable !== false && date && (
					<Button
						variant="ghost"
						size="sm"
						disabled={disabled}
						onClick={() => emit(null)}
					>
						<X className="h-3.5 w-3.5 mr-2" /> {t("clear", "Clear")}
					</Button>
				)}
			</div>
			<div className="rounded-md border bg-muted/40 px-3 py-2 space-y-1">
				{date ? (
					<>
						<p className="text-sm">
							{dayPrecision
								? formatCalendarDate(date, "full")
								: formatAbsoluteDateTime(date)}
						</p>
						<p className="text-xs text-muted-foreground">
							{formatRelativeTime(date, "long")}
							{!dayPrecision && timeZone ? ` · ${timeZone}` : ""}
						</p>
					</>
				) : (
					<p className="text-sm text-muted-foreground">NULL</p>
				)}
				<code className="block text-[11px] text-muted-foreground">{value}</code>
			</div>
		</div>
	);
};
