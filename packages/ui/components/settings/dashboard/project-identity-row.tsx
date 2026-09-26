"use client";

import { useTranslation } from "@flow-like/locales";
import { PencilIcon, SettingsIcon } from "lucide-react";
import type { ReactNode } from "react";
import type { IApp, IMetadata } from "../../../lib";
import { cn, sanitizeImageUrl } from "../../../lib/utils";
import { AppTypeLabel, AppTypeMark } from "../../ui/app-type-mark";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import { Card } from "../../ui/card";
import { Tooltip, TooltipContent, TooltipTrigger } from "../../ui/tooltip";
import { VisibilityBadge } from "./dashboard-primitives";
import type {
	DashboardPermissions,
	InspectorPanel,
} from "./use-project-signals";

/**
 * A control that stays visible when the role cannot use it, with the reason
 * attached. A silently disabled button reads as a bug; a disabled button that
 * names the missing permission reads as an access boundary.
 *
 * The wrapping span is required: a disabled button swallows pointer events, so
 * the tooltip would never open on the trigger itself. It always carries
 * `inline-flex` so the wrapped control keeps the width it had while enabled —
 * a bare `display: inline` span does not stretch a `flex-1` child.
 * Shared by the dashboards rather than redefined in each.
 */
export function GuardedAction({
	allowed,
	reason,
	className,
	children,
}: Readonly<{
	allowed: boolean;
	reason: string;
	className?: string;
	children: ReactNode;
}>) {
	if (allowed) return <>{children}</>;
	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<span className={cn("inline-flex", className)}>{children}</span>
			</TooltipTrigger>
			<TooltipContent side="bottom">{reason}</TooltipContent>
		</Tooltip>
	);
}

/**
 * The compact identity strip shared by both dashboards. Replaces the 128px
 * decorative banner plus separate identity block: the same information in one
 * row, with the media and metadata editing behind the inspector instead of a
 * modal.
 */
export function ProjectIdentityRow({
	app,
	metadata,
	permissions,
	onOpenPanel,
	statusSlot,
	actions,
}: Readonly<{
	app: IApp;
	metadata: IMetadata;
	permissions: DashboardPermissions;
	onOpenPanel: (panel: InspectorPanel) => void;
	statusSlot?: ReactNode;
	actions?: ReactNode;
}>) {
	const { t } = useTranslation("settings");
	// The identity panel spans both guards: name, summary and artwork are
	// `WriteMeta`, the app type is an app-row field and needs `Owner`.
	const canOpenIdentity = permissions.canWriteMeta || permissions.canWriteApp;
	const identityReason = t(
		"yourRoleCannotEditThisProjectsIdentity",
		"Your role cannot edit this project's identity.",
	);
	const accessReason = t(
		"onlyAnOwnerCanChangeSharingSettings",
		"Only an owner can change sharing settings.",
	);

	return (
		<Card className="flex min-w-0 flex-col items-stretch gap-3 px-3 py-3 sm:flex-row sm:items-center sm:px-4">
			<div className="flex min-w-0 items-center gap-3">
				<button
					type="button"
					className="shrink-0 rounded-lg border-0 bg-transparent p-0"
					onClick={() => onOpenPanel("identity")}
					aria-label={t("editIdentityAndMedia", "Edit identity and media")}
					disabled={!canOpenIdentity}
				>
					<AppTypeMark
						type={app.app_type}
						size={40}
						src={sanitizeImageUrl(metadata.icon ?? undefined, "/app-logo.webp")}
						fallback={(metadata.name ?? "").substring(0, 2).toUpperCase()}
					/>
				</button>

				<div className="min-w-0 flex-1">
					<div className="flex flex-wrap items-center gap-2">
						<h1 className="truncate text-base font-semibold tracking-tight">
							{metadata.name}
						</h1>
						<VisibilityBadge visibility={app.visibility} />
						{app.version && (
							<Badge
								variant="outline"
								className="text-xs"
							>{`v${app.version}`}</Badge>
						)}
						{statusSlot}
					</div>
					<p className="flex items-center gap-2 truncate text-xs text-muted-foreground">
						<AppTypeLabel type={app.app_type} className="shrink-0" />
						{metadata.description && (
							<>
								<span aria-hidden>·</span>
								<span className="truncate">{metadata.description}</span>
							</>
						)}
					</p>
				</div>
			</div>

			<div className="flex min-w-0 flex-wrap items-center gap-2 sm:ml-auto sm:shrink-0 sm:flex-nowrap">
				{actions}
				<GuardedAction
					allowed={canOpenIdentity}
					reason={identityReason}
					className="flex-1 sm:flex-none"
				>
					<Button
						variant="outline"
						size="sm"
						onClick={() => onOpenPanel("identity")}
						disabled={!canOpenIdentity}
						className="flex-1 sm:flex-none"
					>
						<PencilIcon className="mr-1.5 h-3 w-3" />
						{t("identity", "Identity")}
					</Button>
				</GuardedAction>
				<GuardedAction
					allowed={permissions.canWriteApp}
					reason={accessReason}
					className="flex-1 sm:flex-none"
				>
					<Button
						variant="outline"
						size="sm"
						onClick={() => onOpenPanel("access")}
						disabled={!permissions.canWriteApp}
						className="flex-1 sm:flex-none"
					>
						<SettingsIcon className="mr-1.5 h-3 w-3" />
						{t("settings", "Settings")}
					</Button>
				</GuardedAction>
			</div>
		</Card>
	);
}
