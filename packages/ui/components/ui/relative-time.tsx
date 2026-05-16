"use client";

import type { ComponentPropsWithoutRef } from "react";
import {
	type DateValue,
	formatAbsoluteDateValue,
	formatRelativeDateValue,
	parseDateValue,
} from "../../lib/date";
import { cn } from "../../lib/utils";

export interface RelativeTimeProps extends ComponentPropsWithoutRef<"span"> {
	value: DateValue;
	fallback?: string;
	absoluteFormat?: string;
}

export function RelativeTime({
	absoluteFormat = "PPp",
	className,
	fallback = "Unknown",
	value,
	...props
}: RelativeTimeProps) {
	const parsed = parseDateValue(value);
	const relativeLabel = formatRelativeDateValue(value, fallback);
	const absoluteLabel = parsed
		? formatAbsoluteDateValue(parsed, fallback, absoluteFormat)
		: undefined;

	return (
		<span className={cn(className)} title={absoluteLabel} {...props}>
			{relativeLabel}
		</span>
	);
}
