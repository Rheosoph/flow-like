"use client";

import { useTranslation } from "@flow-like/locales";
import {
	AlertTriangle,
	CheckCircle2,
	Circle,
	Globe,
	KeyRound,
	Lock,
	type LucideIcon,
	XCircle,
} from "lucide-react";
import type { ReactNode } from "react";
import { cn, humanFileSize } from "../../../lib/utils";
import { RelativeTime } from "../../ui";
import type {
	OverviewCheck,
	OverviewCheckStatus,
	WorkspaceRegistryState,
} from "./workspace-model";

export function WorkspaceSection({
	icon: Icon,
	title,
	action,
	children,
	className,
	bodyClassName,
}: Readonly<{
	icon?: LucideIcon;
	title: ReactNode;
	action?: ReactNode;
	children?: ReactNode;
	className?: string;
	bodyClassName?: string;
}>) {
	return (
		<section
			className={cn(
				"min-w-0 rounded-xl border border-border/60 bg-card px-4 py-3.5",
				className,
			)}
		>
			<div className="flex min-h-6 items-center gap-2">
				{Icon && <Icon className="size-4 shrink-0 text-muted-foreground" />}
				<h2 className="min-w-0 flex-1 truncate text-sm font-semibold">
					{title}
				</h2>
				{action}
			</div>
			{children !== undefined && (
				<div className={cn("mt-3", bodyClassName)}>{children}</div>
			)}
		</section>
	);
}

export function WorkspaceNotice({
	icon: Icon,
	title,
	description,
	action,
}: Readonly<{
	icon: LucideIcon;
	title: ReactNode;
	description?: ReactNode;
	action?: ReactNode;
}>) {
	return (
		<div className="flex flex-col items-center gap-2 rounded-xl border border-dashed border-border/70 px-6 py-10 text-center">
			<Icon className="size-8 text-muted-foreground" />
			<h3 className="text-sm font-semibold">{title}</h3>
			{description && (
				<p className="max-w-md text-sm text-muted-foreground">{description}</p>
			)}
			{action && <div className="mt-2">{action}</div>}
		</div>
	);
}

export function WorkspaceLinkButton({
	children,
	onClick,
}: Readonly<{ children: ReactNode; onClick: () => void }>) {
	return (
		<button
			type="button"
			onClick={onClick}
			className="inline-flex shrink-0 items-center gap-1 rounded-sm text-xs font-medium text-primary outline-none transition-colors hover:text-primary/80 focus-visible:ring-2 focus-visible:ring-ring"
		>
			{children}
		</button>
	);
}

export function FieldGrid({ children }: Readonly<{ children: ReactNode }>) {
	return (
		<dl className="grid grid-cols-[5.5rem_minmax(0,1fr)] items-center gap-x-3 gap-y-2.5 text-sm">
			{children}
		</dl>
	);
}

export function Field({
	label,
	children,
}: Readonly<{ label: ReactNode; children: ReactNode }>) {
	return (
		<>
			<dt className="text-muted-foreground">{label}</dt>
			<dd className="flex min-w-0 items-center gap-2">{children}</dd>
		</>
	);
}

export function CountBadge({ value }: Readonly<{ value: number }>) {
	return (
		<span className="rounded-full bg-muted px-1.5 font-mono text-[11px] leading-4.5 text-muted-foreground">
			{value}
		</span>
	);
}

export function VisibilityLabel({
	visibility,
	className,
}: Readonly<{ visibility: string; className?: string }>) {
	const { t } = useTranslation("common");
	const { Icon, label } =
		visibility === "public"
			? { Icon: Globe, label: t("workspaceVisibilityPublic", "Public") }
			: visibility === "public_request_access"
				? {
						Icon: KeyRound,
						label: t("workspaceVisibilityRequestAccess", "On request"),
					}
				: { Icon: Lock, label: t("workspaceVisibilityPrivate", "Private") };
	return (
		<span className={cn("inline-flex items-center gap-1.5", className)}>
			<Icon className="size-3.5 shrink-0" />
			{label}
		</span>
	);
}

const REGISTRY_STATE_TONE: Record<
	WorkspaceRegistryState,
	{ pill: string; dot: string }
