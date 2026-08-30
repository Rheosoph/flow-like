"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Boxes,
	Copy,
	Database,
	Layers,
	Loader2,
	MoreHorizontal,
	Plus,
	Search,
	SquareTerminal,
	Trash2,
	X,
} from "lucide-react";
import { useState } from "react";
import { useSearch } from "../../../../hooks/use-search-index";
import type { SavedQuery } from "../../../../state/backend-state/query-state";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "../../../ui/alert-dialog";
import { Button } from "../../../ui/button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "../../../ui/dropdown-menu";
import { Input } from "../../../ui/input";
import { ScrollArea } from "../../../ui/scroll-area";
import { Skeleton } from "../../../ui/skeleton";

function QueryGroup({
	title,
	icon: Icon,
	items,
	activeId,
	onSelect,
	onDelete,
	onDuplicate,
}: Readonly<{
	title: string;
	icon: typeof SquareTerminal;
	items: SavedQuery[];
	activeId?: string;
	onSelect: (query: SavedQuery) => void;
	onDelete: (query: SavedQuery) => void;
	onDuplicate: (query: SavedQuery) => void;
}>) {
	const { t } = useTranslation("settings");
	if (items.length === 0) return null;
	const headingId = `saved-query-group-${title.replace(/\s+/g, "-").toLowerCase()}`;
	return (
		<div className="space-y-1">
			<p
				id={headingId}
				className="flex items-center gap-1.5 px-2 py-1 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground"
			>
				<Icon className="h-3 w-3" />
				{title}
				<span className="ml-auto tabular-nums">{items.length}</span>
			</p>
			<ul aria-labelledby={headingId} className="space-y-0.5">
				{items.map((query) => {
					const active = query.id === activeId;
					return (
						<li
							key={query.id}
							className={`group relative flex items-center gap-1 rounded-lg pr-1 text-sm transition-colors ${
								active
									? "bg-primary/10 before:absolute before:inset-y-1.5 before:left-0 before:w-0.5 before:rounded-full before:bg-primary"
									: "hover:bg-muted/50"
							}`}
						>
							<button
								type="button"
								onClick={() => onSelect(query)}
								aria-current={active ? "true" : undefined}
								className="min-w-0 flex-1 rounded-lg px-2.5 py-2 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
							>
								<p className="truncate font-medium">{query.name}</p>
								<p className="flex items-center gap-1 truncate text-[11px] text-muted-foreground">
									{query.surface === "overlay" ? (
										<Boxes className="h-3 w-3 shrink-0" />
									) : (
										<Database className="h-3 w-3 shrink-0" />
									)}
									<span className="truncate">
										{query.description || query.surface}
									</span>
								</p>
							</button>
							<DropdownMenu>
								<DropdownMenuTrigger asChild>
									<Button
										variant="ghost"
										size="icon"
										className="mr-1 h-7 w-7 shrink-0 text-muted-foreground opacity-0 transition-opacity focus-visible:opacity-100 group-focus-within:opacity-100 group-hover:opacity-100"
										aria-label={t("actionsForName", "Actions for {{name}}", {
											name: query.name,
										})}
									>
										<MoreHorizontal className="h-3.5 w-3.5" />
									</Button>
								</DropdownMenuTrigger>
								<DropdownMenuContent align="end" className="w-40">
									<DropdownMenuItem onClick={() => onDuplicate(query)}>
										<Copy className="h-3.5 w-3.5" />{" "}
										{t("duplicate", "Duplicate")}
									</DropdownMenuItem>
									<DropdownMenuSeparator />
									<DropdownMenuItem
										variant="destructive"
										onClick={() => onDelete(query)}
									>
										<Trash2 className="h-3.5 w-3.5" /> {t("delete", "Delete")}
									</DropdownMenuItem>
								</DropdownMenuContent>
							</DropdownMenu>
						</li>
					);
				})}
			</ul>
		</div>
	);
}

function SidebarSkeleton() {
	return (
		<div className="space-y-2 p-2" aria-hidden>
			{["a", "b", "c", "d"].map((key) => (
				<div key={key} className="rounded-lg px-2.5 py-2">
					<Skeleton className="h-3.5 w-3/4" />
					<Skeleton className="mt-1.5 h-2.5 w-1/2 opacity-70" />
				</div>
			))}
		</div>
	);
}

