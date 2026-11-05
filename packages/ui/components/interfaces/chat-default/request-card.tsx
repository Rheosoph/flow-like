"use client";

import { useTranslation } from "@flow-like/locales";
import { ChevronDownIcon, type LucideIcon } from "lucide-react";
import { type ComponentProps, type ReactNode, useState } from "react";
import { cn } from "../../../lib/utils";

export type RequestTone = "warning" | "primary" | "good" | "critical" | "muted";

const MARK_TONES: Record<RequestTone, string> = {
	warning: "border-warning-line/70 bg-warning-bg text-warning",
	primary: "border-primary/25 bg-primary/10 text-primary",
	good: "border-good-line/70 bg-good-bg text-good",
	critical: "border-critical-line/70 bg-critical-bg text-critical",
	muted: "border-border bg-muted text-muted-foreground",
};

const DOT_TONES: Record<RequestTone, string> = {
	warning: "bg-warning-solid",
	primary: "bg-primary",
	good: "bg-good-solid",
	critical: "bg-critical-solid",
	muted: "bg-muted-foreground/50",
};

const BADGE_TONES: Record<RequestTone, string> = {
	warning: "bg-warning-solid text-card",
	primary: "bg-primary text-primary-foreground",
	good: "bg-good-solid text-card",
	critical: "bg-critical-solid text-card",
	muted: "bg-muted-foreground text-card",
};

const ACCENT_TONES: Record<RequestTone, string> = {
	warning: "via-warning-solid",
	primary: "via-primary",
	good: "via-good-solid",
	critical: "via-critical-solid",
	muted: "via-border",
};

/**
 * The shell every request in a chat shares: a tool approval, a question, a flow's interaction, a
 * bot's exchange. `live` marks one that still waits for the user with a hairline in its tone.
 */
export function RequestCard({
	tone = "muted",
	live = false,
	className,
	children,
	...props
}: ComponentProps<"section"> & { tone?: RequestTone; live?: boolean }) {
	return (
		<section
			data-tone={tone}
			className={cn(
				"relative flex w-full min-w-0 flex-col overflow-hidden rounded-xl border bg-card text-card-foreground shadow-sm",
				className,
			)}
			style={{ borderColor: "var(--fl-chat-rule-strong, var(--border))" }}
			{...props}
		>
			{live && (
				<span
					aria-hidden
					className={cn(
						"pointer-events-none absolute inset-x-6 top-0 h-px bg-linear-to-r from-transparent to-transparent",
						ACCENT_TONES[tone],
					)}
				/>
			)}
			{children}
		</section>
	);
}

/** An icon tile in the request's tone, or a face (a bot's avatar) with the icon as its badge. */
export function RequestMark({
	icon: Icon,
	tone = "muted",
	size = "md",
	children,
}: Readonly<{
	icon: LucideIcon;
	tone?: RequestTone;
	size?: "sm" | "md";
	children?: ReactNode;
}>) {
	const box = size === "sm" ? "size-8" : "size-9";
	if (!children) {
		return (
			<span
				className={cn(
					"flex shrink-0 items-center justify-center rounded-xl border",
					box,
					MARK_TONES[tone],
				)}
			>
				<Icon className="size-4" aria-hidden />
			</span>
		);
	}
	return (
		<span
			className={cn(
				"relative flex shrink-0 items-center justify-center [&_.fl-bot-badges]:hidden",
				box,
			)}
		>
			{children}
			<span
				className={cn(
					"absolute -right-1 -bottom-1 flex size-4.5 items-center justify-center rounded-full ring-2 ring-card",
					BADGE_TONES[tone],
				)}
			>
				<Icon className="size-3" strokeWidth={2.5} aria-hidden />
			</span>
		</span>
	);
}

export function RequestCardHeader({
	mark,
	eyebrow,
	tone = "muted",
	live = false,
	title,
	description,
	aside,
	dense = false,
	className,
}: Readonly<{
	mark?: ReactNode;
	/** Who asks and what for, above the title. */
	eyebrow?: ReactNode;
	tone?: RequestTone;
	/** Pulses the eyebrow's dot while the request waits. */
	live?: boolean;
	title: ReactNode;
	description?: ReactNode;
	aside?: ReactNode;
	/** The smaller header of a card that sits in the transcript rather than above the composer. */
	dense?: boolean;
	className?: string;
}>) {
	return (
		<header
			className={cn(
				"flex items-start gap-3",
				dense ? "px-3.5 pt-3 pb-2.5" : "px-4 pt-4 pb-3",
				className,
			)}
		>
			{mark}
			<div className={cn("min-w-0 flex-1", dense && !eyebrow && "self-center")}>
				{eyebrow && (
					<p className="flex min-w-0 items-center gap-1.5 text-xs font-medium text-muted-foreground">
						<span
							aria-hidden
							className={cn(
								"size-1.5 shrink-0 rounded-full",
								DOT_TONES[tone],
								live && "animate-pulse-soft",
							)}
						/>
						<span className="min-w-0 truncate">{eyebrow}</span>
					</p>
				)}
				<h3
					className={cn(
						"font-medium leading-snug tracking-tight text-pretty wrap-break-word text-foreground",
						dense ? "text-sm" : "text-[15px]",
						eyebrow && "mt-1",
					)}
				>
					{title}
				</h3>
				{description && (
					<div className="mt-1 text-[13px] leading-relaxed text-muted-foreground">
						{description}
					</div>
				)}
			</div>
			{aside && (
				<div className="flex shrink-0 items-center gap-1.5 pt-0.5">{aside}</div>
			)}
		</header>
	);
}

