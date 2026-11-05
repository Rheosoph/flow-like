"use client";

import { useTranslation } from "@flow-like/locales";
import { createId } from "@paralleldrive/cuid2";
import { useQueryClient } from "@tanstack/react-query";
import { CopyIcon, PlusIcon, SearchIcon, XIcon } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { toast } from "sonner";
import { useAppPermissions } from "../../../../hooks/use-app-permissions";
import { useInvoke } from "../../../../hooks/use-invoke";
import { useSearch } from "../../../../hooks/use-search-index";
import { RolePermissions } from "../../../../lib/permission/role-permission";
import { IExecutionStage, ILogLevel } from "../../../../lib/schema/flow/board";
import { useSetQueryParams } from "../../../../lib/set-query-params";
import { cn } from "../../../../lib/utils";
import { useBackend } from "../../../../state/backend-state";
import { Button } from "../../../ui/button";
import { Input } from "../../../ui/input";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../../ui/select";
import { SectionLockedPanel } from "../../permission/permission-gate";
import { PermissionNotice } from "../../permission/permission-notice";
import { TemplatePreview } from "../template-preview";
import { CreateTemplateDialog } from "./create-template-dialog";
import { TemplateCard, TemplateCardSkeleton } from "./template-card";
import { TemplatePeekDialog } from "./template-peek-dialog";
import {
	type ITemplateItem,
	type ITemplateSort,
	filterBySource,
	sortTemplates,
	sourceGroups,
} from "./templates-gallery-model";
import { useTemplateItems } from "./use-template-items";

const GRID_STYLE = {
	gridTemplateColumns: "repeat(auto-fill, minmax(min(280px, 100%), 1fr))",
} as const;

const SEARCH_OPTIONS = {
	fields: ["name", "description", "tags", "sourceName"],
	boost: { name: 3, sourceName: 1.5 },
} as const;

export interface TemplatesPageProps {
	readonly appId: string;
	/** Set while a single template is open (`?templateId=`). */
	readonly templateId?: string | null;
	readonly onOpenFlow: (boardId: string) => void;
}

export function TemplatesPage({
	appId,
	templateId,
	onOpenFlow,
}: TemplatesPageProps) {
	const { t } = useTranslation("common");
	const permissions = useAppPermissions(appId);
	const canReadTemplates = permissions.can(RolePermissions.ReadTemplates);
	const canWriteTemplates = permissions.can(RolePermissions.WriteTemplates);

	if (templateId)
		return (
			<TemplatePreview
				appId={appId}
				templateId={templateId}
				canEdit={canWriteTemplates}
			/>
		);

	// A denied list read must not render as "No templates yet" plus an
	// invitation to create one — the read never happened.
	if (!canReadTemplates)
		return (
			<main className="flex flex-col grow max-h-full p-6 pt-0 min-h-0">
				<SectionLockedPanel
					feature={t("templates", "Templates")}
					description={t(
						"yourRoleCannotListThisProjectaposTemplates",
						"Your role cannot list this project's templates, so this page cannot tell you whether any exist.",
					)}
					missing={[RolePermissions.ReadTemplates]}
					roleName={permissions.roleName}
				/>
			</main>
		);

	return <TemplatesGallery appId={appId} onOpenFlow={onOpenFlow} />;
}