export function SavedQuerySidebar({
	queries,
	activeId,
	loading,
	onSelect,
	onNew,
	onDelete,
	onDuplicate,
}: Readonly<{
	queries: SavedQuery[];
	activeId?: string;
	loading: boolean;
	onSelect: (query: SavedQuery) => void;
	onNew: () => void;
	onDelete: (query: SavedQuery) => Promise<void> | void;
	onDuplicate: (query: SavedQuery) => Promise<void> | void;
}>) {
	const { t } = useTranslation("settings");
	const [deleteTarget, setDeleteTarget] = useState<SavedQuery | null>(null);
	const [deleting, setDeleting] = useState(false);
	const [search, setSearch] = useState("");

	const filtered = useSearch(queries, search, {
		fields: ["name", "description", "sql"],
		boost: { name: 3 },
	});

	const storedQueries = filtered.filter((query) => query.kind === "query");
	const views = filtered.filter((query) => query.kind === "view");

	const confirmDelete = async () => {
		if (!deleteTarget) return;
		setDeleting(true);
		try {
			await onDelete(deleteTarget);
			setDeleteTarget(null);
		} finally {
			setDeleting(false);
		}
	};

	return (
		<aside
			aria-label={t("savedQueries", "Saved queries")}
			className="flex h-full min-h-0 flex-col border-r bg-muted/20"
		>
			<div className="flex items-center justify-between gap-2 border-b p-3">
				<div>
					<h3 className="text-sm font-semibold">{t("queries", "Queries")}</h3>
					<p className="text-[11px] text-muted-foreground">
						{t("savedAmpViews", "Saved & views")}
					</p>
				</div>
				<Button size="sm" onClick={onNew}>
					<Plus className="h-4 w-4" /> {t("new", "New")}
				</Button>
			</div>

			{queries.length > 0 && (
				<div className="border-b p-2">
					<div className="relative">
						<Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
						<Input
							value={search}
							onChange={(event) => setSearch(event.target.value)}
							placeholder={t("searchQueries", "Search queries…")}
							aria-label={t("searchSavedQueries", "Search saved queries")}
							className="h-8 pl-8 pr-7 text-xs"
						/>
						{search && (
							<button
								type="button"
								onClick={() => setSearch("")}
								aria-label={t("clearSearch", "Clear search")}
								className="absolute right-1.5 top-1/2 -translate-y-1/2 rounded p-0.5 text-muted-foreground hover:text-foreground"
							>
								<X className="h-3.5 w-3.5" />
							</button>
						)}
					</div>
				</div>
			)}

			<ScrollArea className="min-h-0 flex-1">
				{loading && queries.length === 0 ? (
					<SidebarSkeleton />
				) : queries.length === 0 ? (
					<div className="flex flex-col items-center gap-3 px-4 py-10 text-center">
						<div className="rounded-lg border bg-muted/40 p-2.5 text-muted-foreground/70">
							<SquareTerminal className="h-5 w-5" />
						</div>
						<div className="space-y-0.5">
							<p className="text-sm font-medium">
								{t("noSavedQueries", "No saved queries")}
							</p>
							<p className="text-xs text-muted-foreground">
								{t(
									"writeSqlAndHitSaveToKeepItHere",
									"Write SQL and hit Save to keep it here.",
								)}
							</p>
						</div>
						<Button variant="outline" size="sm" onClick={onNew}>
							<Plus className="h-4 w-4" /> {t("newQuery", "New query")}
						</Button>
					</div>
				) : filtered.length === 0 ? (
					<p className="px-2 py-8 text-center text-xs text-muted-foreground">
						{t("noQueriesMatchSearch", "No queries match “{{search}}”.", {
							search,
						})}
					</p>
				) : (
					<div className="space-y-3 p-2">
						<QueryGroup
							title={t("storedQueries", "Stored queries")}
							icon={SquareTerminal}
							items={storedQueries}
							activeId={activeId}
							onSelect={onSelect}
							onDelete={setDeleteTarget}
							onDuplicate={onDuplicate}
						/>
						<QueryGroup
							title="Views"
							icon={Layers}
							items={views}
							activeId={activeId}
							onSelect={onSelect}
							onDelete={setDeleteTarget}
							onDuplicate={onDuplicate}
						/>
					</div>
				)}
			</ScrollArea>

			<AlertDialog
				open={Boolean(deleteTarget)}
				onOpenChange={(open) => {
					if (!deleting && !open) setDeleteTarget(null);
				}}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>
							{t("delete", "Delete")} {deleteTarget?.name}?
						</AlertDialogTitle>
						<AlertDialogDescription>
							{deleteTarget?.kind === "view"
								? t(
										"queriesThatReferenceThisViewWillStopResolvingIt",
										"Queries that reference this view will stop resolving it.",
									)
								: t(
										"thisRemovesTheSavedQueryYourDataIsUntouched",
										"This removes the saved query. Your data is untouched.",
									)}
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel disabled={deleting}>
							{t("cancel", "Cancel")}
						</AlertDialogCancel>
						<AlertDialogAction
							className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
							disabled={deleting}
							onClick={(event) => {
								event.preventDefault();
								void confirmDelete();
							}}
						>
							{deleting && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
							{t("delete", "Delete")}
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</aside>
	);
}
