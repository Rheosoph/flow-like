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
	children,
}: Readonly<{
	id: string;
	icon: LucideIcon;
	eyebrow: string;
	title: string;
	subtitle?: string;
	children: ReactNode;
}>) {
	return (
		<section
			id={sectionDomId(id)}
			data-showcase-section={id}
			className="scroll-mt-16 space-y-5"
		>
			<header>
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
			</header>
			{children}
		</section>
	);
}

export const cardClass =
	"rounded-xl border border-border bg-card p-4 text-left min-w-0";

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
}: Readonly<{ getText: () => string; className?: string }>) {
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
			variant="outline"
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

export function CodeBlock({ code }: Readonly<{ code: string }>) {
	return (
		<pre className="overflow-x-auto rounded-md border border-border bg-muted/40 px-3.5 py-3 font-mono text-[12.5px] leading-relaxed">
			{code}
		</pre>
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