function TemplatesGallery({
	appId,
	onOpenFlow,
}: Readonly<{ appId: string; onOpenFlow: (boardId: string) => void }>) {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const queryClient = useQueryClient();
	const setQueryParams = useSetQueryParams();
	const permissions = useAppPermissions(appId);
	const canWriteTemplates = permissions.can(RolePermissions.WriteTemplates);
	const canReadBoards = permissions.can(RolePermissions.ReadBoards);
	const canWriteBoards = permissions.can(RolePermissions.WriteBoards);
	// Creating a template runs `upsert_template`, which checks WriteTemplates and
	// then hard-checks ReadBoards before it may read the source flow. Editing an
	// existing template's details only writes meta, which needs WriteTemplates
	// alone — so ReadBoards gates creation, never editing.
	const canAuthorTemplates = canWriteTemplates && canReadBoards;

	const templates = useInvoke(
		backend.templateState.getTemplates,
		backend.templateState,
		[appId],
		appId !== "",
	);
	const items = useTemplateItems(templates.data, appId !== "");

	const [query, setQuery] = useState("");
	const [source, setSource] = useState<string | null>(null);
	const [sort, setSort] = useState<ITemplateSort>("updated");
	const [peek, setPeek] = useState<{ key: string; flowForm: boolean } | null>(
		null,
	);
	const [createOpen, setCreateOpen] = useState(false);

	const groups = useMemo(() => sourceGroups(items), [items]);
	const activeSource =
		source && groups.some((group) => group.name === source) ? source : null;
	const searched = useSearch(items, query, SEARCH_OPTIONS);
	const visible = useMemo(
		() => sortTemplates(filterBySource(searched, activeSource), sort),
		[searched, activeSource, sort],
	);
	const peekItem = peek
		? items.find((item) => item.key === peek.key)
		: undefined;

	const openPeek = useCallback(
		(item: ITemplateItem) => setPeek({ key: item.key, flowForm: false }),
		[],
	);
	const openFlowForm = useCallback(
		(item: ITemplateItem) => setPeek({ key: item.key, flowForm: true }),
		[],
	);
	const clearFilters = useCallback(() => {
		setQuery("");
		setSource(null);
	}, []);

	const createFlow = useCallback(
		async (item: ITemplateItem, name: string) => {
			if (!item.board) return false;
			const boardId = createId();
			try {
				await backend.boardState.upsertBoard(
					appId,
					boardId,
					name,
					item.description,
					ILogLevel.Debug,
					IExecutionStage.Dev,
					undefined,
					item.board,
				);
			} catch (error) {
				console.error("Failed to create a flow from a template:", error);
				toast.error(
					t(
						"templateFlowCreateFailed",
						"Could not create the flow: {{message}}",
						{
							message: error instanceof Error ? error.message : String(error),
						},
					),
				);
				return false;
			}
			void queryClient.invalidateQueries({
				queryKey: ["getBoardSummaries", appId],
			});
			toast.success(
				t("templateFlowCreated", "Flow “{{name}}” created", { name }),
				{
					action: {
						label: t("open", "Open"),
						onClick: () => onOpenFlow(boardId),
					},
				},
			);
			return true;
		},
		[appId, backend.boardState, onOpenFlow, queryClient, t],
	);

	const deleteTemplate = useCallback(
		async (item: ITemplateItem) => {
			try {
				await backend.templateState.deleteTemplate(item.appId, item.templateId);
			} catch (error) {
				console.error("Failed to delete template:", error);
				toast.error(t("failedToDeleteTemplate", "Failed to delete template"));
				return false;
			}
			await templates.refetch();
			toast.success(
				t("templateDeletedNamed", "Deleted “{{name}}”", { name: item.name }),
			);
			return true;
		},
		[backend.templateState, t, templates.refetch],
	);

	const editTemplate = useCallback(
		(item: ITemplateItem) => setQueryParams("templateId", item.templateId),
		[setQueryParams],
	);

	const loading = templates.isLoading;
	const total = items.length;

	return (
		<main className="flex min-h-0 max-h-full grow flex-col gap-5 overflow-auto p-6 pt-0 md:overflow-visible">
			<header className="flex flex-col gap-4 pt-4">
				<div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
					<div className="min-w-0">
						<div className="flex items-center gap-2">
							<h1 className="text-xl font-semibold tracking-tight">
								{t("templates", "Templates")}
							</h1>
							{total > 0 ? (
								<span className="rounded-full bg-muted px-2 py-0.5 text-xs font-medium tabular-nums text-muted-foreground">
									{total}
								</span>
							) : null}
						</div>
						<p className="mt-0.5 text-xs text-muted-foreground">
							{t(
								"saveVersionedSnapshotsOfYourFlowsToShareReuseOrRollBack",
								"Save versioned snapshots of your flows to share, reuse, or roll back",
							)}
						</p>
					</div>
					<Button
						onClick={() => setCreateOpen(true)}
						disabled={!canAuthorTemplates}
						className="self-start sm:self-auto"
					>
						<PlusIcon aria-hidden className="size-4" />
						{t("createTemplate", "Create Template")}
					</Button>
				</div>

				{!canWriteTemplates ? (
					<PermissionNotice
						tone="readOnly"
						title={t(
							"templatesAreReadonlyForYourRole",
							"Templates are read-only for your role",
						)}
						description={t(
							"yourRoleCannotCreateEditOrDeleteTemplatesInThisProject",
							"Your role cannot create, edit or delete templates in this project.",
						)}
						missing={[RolePermissions.WriteTemplates]}
					/>
				) : !canReadBoards ? (
					<PermissionNotice
						tone="readOnly"
						title={t(
							"creatingTemplatesIsDisabled",
							"Creating templates is disabled",
						)}
						description={t(
							"creatingATemplateReadsTheSourceFlowSoItAlsoNeedsReadBoardsExistingTemplateDetailsStayEditable",
							"Creating a template reads the source flow, so it also needs Read Boards. Existing template details stay editable.",
						)}
						missing={[RolePermissions.ReadBoards]}
					/>
				) : null}

				{total > 0 ? (
					<div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-2">
						<div className="relative w-full sm:w-80">
							<SearchIcon
								aria-hidden
								className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground/60"
							/>
							<Input
								value={query}
								onChange={(event) => setQuery(event.target.value)}
								placeholder={t(
									"templateSearchPlaceholder",
									"Search name, description or flow",
								)}
								aria-label={t("searchTemplates", "Search templates...")}
								className="h-9 border-border/60 bg-muted/40 pl-9 pr-8 text-sm focus:bg-background"
							/>
							{query ? (
								<button
									type="button"
									onClick={() => setQuery("")}
									aria-label={t("clearSearch", "Clear search")}
									className="absolute right-3 top-1/2 -translate-y-1/2 text-muted-foreground/60 transition-colors hover:text-foreground"
								>
									<XIcon aria-hidden className="size-3.5" />
								</button>
							) : null}
						</div>
						<div className="flex items-center gap-2">
							<span className="text-xs text-muted-foreground">
								{t("sort", "Sort")}
							</span>
							<Select
								value={sort}
								onValueChange={(value) => setSort(value as ITemplateSort)}
							>
								<SelectTrigger
									size="sm"
									aria-label={t("sortTemplates", "Sort templates")}
									className="h-8 min-w-40 text-xs"
								>
									<SelectValue />
								</SelectTrigger>
								<SelectContent align="end">
									<SelectItem value="updated">
										{t("recentlyUpdated", "Recently updated")}
									</SelectItem>
									<SelectItem value="name">
										{t("templateSortName", "Name")}
									</SelectItem>
									<SelectItem value="nodes">
										{t("templateMostNodes", "Most nodes")}
									</SelectItem>
								</SelectContent>
							</Select>
						</div>
					</div>
				) : null}
				{groups.length > 1 ? (
					<SourceChips
						groups={groups}
						total={total}
						active={activeSource}
						onPick={setSource}
					/>
				) : null}
			</header>

			{loading ? (
				<div className="grid gap-4" style={GRID_STYLE}>
					<TemplateCardSkeleton />
					<TemplateCardSkeleton />
					<TemplateCardSkeleton />
				</div>
			) : total === 0 ? (
				<EmptyTemplates
					canCreate={canAuthorTemplates}
					onCreate={() => setCreateOpen(true)}
				/>
			) : visible.length === 0 ? (
				<div className="flex flex-col items-center gap-3 rounded-xl border border-dashed border-border/60 px-6 py-12 text-center">
					<p className="text-sm text-muted-foreground">
						{query
							? t("templateNoMatchQuery", "No templates match “{{query}}”.", {
									query,
								})
							: t(
									"templateNoMatchFilters",
									"No templates match these filters.",
								)}
					</p>
					<Button variant="outline" size="sm" onClick={clearFilters}>
						{t("clearFilters", "Clear filters")}
					</Button>
				</div>
			) : (
				<div className="grid gap-4 pb-6" style={GRID_STYLE}>
					{visible.map((item) => (
						<TemplateCard
							key={item.key}
							item={item}
							canCreateFlow={canWriteBoards}
							onOpen={openPeek}
							onUse={openFlowForm}
						/>
					))}
				</div>
			)}

			<TemplatePeekDialog
				peek={
					peekItem && peek
						? { item: peekItem, startWithFlowForm: peek.flowForm }
						: null
				}
				onClose={() => setPeek(null)}
				canCreateFlow={canWriteBoards}
				canDelete={canWriteTemplates}
				onCreateFlow={createFlow}
				onEdit={editTemplate}
				onDelete={deleteTemplate}
			/>
			<CreateTemplateDialog
				appId={appId}
				open={createOpen}
				onOpenChange={setCreateOpen}
				onCreated={async () => {
					setQuery("");
					setSource(null);
					setSort("updated");
					await templates.refetch();
				}}
			/>
		</main>
	);
}

