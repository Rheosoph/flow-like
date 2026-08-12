"use client";

import type { ComponentPropsWithoutRef, ReactNode } from "react";
import { cn } from "../../lib/utils";

export type FlowNodeShellTone = "primary" | "tertiary" | "neutral";
export type FlowNodeShellState = "idle" | "running" | "complete";

export interface FlowNodeShellProps extends ComponentPropsWithoutRef<"div"> {
	label: string;
	description?: string;
	kind?: string;
	icon?: ReactNode;
	tone?: FlowNodeShellTone;
	state?: FlowNodeShellState;
	selected?: boolean;
	inputPins?: number;
	outputPins?: number;
}

const headerTone: Record<FlowNodeShellTone, string> = {
	primary:
		"from-card via-rose-300/45 to-rose-300/80 dark:via-primary/45 dark:to-primary/80",
	tertiary:
		"from-card via-emerald-300/45 to-emerald-300/80 dark:via-tertiary/45 dark:to-tertiary/80",
	neutral: "from-card via-muted to-muted",
};

function Pins({
	side,
	count,
}: Readonly<{ side: "input" | "output"; count: number }>) {
	if (count <= 0) return null;
	const pinIds = ["one", "two", "three", "four", "five", "six"].slice(0, count);
	return pinIds.map((pinId, index) => (
		<span
			aria-hidden="true"
			className={cn(
				"absolute z-10 size-2.5 rounded-full border-2 border-card bg-sky-400 shadow-[0_0_0_1px_color-mix(in_oklch,var(--border)_80%,transparent)]",
				side === "input" ? "-left-1.5" : "-right-1.5",
			)}
			key={`${side}-${pinId}`}
			style={{ top: `${34 + index * 16}px` }}
		/>
	));
}

/**
 * Provider-free presentation shell that mirrors the visual anatomy of a Flow-Like node.
 * It is intentionally stateless so documentation, marketing, and loading surfaces can use
 * authentic product UI without pulling the full board/runtime dependency graph.
 */
export function FlowNodeShell({
	label,
	description,
	kind,
	icon,
	tone = "neutral",
	state = "idle",
	selected = false,
	inputPins = 1,
	outputPins = 1,
	className,
	children,
	...props
}: Readonly<FlowNodeShellProps>) {
	return (
		<div
			className={cn(
				"relative min-h-20 rounded-md border bg-card px-3 pb-2 pt-7 text-card-foreground shadow-sm transition-[border-color,box-shadow,opacity,transform]",
				selected &&
					"-translate-y-0.5 border-primary shadow-[0_0_0_3px_color-mix(in_oklch,var(--primary)_12%,transparent),0_14px_30px_color-mix(in_srgb,black_16%,transparent)]",
				state === "complete" && "opacity-65",
				className,
			)}
			data-slot="flow-node-shell"
			data-state={state}
			{...props}
		>
			<div
				className={cn(
					"absolute inset-x-0 top-0 flex h-5 items-center gap-1 rounded-t-[inherit] border-b border-foreground/35 bg-linear-to-r px-1.5",
					headerTone[tone],
				)}
			>
				<span className="grid size-2.5 shrink-0 place-items-center text-[8px]">
					{icon}
				</span>
				<strong className="min-w-0 flex-1 truncate text-[9px] font-semibold leading-none">
					{label}
				</strong>
				{state === "running" && (
					<span className="size-2 animate-pulse rounded-full border border-foreground/60 bg-background/80" />
				)}
				{state === "complete" && (
					<span className="text-[9px] font-bold text-foreground">✓</span>
				)}
			</div>
			<Pins count={inputPins} side="input" />
			<Pins count={outputPins} side="output" />
			{kind && (
				<p className="truncate font-mono text-[8px] uppercase tracking-[0.12em] text-muted-foreground">
					{kind}
				</p>
			)}
			{description && (
				<p className="mt-1 line-clamp-2 text-[10px] leading-snug text-muted-foreground">
					{description}
				</p>
			)}
			{children}
		</div>
	);
}
