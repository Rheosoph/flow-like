"use client";

import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { cx } from "./tone";

export interface SegmentOption<T extends string> {
	value: T;
	label: ReactNode;
	icon?: LucideIcon;
	count?: number;
	disabled?: boolean;
}

/** SPEC §4.36 segmented control: `aria-pressed` buttons in a group; the pressed one is inverted. */
export function Segmented<T extends string>({
	label,
	options,
	value,
	onChange,
	size = "md",
	wrap = false,
	className,
}: Readonly<{
	/** Accessible name of the group ("View"). */
	label: string;
	options: readonly SegmentOption<T>[];
	value: T;
	onChange(value: T): void;
	size?: "md" | "sm";
	wrap?: boolean;
	className?: string;
}>) {
	return (
		<fieldset
			aria-label={label}
			className={cx(
				"m-0 inline-flex max-w-full min-w-0 gap-0.5 rounded-lg border border-border bg-muted p-0.5 align-middle",
				wrap && "flex-wrap",
				className,
			)}
		>
			{options.map((option) => {
				const Icon = option.icon;
				const pressed = option.value === value;
				return (
					<button
						key={option.value}
						type="button"
						aria-pressed={pressed}
						disabled={option.disabled}
						onClick={() => onChange(option.value)}
						className={cx(
							"inline-flex items-center gap-1.5 rounded-md font-medium whitespace-nowrap focus-visible:outline-2 focus-visible:outline-ring disabled:cursor-not-allowed disabled:opacity-50",
							size === "sm" ? "h-5.5 px-2 text-xs" : "h-6.5 px-2.5 text-ui",
							pressed
								? "bg-foreground text-background"
								: "text-ink-2 hover:bg-row-hover hover:text-foreground",
						)}
					>
						{Icon ? <Icon aria-hidden className="size-3.5" /> : null}
						{option.label}
						{option.count !== undefined ? (
							<span className="font-mono text-xs tabular-nums opacity-80">
								{option.count}
							</span>
						) : null}
					</button>
				);
			})}
		</fieldset>
	);
}

/** Separates label and count in the accessible name ("Expired 1", not "Expired1"); a flex row doesn't paint it. */
const NAME_GAP = " ";

/** SPEC §4.36 filter chip: a pill toggle. */
export function FilterChip({
	pressed,
	onPressedChange,
	icon: Icon,
	count,
	className,
	children,
}: Readonly<{
	pressed: boolean;
	onPressedChange(pressed: boolean): void;
	icon?: LucideIcon;
	count?: number;
	className?: string;
	children: ReactNode;
}>) {
	return (
		<button
			type="button"
			aria-pressed={pressed}
			onClick={() => onPressedChange(!pressed)}
			className={cx(
				"inline-flex h-6.5 items-center gap-1 rounded-full border px-2.5 text-xs font-medium whitespace-nowrap focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring",
				pressed
					? "border-foreground bg-foreground text-background"
					: "border-border bg-card text-ink-2 hover:border-border-strong hover:text-foreground",
				className,
			)}
		>
			{Icon ? <Icon aria-hidden className="size-3" /> : null}
			{children}
			{count !== undefined ? (
				<>
					{NAME_GAP}
					<span className="font-mono tabular-nums opacity-75">{count}</span>
				</>
			) : null}
		</button>
	);
}
