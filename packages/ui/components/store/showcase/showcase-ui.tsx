"use client";

import { useTranslation } from "@flow-like/locales";
import { Check, Copy, type LucideIcon } from "lucide-react";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useRef,
	useState,
} from "react";
import { cn } from "../../../lib/utils";
import { Button } from "../../ui/button";
import {
	type ColumnFamilyKey,
	FAMILY_ORDER,
	type ShowcaseNodeKind,
	familyVar,
} from "./model";

export const sectionDomId = (key: string) => `showcase-sec-${key}`;
export const ABOUT_DOM_ID = "store-about";
export const REVIEWS_DOM_ID = "store-reviews";

export function scrollToSection(domId: string) {
	const el = document.getElementById(domId);
	if (!el) return;
	const reduced =
		typeof window.matchMedia === "function" &&
		window.matchMedia("(prefers-reduced-motion: reduce)").matches;
	el.scrollIntoView({
		behavior: reduced ? "auto" : "smooth",
		block: "start",
	});
}

export const NODE_TONE: Record<ShowcaseNodeKind, string> = {
	event: "text-primary",
	flow: "text-muted-foreground",
	table: "text-[var(--showcase-text)]",
	source: "text-[var(--showcase-text)]",
	page: "text-tertiary",
	widget: "text-tertiary",
};

export function ShowcaseSection({
	id,
	icon: Icon,
	eyebrow,
	title,
	subtitle,
	aside,
	children,
}: Readonly<{
	id: string;
	icon: LucideIcon;
	eyebrow: string;
	title: string;
	subtitle?: string;
	aside?: ReactNode;
	children: ReactNode;
}>) {
	return (
		<section
			id={sectionDomId(id)}
			data-showcase-section={id}
			className="scroll-mt-16 space-y-5"
		>
			<header className="flex flex-wrap items-end justify-between gap-x-6 gap-y-3">
				<div className="min-w-0">
					<div className="flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-widest text-muted-foreground">
						<Icon className="h-3.5 w-3.5" aria-hidden="true" />
						{eyebrow}
					</div>
					<h2 className="mt-1 text-xl font-semibold tracking-tight">{title}</h2>
					{subtitle ? (
						<p className="mt-1 max-w-prose text-sm text-muted-foreground">
							{subtitle}
						</p>
					) : null}
				</div>
				{aside}
			</header>
			{children}
		</section>
	);
}

export const cardClass =
	"rounded-xl border border-border bg-card p-4 text-left min-w-0";

export const tabsListClass =
	"h-auto w-full justify-start gap-5 rounded-none border-b border-border bg-transparent p-0";

export const tabsTriggerClass =
	"relative h-auto flex-none gap-1.5 rounded-none border-0 bg-transparent px-0 pt-1 pb-2.5 text-[13px] text-muted-foreground shadow-none after:absolute after:inset-x-0 after:bottom-0 after:h-0.5 after:rounded-full hover:text-foreground focus-visible:rounded-sm focus-visible:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60 data-[state=active]:bg-transparent data-[state=active]:text-foreground data-[state=active]:shadow-none data-[state=active]:after:bg-foreground dark:text-muted-foreground dark:data-[state=active]:border-transparent dark:data-[state=active]:bg-transparent dark:data-[state=active]:text-foreground";

export function TabCount({ n }: Readonly<{ n: number }>) {
	return (
		<span className="rounded-full bg-muted px-1.5 text-[10.5px] font-medium tabular-nums text-muted-foreground">
			{n}
		</span>
	);
}

export const gridWrapClass =
	"overflow-auto overscroll-contain rounded-lg border border-border bg-card";
export const gridTableClass =
	"my-0 w-full border-separate border-spacing-0 text-[13px]";
export const gridHeadClass =
	"sticky top-0 z-10 whitespace-nowrap border-0 border-b border-border bg-muted px-3 py-2 text-left text-xs font-medium text-muted-foreground";
export const gridCellClass =
	"border-0 border-b border-border/60 px-3 py-2 align-middle";

export const interactiveCardClass = cn(
	cardClass,
	"w-full transition-[border-color,transform] hover:border-foreground/25 motion-safe:hover:-translate-y-px focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
);

