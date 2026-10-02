"use client";

import { useTranslation } from "@flow-like/locales";
import type { LucideIcon } from "lucide-react";
import { Fragment, type MouseEvent, type ReactNode, useId } from "react";
import {
	Breadcrumb,
	BreadcrumbItem,
	BreadcrumbLink,
	BreadcrumbList,
	BreadcrumbPage,
	BreadcrumbSeparator,
} from "../../../ui/breadcrumb";
import { cx } from "./tone";

export interface BlockProps {
	title: ReactNode;
	icon?: LucideIcon;
	count?: number;
	/** Short muted summary after the title ("3 of 5 devices"). */
	summary?: ReactNode;
	/** The block's FreshnessStamp (R5). */
	stamp?: ReactNode;
	tools?: ReactNode;
	/** A filter/search row between head and body. */
	toolbar?: ReactNode;
	foot?: ReactNode;
	/** No body padding (tables, lists). */
	flush?: boolean;
	/** Danger zone outline. */
	danger?: boolean;
	id?: string;
	className?: string;
	bodyClassName?: string;
	children?: ReactNode;
}

/** SPEC §4.25: card with a head (title, count, stamp, tools), body and optional sunken foot. */
export function Block({
	title,
	icon: Icon,
	count,
	summary,
	stamp,
	tools,
	toolbar,
	foot,
	flush = false,
	danger = false,
	id,
	className,
	bodyClassName,
	children,
}: Readonly<BlockProps>) {
	const headingId = useId();
	return (
		<section
			id={id}
			aria-labelledby={headingId}
			data-block=""
			className={cx(
				"min-w-0 overflow-clip rounded-lg border bg-card",
				danger ? "border-critical-line" : "border-border",
				className,
			)}
		>
			<header className="flex min-h-12 flex-wrap items-center gap-x-3 gap-y-2 border-b border-hairline px-4 py-3">
				<h2
					id={headingId}
					className="inline-flex min-w-0 items-center gap-2 text-[15px]/5 font-semibold tracking-[-0.005em] text-foreground"
				>
					{Icon ? (
						<Icon
							aria-hidden
							className={cx(
								"size-4 shrink-0",
								danger ? "text-critical" : "text-ink-2",
							)}
						/>
					) : null}
					<span className="min-w-0">{title}</span>
					{count !== undefined ? (
						<span className="inline-flex h-5 min-w-5.5 items-center justify-center rounded-full bg-muted px-1.5 font-mono text-xs font-medium tabular-nums text-ink-2">
							{count}
						</span>
					) : null}
				</h2>
				{summary ? (
					<span className="text-xs text-muted-foreground">{summary}</span>
				) : null}
				<span className="flex-1" />
				{stamp}
				{tools ? (
					<div className="flex flex-wrap items-center gap-2">{tools}</div>
				) : null}
			</header>
			{toolbar ? (
				<div className="flex flex-wrap items-center gap-x-2.5 gap-y-2 border-b border-hairline px-4 py-2.5">
					{toolbar}
				</div>
			) : null}
			<div
				className={cx(
					"flex min-w-0 flex-col",
					flush ? "gap-0 p-0" : "gap-3 px-4 py-3",
					bodyClassName,
				)}
			>
				{children}
			</div>
			{foot ? (
				<footer className="flex flex-wrap items-center gap-x-3 gap-y-2 border-t border-hairline bg-surface-sunken px-4 py-2.5 text-xs text-muted-foreground">
					{foot}
				</footer>
			) : null}
		</section>
	);
}

export interface Crumb {
	label: ReactNode;
	href?: string;
	/** Client-side navigation; called instead of following `href`. */
	onNavigate?: () => void;
}

export interface ObjectFact {
	id: string;
	label: ReactNode;
	value: ReactNode;
}

export interface ObjectHeaderProps {
	crumbs?: readonly Crumb[];
	glyph?: ReactNode;
	/** The device or service name (rendered mono). */
	name: ReactNode;
	nameChips?: ReactNode;
	chips?: ReactNode;
	facts?: readonly ObjectFact[];
	/** Trailing pieces of the facts line (IdRef, lane). */
	extra?: ReactNode;
	actions?: ReactNode;
	className?: string;
}

/** On a phone the one primary comes first and takes the full width. */
function HeaderActions({ actions }: Readonly<{ actions?: ReactNode }>) {
	if (!actions) return null;
	return (
		<div className="flex flex-wrap items-center gap-2 @max-[720px]/devices:[&>[data-dv-primary]]:order-first @max-[720px]/devices:[&>[data-dv-primary]]:basis-full">
			{actions}
		</div>
	);
}

