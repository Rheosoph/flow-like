"use client";

import { useTranslation } from "@flow-like/locales";
import { PencilIcon, SettingsIcon } from "lucide-react";
import type { ReactNode } from "react";
import type { IApp, IMetadata } from "../../../lib";
import { sanitizeImageUrl } from "../../../lib/utils";
import { AppTypeLabel, AppTypeMark } from "../../ui/app-type-mark";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import { Card } from "../../ui/card";
import { VisibilityBadge } from "./dashboard-primitives";
import type { InspectorPanel } from "./use-project-signals";

/**
 * The compact identity strip shared by both dashboards. Replaces the 128px
 * decorative banner plus separate identity block: the same information in one
 * row, with the media and metadata editing behind the inspector instead of a
 * modal.
 */
export function ProjectIdentityRow({
	app,
	metadata,
	canEdit,
	onOpenPanel,
	statusSlot,
	actions,
}: Readonly<{
	app: IApp;
	metadata: IMetadata;
	canEdit: boolean;
	onOpenPanel: (panel: InspectorPanel) => void;
	statusSlot?: ReactNode;
	actions?: ReactNode;
}>) {
	const { t } = useTranslation("settings");
	return (
		<Card className="flex min-w-0 flex-col items-stretch gap-3 px-3 py-3 sm:flex-row sm:items-center sm:px-4">
			<div className="flex min-w-0 items-center gap-3">
				<button
					type="button"
					className="shrink-0 rounded-lg border-0 bg-transparent p-0"
					onClick={() => onOpenPanel("identity")}
					aria-label={t("editIdentityAndMedia", "Edit identity and media")}
					disabled={!canEdit}
				>
					<AppTypeMark
						type={app.app_type}
						size={40}
						src={sanitizeImageUrl(metadata.icon ?? undefined, "/app-logo.webp")}
						fallback={metadata.name.substring(0, 2).toUpperCase()}
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
				{canEdit && (
					<>
						<Button
							variant="outline"
							size="sm"
							onClick={() => onOpenPanel("identity")}
							className="flex-1 sm:flex-none"
						>
							<PencilIcon className="mr-1.5 h-3 w-3" />
							{t("identity", "Identity")}
						</Button>
						<Button
							variant="outline"
							size="sm"
							onClick={() => onOpenPanel("access")}
							className="flex-1 sm:flex-none"
						>
							<SettingsIcon className="mr-1.5 h-3 w-3" />
							{t("settings", "Settings")}
						</Button>
					</>
				)}
			</div>
		</Card>
	);
}