export function Chip({
	icon: Icon,
	children,
	tone = "muted",
	className,
}: Readonly<{
	icon?: LucideIcon;
	children: ReactNode;
	tone?: "muted" | "accent" | "dashed";
	className?: string;
}>) {
	return (
		<span
			className={cn(
				"inline-flex h-5.5 items-center gap-1 whitespace-nowrap rounded-full px-2 text-[11.5px] font-medium",
				tone === "muted" && "bg-muted text-muted-foreground",
				tone === "accent" && "bg-primary/10 text-primary",
				tone === "dashed" &&
					"border border-dashed border-border text-muted-foreground",
				className,
			)}
		>
			{Icon ? <Icon className="h-3 w-3" aria-hidden="true" /> : null}
			{children}
		</span>
	);
}

export interface BarPart {
	key: string;
	n: number;
	color: string;
	label: string;
}

export function CompositionBar({
	parts,
	label,
}: Readonly<{ parts: BarPart[]; label: string }>) {
	return (
		<div className="space-y-2">
			<div className="flex h-2 gap-0.5" role="img" aria-label={label}>
				{parts.map((p) => (
					<span
						key={p.key}
						className="min-w-1 rounded-sm"
						style={{ flex: p.n, background: p.color }}
					/>
				))}
			</div>
			<ul className="flex flex-wrap gap-x-3 gap-y-1 text-[11.5px] text-muted-foreground">
				{parts.map((p) => (
					<li key={p.key} className="inline-flex items-center gap-1.5">
						<span
							className="h-2 w-2 rounded-xs"
							style={{ background: p.color }}
							aria-hidden="true"
						/>
						{p.label} {p.n}
					</li>
				))}
			</ul>
		</div>
	);
}

export function useFamilyLabels(): Record<ColumnFamilyKey, string> {
	const { t } = useTranslation("store");
	return {
		text: t("showcaseFamilyText", "Text"),
		number: t("showcaseFamilyNumber", "Number"),
		time: t("showcaseFamilyTime", "Time"),
		vector: t("showcaseFamilyVector", "Vector"),
		geo: t("showcaseFamilyGeo", "Geo"),
		other: t("showcaseFamilyOther", "Other"),
	};
}

export function familyParts(
	counts: { key: ColumnFamilyKey; n: number }[],
	labels: Record<ColumnFamilyKey, string>,
): BarPart[] {
	return FAMILY_ORDER.flatMap((k) => {
		const hit = counts.find((c) => c.key === k);
		return hit
			? [{ key: k, n: hit.n, color: familyVar(k), label: labels[k] }]
			: [];
	});
}

export function CopyButton({
	getText,
	className,
	variant = "outline",
}: Readonly<{
	getText: () => string;
	className?: string;
	variant?: "outline" | "ghost";
}>) {
	const { t } = useTranslation("store");
	const [copied, setCopied] = useState(false);
	const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
	useEffect(
		() => () => {
			if (timer.current) clearTimeout(timer.current);
		},
		[],
	);
	const copy = useCallback(async () => {
		try {
			await navigator.clipboard.writeText(getText());
			setCopied(true);
			if (timer.current) clearTimeout(timer.current);
			timer.current = setTimeout(() => setCopied(false), 1800);
		} catch {
			setCopied(false);
		}
	}, [getText]);
	return (
		<Button
			type="button"
			variant={variant}
			size="sm"
			className={className}
			onClick={copy}
		>
			{copied ? (
				<Check className="h-3.5 w-3.5" aria-hidden="true" />
			) : (
				<Copy className="h-3.5 w-3.5" aria-hidden="true" />
			)}
			<span aria-live="polite">
				{copied ? t("showcaseCopied", "Copied") : t("showcaseCopy", "Copy")}
			</span>
		</Button>
	);
}

export function CodeBlock({
	code,
	header,
}: Readonly<{ code: string; header?: ReactNode }>) {
	return (
		<div className="overflow-hidden rounded-lg border border-border bg-muted/30">
			{header ? (
				<div className="flex items-center justify-between gap-2 border-b border-border bg-muted/40 py-1 pr-1.5 pl-3">
					{header}
				</div>
			) : null}
			<pre className="overflow-x-auto px-3.5 py-3 font-mono text-[12.5px] leading-relaxed">
				{code}
			</pre>
		</div>
	);
}

export function LockedNote({
	icon: Icon,
	children,
}: Readonly<{ icon: LucideIcon; children: ReactNode }>) {
	return (
		<div className="flex items-start gap-3 rounded-xl border border-dashed border-border p-4 text-sm text-muted-foreground">
			<Icon className="mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
			<p>{children}</p>
		</div>
	);
}