function SourceChips({
	groups,
	total,
	active,
	onPick,
}: Readonly<{
	groups: readonly { name: string; count: number }[];
	total: number;
	active: string | null;
	onPick: (source: string | null) => void;
}>) {
	const { t } = useTranslation("common");
	const chips = [
		{ value: null, label: t("templateAllFlows", "All flows"), count: total },
		...groups.map((group) => ({
			value: group.name,
			label: group.name,
			count: group.count,
		})),
	];
	return (
		<fieldset className="flex min-w-0 flex-wrap gap-1.5">
			<legend className="sr-only">
				{t("templateFilterBySourceFlow", "Filter by source flow")}
			</legend>
			{chips.map((chip) => {
				const pressed = chip.value === active;
				return (
					<button
						key={chip.value ?? ""}
						type="button"
						aria-pressed={pressed}
						onClick={() => onPick(chip.value)}
						className={cn(
							"inline-flex h-8 max-w-56 items-center gap-1.5 rounded-full border px-3 text-xs font-medium transition-colors",
							pressed
								? "border-foreground bg-foreground text-background"
								: "border-border/70 bg-card text-foreground hover:bg-muted/60",
						)}
					>
						<span className="truncate">{chip.label}</span>
						<span
							className={cn(
								"tabular-nums",
								pressed ? "text-background/70" : "text-muted-foreground",
							)}
						>
							{chip.count}
						</span>
					</button>
				);
			})}
		</fieldset>
	);
}

function EmptyTemplates({
	canCreate,
	onCreate,
}: Readonly<{ canCreate: boolean; onCreate: () => void }>) {
	const { t } = useTranslation("common");
	return (
		<div className="flex flex-col items-center gap-3 rounded-xl border border-dashed border-border/60 bg-card/60 px-6 py-16 text-center">
			<span className="flex size-12 items-center justify-center rounded-full bg-primary/10 text-primary">
				<CopyIcon aria-hidden className="size-5" />
			</span>
			<p className="text-base font-semibold">
				{t("noTemplatesYet", "No templates yet")}
			</p>
			<p className="max-w-md text-sm text-muted-foreground">
				{t(
					"templatesEmptyExplanation",
					"Templates let you snapshot a flow at a specific version so you can share, reuse, or roll back to it later.",
				)}
			</p>
			{canCreate ? (
				<Button onClick={onCreate} className="mt-1">
					<PlusIcon aria-hidden className="size-4" />
					{t("createYourFirstTemplate", "Create Your First Template")}
				</Button>
			) : null}
		</div>
	);
}