export function RequestCardBody({
	dense = false,
	className,
	...props
}: ComponentProps<"div"> & { dense?: boolean }) {
	return (
		<div
			className={cn(
				"min-h-0",
				dense ? "space-y-2.5 px-3.5 pb-3.5" : "space-y-3 px-4 pb-4",
				className,
			)}
			{...props}
		/>
	);
}

/** The action row: an optional note or option on the left, the buttons on the right. */
export function RequestCardFooter({
	start,
	children,
	className,
}: Readonly<{ start?: ReactNode; children: ReactNode; className?: string }>) {
	return (
		<footer
			className={cn(
				"flex shrink-0 flex-wrap items-center gap-x-3 gap-y-2 border-t bg-muted/30 px-4 py-3",
				className,
			)}
			style={{ borderColor: "var(--fl-chat-rule, var(--border))" }}
		>
			<div className="min-w-0 flex-1">{start}</div>
			<div className="ml-auto flex shrink-0 items-center gap-2">{children}</div>
		</footer>
	);
}

/** A small rounded tag for the header's aside: a countdown, a status. */
export function RequestPill({
	tone = "muted",
	icon: Icon,
	children,
	className,
}: Readonly<{
	tone?: RequestTone;
	icon?: LucideIcon;
	children: ReactNode;
	className?: string;
}>) {
	return (
		<span
			className={cn(
				"inline-flex h-6 items-center gap-1 rounded-full border px-2 text-[11px] font-medium tabular-nums",
				MARK_TONES[tone],
				className,
			)}
		>
			{Icon && <Icon className="size-3" aria-hidden />}
			{children}
		</span>
	);
}

const LONG_DETAILS = 220;

/** A request's own words, three lines at first; a long one opens in place and scrolls. */
export function RequestDetails({ text }: Readonly<{ text: string }>) {
	const { t } = useTranslation("chat");
	const [open, setOpen] = useState(false);
	const long = text.length > LONG_DETAILS || text.split("\n").length > 3;
	return (
		<div className="rounded-lg bg-muted/50 px-3 py-2.5">
			<p
				className={cn(
					"whitespace-pre-wrap wrap-break-word text-xs leading-relaxed text-muted-foreground",
					long && !open && "line-clamp-3",
					open && "max-h-56 overflow-y-auto overscroll-contain",
				)}
			>
				{text}
			</p>
			{long && (
				<button
					type="button"
					aria-expanded={open}
					onClick={() => setOpen((value) => !value)}
					className="mt-1.5 inline-flex items-center gap-1 rounded text-xs font-medium text-foreground/75 outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-primary/40"
				>
					{open ? t("showLess", "Show less") : t("showMore", "Show more")}
					<ChevronDownIcon
						className={cn("size-3 transition-transform", open && "rotate-180")}
						aria-hidden
					/>
				</button>
			)}
		</div>
	);
}

/**
 * One selectable answer: the whole row is the control's label, so a click anywhere toggles it
 * once and the row never nests a button in a button.
 */
export function RequestChoice({
	htmlFor,
	active,
	disabled,
	control,
	label,
	description,
	size = "md",
}: Readonly<{
	htmlFor: string;
	active: boolean;
	disabled?: boolean;
	control: ReactNode;
	label: ReactNode;
	description?: ReactNode;
	size?: "md" | "lg";
}>) {
	return (
		<label
			htmlFor={htmlFor}
			data-active={active || undefined}
			className={cn(
				"flex w-full cursor-pointer items-start gap-3 rounded-lg border text-left transition-colors",
				size === "lg" ? "px-3.5 py-3" : "px-3 py-2.5",
				active
					? "border-primary/45 bg-primary/5"
					: "border-border/70 bg-background/60 hover:border-border hover:bg-muted/40",
				disabled && "pointer-events-none opacity-60",
			)}
		>
			{control}
			<span className="min-w-0 flex-1">
				<span className="block text-sm font-medium leading-snug">{label}</span>
				{description && (
					<span className="mt-0.5 block text-xs leading-relaxed text-muted-foreground">
						{description}
					</span>
				)}
			</span>
		</label>
	);
}

export const requestButtonClass =
	"h-8 rounded-lg px-3.5 outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0";