> = {
	live: {
		pill: "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400",
		dot: "bg-emerald-500",
	},
	in_review: {
		pill: "bg-sky-500/10 text-sky-600 dark:text-sky-400",
		dot: "bg-sky-500",
	},
	disabled: {
		pill: "bg-destructive/10 text-destructive",
		dot: "bg-destructive",
	},
	rejected: {
		pill: "bg-destructive/10 text-destructive",
		dot: "bg-destructive",
	},
};

export function StatePill({
	tone,
	children,
}: Readonly<{ tone: { pill: string; dot: string }; children: ReactNode }>) {
	return (
		<span
			className={cn(
				"inline-flex h-5.5 shrink-0 items-center gap-1.5 rounded-full px-2.5 text-xs font-medium",
				tone.pill,
			)}
		>
			<span className={cn("size-1.5 rounded-full", tone.dot)} />
			{children}
		</span>
	);
}

export function RegistryStatePill({
	state,
}: Readonly<{ state: WorkspaceRegistryState }>) {
	const { t } = useTranslation("common");
	const labels: Record<WorkspaceRegistryState, string> = {
		live: t("live", "Live"),
		in_review: t("inReview", "In review"),
		disabled: t("disabled", "Disabled"),
		rejected: t("rejected", "Rejected"),
	};
	return (
		<StatePill tone={REGISTRY_STATE_TONE[state]}>{labels[state]}</StatePill>
	);
}

export function LiveVersionPill({ version }: Readonly<{ version: string }>) {
	return (
		<StatePill tone={REGISTRY_STATE_TONE.live}>
			<span className="font-mono">{version}</span>
		</StatePill>
	);
}

const CHECK_ICON: Record<OverviewCheckStatus, LucideIcon> = {
	ok: CheckCircle2,
	warning: AlertTriangle,
	error: XCircle,
	pending: Circle,
};

const CHECK_TONE: Record<OverviewCheckStatus, string> = {
	ok: "text-emerald-600 dark:text-emerald-400",
	warning: "text-tertiary",
	error: "text-destructive",
	pending: "text-muted-foreground",
};

function CheckValue({ check }: Readonly<{ check: OverviewCheck }>) {
	const { t } = useTranslation("common");
	if (check.id === "lint") {
		if (check.status === "pending")
			return t("workspaceLintNotRun", "Not run yet");
		return `${t("workspaceLintErrorCount", {
			defaultValue_one: "{{count}} error",
			defaultValue_other: "{{count}} errors",
			count: check.errors,
		})} · ${t("workspaceLintWarningCount", {
			defaultValue_one: "{{count}} warning",
			defaultValue_other: "{{count}} warnings",
			count: check.warnings,
		})}`;
	}
	if (check.status === "pending")
		return t("workspaceBuildUnknown", "No build info");
	if (check.status === "error" && check.sizeBytes === undefined)
		return t("workspaceBuildMissing", "No WASM build");
	return (
		<>
			{check.sizeBytes !== undefined && humanFileSize(check.sizeBytes, true)}
			{check.sizeBytes !== undefined && check.builtAt !== undefined && " · "}
			{check.builtAt !== undefined && <RelativeTime value={check.builtAt} />}
			{check.status === "warning" &&
				` · ${t("workspaceBuildStale", "older than the source")}`}
		</>
	);
}

/** The Build / Lint tiles for a linked checkout's overview. */
export function WorkspaceChecks({
	checks,
}: Readonly<{ checks: readonly OverviewCheck[] }>) {
	const { t } = useTranslation("common");
	const labels: Record<OverviewCheck["id"], string> = {
		build: t("workspaceCheckBuild", "Build"),
		lint: t("workspaceCheckLint", "Lint"),
	};
	return (
		<div className="grid grid-cols-1 gap-2.5 sm:grid-cols-2">
			{checks.map((check) => {
				const Icon = CHECK_ICON[check.status];
				return (
					<div
						key={check.id}
						className="min-w-0 rounded-lg border border-border/60 px-3 py-2.5"
					>
						<div className="flex items-center gap-1.5 text-xs font-semibold">
							<Icon className={cn("size-3.5", CHECK_TONE[check.status])} />
							{labels[check.id]}
						</div>
						<div
							className={cn(
								"mt-1 truncate text-xs",
								check.status === "warning" || check.status === "error"
									? CHECK_TONE[check.status]
									: "text-muted-foreground",
							)}
						>
							<CheckValue check={check} />
						</div>
					</div>
				);
			})}
		</div>
	);
}
