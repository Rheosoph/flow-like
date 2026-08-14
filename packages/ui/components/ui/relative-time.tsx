"use client";

import { useMemo } from "react";
import {
	type DateValue,
	formatAbsoluteDateTime,
	formatRelativeTime,
	parseTemporalValue,
} from "../../lib/date";
import { Tooltip, TooltipContent, TooltipTrigger } from "./tooltip";

export interface RelativeTimeProps {
	/** Anything a temporal column can hold — ISO text, a Date, or an epoch number. */
	value: unknown;
	className?: string;
	/** Rendered when the value is not an instant; defaults to the raw value. */
	fallback?: React.ReactNode;
	style?: Intl.RelativeTimeFormatStyle;
}

/**
 * A timestamp read as "2 days ago", with the exact instant on hover.
 *
 * Every temporal surface shares this so a value never reads one way in a table
 * and another in the drawer that opens from it.
 */
export function RelativeTime({
	value,
	className,
	fallback,
	style = "long",
}: RelativeTimeProps) {
	const parsed = useMemo(() => parseTemporalValue(value), [value]);

	if (!parsed) {
		return (
			<span className={className}>
				{fallback ?? (value == null ? "" : String(value))}
			</span>
		);
	}

	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<span className={className}>
					{formatRelativeTime(parsed as DateValue, style)}
				</span>
			</TooltipTrigger>
			<TooltipContent side="bottom" className="text-xs">
				{formatAbsoluteDateTime(parsed as DateValue)}
			</TooltipContent>
		</Tooltip>
	);
}
