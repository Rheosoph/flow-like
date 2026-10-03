"use client";

import type { ReactNode } from "react";
import { Tabs, TabsList, TabsTrigger } from "../../../ui/tabs";
import { TONE_CHIP, cx } from "./tone";

export type TabCountTone = "critical" | "warning" | "info" | "neutral";

/** Count badge on a tab: critical is square-cornered, the number is mono (SPEC §4.36). */
export function TabCount({
	count,
	tone = "neutral",
	label,
	className,
}: Readonly<{
	count: number;
	tone?: TabCountTone;
	/** Accessible meaning of the number ("3 need attention"). */
	label?: string;
	className?: string;
}>) {
	return (
		<span
			data-count-tone={tone}
			className={cx(
				"inline-flex h-4.5 min-w-4.5 items-center justify-center rounded-full border px-1.25 font-mono text-xs font-medium tabular-nums",
				tone === "neutral"
					? "border-border bg-muted text-ink-2"
					: TONE_CHIP[tone],
				className,
			)}
		>
			{label ? (
				<>
					<span aria-hidden>{count}</span>
					<span className="sr-only">{label}</span>
				</>
			) : (
				count
			)}
		</span>
	);
}

export interface UnderlineTab<T extends string> {
	value: T;
	label: ReactNode;
	/** Shorter label for narrow containers. */
	shortLabel?: ReactNode;
	count?: { count: number; tone?: TabCountTone; label?: string };
	disabled?: boolean;
}

interface UnderlineTabListProps<T extends string> {
	/** Accessible name of the tab list ("Device sections"). */
	label: string;
	tabs: readonly UnderlineTab<T>[];
	/** Stick under the area top bar (N2/N3). */
	sticky?: boolean;
	listClassName?: string;
}

export interface UnderlineTabsProps<T extends string>
	extends UnderlineTabListProps<T> {
	value: T;
	onValueChange(value: T): void;
	className?: string;
	children?: ReactNode;
}

function UnderlineTabTrigger<T extends string>({
	tab,
}: Readonly<{ tab: UnderlineTab<T> }>) {
	return (
		<TabsTrigger
			value={tab.value}
			disabled={tab.disabled}
			className="-mb-px h-10 flex-none gap-1.5 rounded-none border-0 border-b-2 border-transparent bg-transparent px-3 text-[13px]/[18px] font-medium text-muted-foreground shadow-none hover:text-foreground focus-visible:ring-0 focus-visible:outline-2 focus-visible:-outline-offset-2 data-[state=active]:border-foreground data-[state=active]:bg-transparent data-[state=active]:text-foreground data-[state=active]:shadow-none dark:text-muted-foreground dark:data-[state=active]:border-foreground dark:data-[state=active]:bg-transparent dark:data-[state=active]:text-foreground"
		>
			{tab.shortLabel ? (
				<>
					<span className="@max-[560px]/devices:hidden">{tab.label}</span>
					<span className="hidden @max-[560px]/devices:inline">
						{tab.shortLabel}
					</span>
				</>
			) : (
				tab.label
			)}
			{tab.count ? (
				<TabCount
					count={tab.count.count}
					tone={tab.count.tone}
					label={tab.count.label}
				/>
			) : null}
		</TabsTrigger>
	);
}

function UnderlineTabList<T extends string>({
	label,
	tabs,
	sticky = false,
	listClassName,
}: Readonly<UnderlineTabListProps<T>>) {
	return (
		<TabsList
			aria-label={label}
			className={cx(
				"no-scrollbar flex h-auto w-full max-w-full justify-start gap-0.5 overflow-x-auto rounded-none border-b border-border bg-transparent p-0 text-muted-foreground",
				sticky && "sticky top-0 z-20 bg-background",
				listClassName,
			)}
		>
			{tabs.map((tab) => (
				<UnderlineTabTrigger key={tab.value} tab={tab} />
			))}
		</TabsList>
	);
}

/**
 * SPEC §4.36 tabs on Radix Tabs (arrow keys, roving focus): 2 px foreground
 * underline, no pill or shadow. Panels are `TabsContent` children.
 */
export function UnderlineTabs<T extends string>({
	value,
	onValueChange,
	className,
	children,
	...list
}: Readonly<UnderlineTabsProps<T>>) {
	return (
		<Tabs
			value={value}
			onValueChange={(next) => onValueChange(next as T)}
			className={cx("gap-0", className)}
		>
			<UnderlineTabList {...list} />
			{children}
		</Tabs>
	);
}
