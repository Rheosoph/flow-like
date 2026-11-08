"use client";
import { useTranslation } from "@flow-like/locales";
import { useEffect, useState } from "react";
import { useUserLookup } from "../../hooks/use-user-lookup";
import {
	type PublicAvailability,
	effectiveAvailability,
} from "../../lib/availability";
import { cn } from "../../lib/utils";

const COLORS = {
	available: "bg-emerald-500",
	busy: "bg-amber-500",
	focus: "bg-violet-500",
	away: "bg-slate-400",
};
export function AvailabilityBadge({
	value,
	dot = false,
	className,
}: { value?: PublicAvailability; dot?: boolean; className?: string }) {
	const { t } = useTranslation("bots");
	const [now, setNow] = useState(Date.now);
	useEffect(() => {
		if (!value?.until) return;
		setNow(Date.now());
		const timer = setTimeout(
			() => setNow(Date.now()),
			Math.max(0, value.until - Date.now()),
		);
		return () => clearTimeout(timer);
	}, [value?.until]);
	if (!value || (value.until !== null && value.until <= now)) return null;
	const current = effectiveAvailability(value, now);
	const label = t(`availability.status.${current.status}`, current.status);
	return (
		<span
			title={label}
			aria-label={label}
			className={cn("inline-flex items-center gap-1.5 text-xs", className)}
		>
			<span
				aria-hidden
				className={cn(
					"size-2 shrink-0 rounded-full ring-2 ring-background",
					COLORS[current.status],
				)}
			/>
			{!dot && <span>{label}</span>}
		</span>
	);
}

export function UserAvailabilityBadge({
	subject,
	dot = false,
	className,
}: { subject: string; dot?: boolean; className?: string }) {
	const lookup = useUserLookup(subject);
	return (
		<AvailabilityBadge
			value={lookup.data?.availability}
			dot={dot}
			className={className}
		/>
	);
}
