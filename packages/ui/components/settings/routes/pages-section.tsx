"use client";

import { useTranslation } from "@flow-like/locales";
import { FileText, LayoutGrid, Pencil, Trash2, Workflow } from "lucide-react";
import type { IMetadata } from "../../../types";
import { Button } from "../../ui/button";
import { Card, CardContent } from "../../ui/card";
import { EmptyState } from "../../ui/empty-state";
import { Tooltip, TooltipContent, TooltipTrigger } from "../../ui/tooltip";

export interface PageData {
	appId: string;
	pageId: string;
	boardId: string | null;
	metadata: IMetadata;
}

export interface PagesSectionProps {
	pages: PageData[];
	onOpenPage: (pageId: string, boardId?: string) => void;
	onOpenBoard: (boardId: string) => void;
	onDelete: (pageId: string, boardId: string | null) => Promise<void>;
}

export function PagesSection({
	pages,
	onOpenPage,
	onOpenBoard,
	onDelete,
}: PagesSectionProps) {
	const { t } = useTranslation("settings");
	if (pages.length === 0) {
		return (
			<EmptyState
				icons={[LayoutGrid]}
				title={t("noPagesYet", "No pages yet")}
				description={`Pages are created from within a flow. Open a flow and use the Pages panel to create your first page.`}
			/>
		);
	}

	return (
		<div className="space-y-4">
			<div className="space-y-1">
				<h2 className="text-lg font-semibold">{t("allPages", "All Pages")}</h2>
				<p className="text-sm text-muted-foreground">
					{t(
						"manageYourAppsVisualInterfaces",
						"Manage your app's visual interfaces",
					)}
				</p>
			</div>

			<div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
				{pages.map((page) => (
					<PageCard
						key={page.pageId}
						pageId={page.pageId}
						boardId={page.boardId}
						meta={page.metadata}
						onOpen={() => onOpenPage(page.pageId, page.boardId ?? undefined)}
						onOpenBoard={
							page.boardId ? () => onOpenBoard(page.boardId!) : undefined
						}
						onDelete={() => onDelete(page.pageId, page.boardId)}
					/>
				))}
			</div>
		</div>
	);
}

export interface PageCardProps {
	pageId: string;
	boardId: string | null;
	meta: IMetadata;
	onOpen: () => void;
	onOpenBoard?: () => void;
	onDelete: () => void;
}

export function PageCard({
	pageId,
	boardId,
	meta,
	onOpen,
	onOpenBoard,
	onDelete,
}: PageCardProps) {
	const { t } = useTranslation("settings");
	return (
		<Card className="group hover:shadow-lg transition-all duration-200 border-border/60 hover:border-primary/30 overflow-hidden">
			{/* Preview Area */}
			<div
				className="h-32 bg-linear-to-br from-muted/50 to-muted flex items-center justify-center cursor-pointer relative"
				onClick={onOpen}
			>
				<div className="absolute inset-0 bg-grid-pattern opacity-5" />
				<FileText className="h-12 w-12 text-muted-foreground/30" />
				<div className="absolute inset-0 bg-primary/5 opacity-0 group-hover:opacity-100 transition-opacity flex items-center justify-center">
					<Button variant="secondary" size="sm" className="gap-2">
						<Pencil className="h-4 w-4" />
						{t("editPage", "Edit Page")}
					</Button>
				</div>
			</div>

			<CardContent className="p-4">
				<div className="flex items-start justify-between gap-2">
					<div className="min-w-0 flex-1">
						<h3 className="font-semibold truncate">{meta.name}</h3>
						{meta.description && (
							<p className="text-sm text-muted-foreground line-clamp-1 mt-0.5">
								{meta.description}
							</p>
						)}
					</div>
				</div>

				<div className="flex items-center justify-between mt-4 pt-3 border-t border-border/40">
					<div className="flex items-center gap-2">
						{onOpenBoard && (
							<Tooltip>
								<TooltipTrigger asChild>
									<Button
										variant="outline"
										size="sm"
										className="h-8 gap-1.5"
										onClick={onOpenBoard}
									>
										<Workflow className="h-3.5 w-3.5" />
										{t("flow", "Flow")}
									</Button>
								</TooltipTrigger>
								<TooltipContent>
									{t("openConnectedFlow", "Open connected flow")}
								</TooltipContent>
							</Tooltip>
						)}
					</div>
					<div className="flex items-center gap-1">
						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant="ghost"
									size="icon"
									className="h-8 w-8"
									onClick={onOpen}
								>
									<Pencil className="h-4 w-4" />
								</Button>
							</TooltipTrigger>
							<TooltipContent>{t("editPage", "Edit Page")}</TooltipContent>
						</Tooltip>
						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant="ghost"
									size="icon"
									className="h-8 w-8 text-destructive hover:text-destructive hover:bg-destructive/10"
									onClick={onDelete}
								>
									<Trash2 className="h-4 w-4" />
								</Button>
							</TooltipTrigger>
							<TooltipContent>{t("deletePage", "Delete Page")}</TooltipContent>
						</Tooltip>
					</div>
				</div>
			</CardContent>
		</Card>
	);
}