/** A link when it has an `href`, a button when it only navigates client-side, plain text otherwise. */
function CrumbTarget({ crumb }: Readonly<{ crumb: Crumb }>) {
	const { href, onNavigate, label } = crumb;
	if (!href && !onNavigate) return <span>{label}</span>;
	if (!href) {
		return (
			<BreadcrumbLink asChild className="cursor-pointer hover:underline">
				<button type="button" onClick={onNavigate}>
					{label}
				</button>
			</BreadcrumbLink>
		);
	}
	return (
		<BreadcrumbLink
			href={href}
			onClick={
				onNavigate
					? (event: MouseEvent<HTMLAnchorElement>) => {
							if (event.metaKey || event.ctrlKey || event.shiftKey) return;
							event.preventDefault();
							onNavigate();
						}
					: undefined
			}
			className="hover:underline"
		>
			{label}
		</BreadcrumbLink>
	);
}

function Crumbs({ crumbs }: Readonly<{ crumbs: readonly Crumb[] }>) {
	const { t } = useTranslation("devices");
	if (!crumbs.length) return null;
	const items = crumbs.map((crumb, index) => ({
		crumb,
		key: `${index}:${crumb.href ?? ""}`,
		last: index === crumbs.length - 1,
	}));
	return (
		<Breadcrumb aria-label={t("common.breadcrumb", "Breadcrumb")}>
			<BreadcrumbList className="gap-1 text-xs sm:gap-1">
				{items.map(({ crumb, key, last }) => (
					<Fragment key={key}>
						<BreadcrumbItem>
							{last ? (
								<BreadcrumbPage className="text-ink-2">
									{crumb.label}
								</BreadcrumbPage>
							) : (
								<CrumbTarget crumb={crumb} />
							)}
						</BreadcrumbItem>
						{last ? null : <BreadcrumbSeparator className="[&>svg]:size-3" />}
					</Fragment>
				))}
			</BreadcrumbList>
		</Breadcrumb>
	);
}

/** SPEC §4.25 `.phead`: crumbs, title + sub, and the page actions (one primary at most, R2). */
export function PageHeader({
	crumbs = [],
	title,
	sub,
	actions,
	className,
}: Readonly<{
	crumbs?: readonly Crumb[];
	title: ReactNode;
	sub?: ReactNode;
	actions?: ReactNode;
	className?: string;
}>) {
	return (
		<header className={cx("flex flex-col gap-3", className)}>
			<Crumbs crumbs={crumbs} />
			<div className="flex flex-wrap items-end justify-between gap-x-6 gap-y-3 @max-[720px]/devices:flex-col @max-[720px]/devices:items-stretch">
				<div className="min-w-0">
					<h1 className="text-2xl/[30px] font-semibold tracking-[-0.015em]">
						{title}
					</h1>
					{sub ? (
						<p className="mt-0.5 text-ui text-muted-foreground">{sub}</p>
					) : null}
				</div>
				<HeaderActions actions={actions} />
			</div>
		</header>
	);
}

function ObjectFacts({
	facts,
	extra,
}: Readonly<{ facts: readonly ObjectFact[]; extra?: ReactNode }>) {
	if (!facts.length && !extra) return null;
	return (
		<div className="mt-2.5 flex flex-wrap items-center gap-x-4.5 gap-y-1.5 text-xs text-ink-2">
			{facts.map((fact) => (
				<span key={fact.id} className="inline-flex min-w-0 items-center gap-1">
					<span className="whitespace-nowrap text-muted-foreground">
						{fact.label}
					</span>
					{fact.value}
				</span>
			))}
			{extra}
		</div>
	);
}

function ObjectIdentity({
	glyph,
	name,
	nameChips,
	chips,
	facts = [],
	extra,
}: Readonly<Omit<ObjectHeaderProps, "crumbs" | "actions" | "className">>) {
	return (
		<div className="min-w-0 flex-[1_1_520px]">
			<h1 className="flex flex-wrap items-center gap-x-2.5 gap-y-1.5 text-[22px]/7 font-semibold tracking-[-0.02em]">
				{glyph}
				<span className="font-mono wrap-anywhere">{name}</span>
				{nameChips}
			</h1>
			{chips ? (
				<div className="mt-2.5 flex flex-wrap gap-1.5">{chips}</div>
			) : null}
			<ObjectFacts facts={facts} extra={extra} />
		</div>
	);
}

/** SPEC §4.25 `.idhead` for N2/N3: glyph + mono name + chips, a chip line and a facts line. */
export function ObjectHeader({
	crumbs = [],
	actions,
	className,
	...identity
}: Readonly<ObjectHeaderProps>) {
	return (
		<header className={cx("flex flex-col gap-2.5", className)}>
			<Crumbs crumbs={crumbs} />
			<div className="flex flex-wrap items-start justify-between gap-x-6 gap-y-3">
				<ObjectIdentity {...identity} />
				<HeaderActions actions={actions} />
			</div>
		</header>
	);
}
