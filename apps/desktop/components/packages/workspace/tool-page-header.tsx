"use client";

import { Button } from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { ArrowLeft, type LucideIcon } from "lucide-react";
import Link from "next/link";
import type { ReactNode } from "react";
import { projectRoutes } from "../local-projects";

const MINE_HREF = "/store/packages?tab=mine";

/** Back target for a `/developer/*` tool: the checkout's workspace, or Mine without one. */
export function toolBackHref(projectPath?: string | null): string {
	return projectPath
		? projectRoutes.workspace({ project: projectPath })
		: MINE_HREF;
}

export function ToolPageHeader({
	icon: Icon,
	title,
	description,
	backHref,
	actions,
}: Readonly<{
	icon: LucideIcon;
	title: string;
	description: string;
	backHref: string;
	actions?: ReactNode;
}>) {
	const { t } = useTranslation("common");
	return (
		<div className="flex flex-wrap items-center justify-between gap-3 border-b border-border/40 pb-4">
			<div className="flex min-w-0 items-center gap-3">
				<Button
					asChild
					variant="ghost"
					size="icon"
					className="size-8 shrink-0 rounded-full text-muted-foreground hover:text-foreground"
				>
					<Link href={backHref} aria-label={t("back", "Back")}>
						<ArrowLeft className="size-4" />
					</Link>
				</Button>
				<div className="min-w-0">
					<h1 className="flex items-center gap-2 text-2xl font-semibold tracking-tight">
						<Icon className="size-5 shrink-0 text-muted-foreground" />
						<span className="truncate">{title}</span>
					</h1>
					<p className="text-sm text-muted-foreground">{description}</p>
				</div>
			</div>
			{actions && (
				<div className="flex flex-wrap items-center gap-2">{actions}</div>
			)}
		</div>
	);
}
