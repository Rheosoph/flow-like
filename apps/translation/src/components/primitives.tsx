import type { ReactNode } from "react";
import type { KeyStatus } from "../lib/keys";

export const STATUS_LABEL: Record<KeyStatus, string> = {
	translated: "Translated",
	missing: "Missing",
	copied: "Same as source",
	broken: "Placeholder lost",
	orphan: "Not in source",
};

export const STATUS_TONE: Record<KeyStatus, string> = {
	translated: "bg-(--ok)",
	missing: "bg-muted-foreground/45",
	copied: "bg-(--warn)",
	broken: "bg-(--crit)",
	orphan: "bg-(--crit)",
};

export function StatusDot({ status }: Readonly<{ status: KeyStatus }>) {
	return (
		<span
			className={`size-1.5 shrink-0 rounded-full ${STATUS_TONE[status]}`}
			aria-hidden
		/>
	);
}

export function Meter({
	value,
	className = "",
}: Readonly<{ value: number; className?: string }>) {
	const tone =
		value === 0 ? "bg-(--crit)" : value < 60 ? "bg-(--warn)" : "bg-(--ok)";
	return (
		<div
			className={`h-1 w-full overflow-hidden rounded-full bg-muted-foreground/20 ${className}`}
			aria-hidden
		>
			<div
				className={`h-full rounded-full transition-[width] duration-300 ${tone}`}
				style={{ width: `${value}%` }}
			/>
		</div>
	);
}

export function Chip({
	active,
	onClick,
	children,
	count,
}: Readonly<{
	active: boolean;
	onClick: () => void;
	children: ReactNode;
	count?: number;
}>) {
	return (
		<button
			type="button"
			aria-pressed={active}
			onClick={onClick}
			className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-[11.5px] transition-colors ${
				active
					? "border-foreground bg-foreground text-background"
					: "border-border text-muted-foreground hover:bg-muted"
			}`}
		>
			{children}
			{count !== undefined && (
				<span className="font-mono text-[10.5px] tabular-nums opacity-75">
					{count}
				</span>
			)}
		</button>
	);
}

export function Panel({
	title,
	subtitle,
	action,
	children,
	className = "",
}: Readonly<{
	title: string;
	subtitle?: string;
	action?: ReactNode;
	children: ReactNode;
	className?: string;
}>) {
	return (
		<section
			className={`overflow-hidden rounded-md border border-border bg-card ${className}`}
		>
			<header className="flex flex-wrap items-center gap-2 border-b border-border px-3.5 py-2.5">
				<h2 className="text-[13px] font-semibold">{title}</h2>
				{subtitle && (
					<span className="text-[11.5px] text-muted-foreground">
						{subtitle}
					</span>
				)}
				{action && (
					<div className="ml-auto flex items-center gap-1.5">{action}</div>
				)}
			</header>
			{children}
		</section>
	);
}

export function PrimaryButton({
	children,
	onClick,
	disabled,
	type = "button",
}: Readonly<{
	children: ReactNode;
	onClick?: () => void;
	disabled?: boolean;
	type?: "button" | "submit";
}>) {
	return (
		<button
			type={type}
			onClick={onClick}
			disabled={disabled}
			className="inline-flex items-center gap-1.5 rounded-md bg-primary px-3 py-1.5 text-[12.5px] font-medium text-primary-foreground transition-[filter] hover:brightness-105 disabled:cursor-not-allowed disabled:opacity-50"
		>
			{children}
		</button>
	);
}

export function GhostButton({
	children,
	onClick,
	title,
}: Readonly<{ children: ReactNode; onClick?: () => void; title?: string }>) {
	return (
		<button
			type="button"
			title={title}
			onClick={onClick}
			className="inline-flex items-center gap-1.5 rounded-md border border-border px-2.5 py-1 text-[11.5px] text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
		>
			{children}
		</button>
	);
}
