"use client";

import { useTranslation } from "@flow-like/locales";
import { createId } from "@paralleldrive/cuid2";
import {
	Cloud,
	ExternalLink,
	FileText,
	Loader2,
	Monitor,
	MoreHorizontal,
	PlusIcon,
	Settings,
	Shuffle,
	Trash2,
} from "lucide-react";
import { useCallback, useState } from "react";
import { useInvalidateInvoke, useInvoke } from "../../hooks";
import {
	type IBoard,
	IExecutionMode,
	IExecutionStage,
	ILogLevel,
	IVersionType,
} from "../../lib";
import { useBackend } from "../../state/backend-state";
import type { PageListItem } from "../../state/backend-state/page-state";
import {
	Badge,
	Button,
	Card,
	CardHeader,
	CardTitle,
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuLabel,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
	Input,
	Label,
	ScrollArea,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Tabs,
	TabsContent,
	TabsList,
	TabsTrigger,
	Textarea,
} from "../ui";
export interface IBoardMeta {
	name: string;
	description: string;
	stage: IExecutionStage;
	logLevel: ILogLevel;
	executionMode: IExecutionMode;
}

export function BoardMeta({
	appId,
	boardId,
	board,
	version,
	closeMeta,
	selectVersion,
	onPageClick,
	isOffline,
}: Readonly<{
	appId: string;
	boardId: string;
	board: IBoard;
	version?: [number, number, number];
	closeMeta: () => void;
	selectVersion: (version?: [number, number, number]) => void;
	onPageClick?: (pageId: string) => void;
	isOffline?: boolean;
}>) {
	const { t } = useTranslation("flow");
	const [boardMeta, setBoardMeta] = useState<IBoardMeta>({
		name: board.name,
		description: board.description,
		stage: board.stage,
		logLevel: board.log_level,
		executionMode: board.execution_mode ?? IExecutionMode.Hybrid,
	});
	const [activeTab, setActiveTab] = useState<"settings" | "pages">("settings");
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const versions = useInvoke(
		backend.boardState.getBoardVersions,
		backend.boardState,
		[appId, boardId],
	);
	const pages = useInvoke(backend.pageState.getPages, backend.pageState, [
		appId,
		boardId,
	]);

	const [localVersion, setLocalVersion] = useState<
		[number, number, number] | undefined
	>(board.version as [number, number, number] | undefined);

	const invalidateBoard = useCallback(async () => {
		await invalidate(backend.boardState.getBoard, [appId, boardId]);
	}, [invalidate, appId, boardId, backend]);

	const saveMeta = useCallback(async () => {
		await backend.boardState.upsertBoard(
			appId,
			boardId,
			boardMeta.name,
			boardMeta.description,
			boardMeta.logLevel,
			boardMeta.stage,
			boardMeta.executionMode,
		);

		await invalidateBoard();
		closeMeta();
	}, [appId, boardId, board, boardMeta, backend, invalidateBoard, closeMeta]);

	const createVersion = useCallback(
		async (type: IVersionType) => {
			const newVersion = await backend.boardState.createBoardVersion(
				appId,
				boardId,
				type,
			);
			setLocalVersion(newVersion);
			await versions.refetch();
		},
		[appId, boardId, versions, backend],
	);

	return (
		<Dialog
			open={true}
			onOpenChange={async (open) => {
				if (!open) closeMeta();
			}}
		>
			<DialogContent className="max-w-lg">
				<DialogHeader>
					<DialogTitle>{t('manageBoard', 'Manage Board')}</DialogTitle>
					<DialogDescription>
						{t('configureBoardSettingsAndManagePages', 'Configure board settings and manage pages')}
					</DialogDescription>
				</DialogHeader>

				<Tabs
					value={activeTab}
					onValueChange={(v) => setActiveTab(v as "settings" | "pages")}
				>
					<TabsList className="grid w-full grid-cols-2">
						<TabsTrigger value="settings" className="gap-2">
							<Settings className="h-4 w-4" />
							{t('settings', 'Settings')}
						</TabsTrigger>
						<TabsTrigger value="pages" className="gap-2">
							<FileText className="h-4 w-4" />
							{t('pages', 'Pages')}
							{(pages.data?.length ?? 0) > 0 && (
								<Badge variant="secondary" className="ml-1 h-5 px-1.5">
									{pages.data?.length}
								</Badge>
							)}
						</TabsTrigger>
					</TabsList>

					<TabsContent value="settings" className="space-y-4 mt-4">
						<SettingsTab
							boardMeta={boardMeta}
							setBoardMeta={setBoardMeta}
							version={version}
							selectVersion={selectVersion}
							localVersion={localVersion}
							versions={versions.data ?? []}
							createVersion={createVersion}
							saveMeta={saveMeta}
							isOffline={isOffline}
						/>
					</TabsContent>

					<TabsContent value="pages" className="mt-4">
						<PagesTab
							appId={appId}
							boardId={boardId}
							pages={pages}
							onPageClick={onPageClick}
						/>
					</TabsContent>
				</Tabs>
			</DialogContent>
		</Dialog>
	);
}

function SettingsTab({
	boardMeta,
	setBoardMeta,
	version,
	selectVersion,
	localVersion,
	versions,
	createVersion,
	saveMeta,
	isOffline,
}: {
	boardMeta: IBoardMeta;
	setBoardMeta: React.Dispatch<React.SetStateAction<IBoardMeta>>;
	version?: [number, number, number];
	selectVersion: (v?: [number, number, number]) => void;
	localVersion?: [number, number, number];
	versions: [number, number, number][];
	createVersion: (type: IVersionType) => Promise<void>;
	saveMeta: () => Promise<void>;
	isOffline?: boolean;
}) {
	const { t } = useTranslation("flow");
	return (
		<>
			<div className="grid w-full items-center gap-1.5">
				<Label htmlFor="name">Name</Label>
				<Input
					value={boardMeta.name}
					onChange={(e) =>
						setBoardMeta((old) => ({ ...old, name: e.target.value }))
					}
					type="text"
					id="name"
					placeholder="Name"
				/>
			</div>
			<div className="grid w-full items-center gap-1.5">
				<Label htmlFor="description">{t('description', 'Description')}</Label>
				<Textarea
					value={boardMeta.description}
					onChange={(e) =>
						setBoardMeta((old) => ({
							...old,
							description: e.target.value,
						}))
					}
					id="description"
					placeholder="Description"
				/>
			</div>
			<div className="grid w-full items-center gap-1.5">
				<Label htmlFor="stage">{t('stage', 'Stage')}</Label>
				<Select
					value={boardMeta.stage}
					onValueChange={(e) =>
						setBoardMeta((old) => ({
							...old,
							stage: e as IExecutionStage,
						}))
					}
				>
					<SelectTrigger id="stage" className="w-full">
						<SelectValue placeholder="Stage" />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value={IExecutionStage.Dev}>{t('development', 'Development')}</SelectItem>
						<SelectItem value={IExecutionStage.Int}>{t('integration', 'Integration')}</SelectItem>
						<SelectItem value={IExecutionStage.QA}>QA</SelectItem>
						<SelectItem value={IExecutionStage.PreProd}>
							{t('preproduction', 'Pre-Production')}
						</SelectItem>
						<SelectItem value={IExecutionStage.Prod}>{t('production', 'Production')}</SelectItem>
					</SelectContent>
				</Select>
			</div>
			<div className="grid w-full items-center gap-1.5">
				<Label htmlFor="log-level">{t('logLevel', 'Log Level')}</Label>
				<Select
					value={boardMeta.logLevel}
					onValueChange={(e) =>
						setBoardMeta((old) => ({ ...old, logLevel: e as ILogLevel }))
					}
				>
					<SelectTrigger id="log-level" className="w-full">
						<SelectValue placeholder={t('logLevel', 'Log Level')} />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value={ILogLevel.Debug}>{t('debug', 'Debug')}</SelectItem>
						<SelectItem value={ILogLevel.Info}>{t('info', 'Info')}</SelectItem>
						<SelectItem value={ILogLevel.Warn}>{t('warning', 'Warning')}</SelectItem>
						<SelectItem value={ILogLevel.Error}>{t('error', 'Error')}</SelectItem>
						<SelectItem value={ILogLevel.Fatal}>{t('fatal', 'Fatal')}</SelectItem>
					</SelectContent>
				</Select>
			</div>
			<div className="grid w-full items-center gap-1.5">
				<Label htmlFor="execution-mode">{t('executionMode', 'Execution Mode')}</Label>
				{isOffline ? (
					<>
						<div className="flex items-center gap-2 h-10 px-3 py-2 rounded-md border border-input bg-background text-sm">
							<Monitor className="h-4 w-4" />
							<span>{t('local', 'Local')}</span>
						</div>
						<p className="text-xs text-muted-foreground">
							{t('offlineProjectsOnlySupportLocalExecution', 'Offline projects only support local execution.')}
						</p>
					</>
				) : (
					<>
						<Select
							value={boardMeta.executionMode}
							onValueChange={(e) =>
								setBoardMeta((old) => ({
									...old,
									executionMode: e as IExecutionMode,
								}))
							}
						>
							<SelectTrigger id="execution-mode" className="w-full">
								<SelectValue placeholder={t('executionMode', 'Execution Mode')} />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value={IExecutionMode.Hybrid}>
									<div className="flex items-center gap-2">
										<Shuffle className="h-4 w-4" />
										<span>{t('hybrid', 'Hybrid')}</span>
									</div>
								</SelectItem>
								<SelectItem value={IExecutionMode.Remote}>
									<div className="flex items-center gap-2">
										<Cloud className="h-4 w-4" />
										<span>{t('remote', 'Remote')}</span>
									</div>
								</SelectItem>
								<SelectItem value={IExecutionMode.Local}>
									<div className="flex items-center gap-2">
										<Monitor className="h-4 w-4" />
										<span>{t('local', 'Local')}</span>
									</div>
								</SelectItem>
							</SelectContent>
						</Select>
						<p className="text-xs text-muted-foreground">
							{boardMeta.executionMode === IExecutionMode.Hybrid &&
								t('runsLocallyWhenPossibleFallsBackToRemoteExecution', 'Runs locally when possible, falls back to remote execution.')}
							{boardMeta.executionMode === IExecutionMode.Remote &&
								t('alwaysRunsOnRemoteServersRequiredForBoardsWithSecrets', 'Always runs on remote servers. Required for boards with secrets.')}
							{boardMeta.executionMode === IExecutionMode.Local &&
								t('alwaysRunsLocallyBestForHighperformanceWorkloadsLikeEmbeddings', 'Always runs locally. Best for high-performance workloads like embeddings.')}
						</p>
					</>
				)}
			</div>
			<div className="grid w-full items-center gap-1.5">
				<Label htmlFor="version">{t('version', 'Version')}</Label>
				<Select
					value={version ? version.join(".") : "Latest"}
					onValueChange={(e) => {
						if (e === "Latest") {
							selectVersion(undefined);
						} else {
							const v = e.split(".").map(Number) as [number, number, number];
							selectVersion(v);
						}
					}}
				>
					<SelectTrigger id="version" className="w-full">
						<SelectValue placeholder="Version" />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="Latest">
							{t("latestVersion", "Latest ({{version}})", {
								version: localVersion?.join(".") ?? "—",
							})}
						</SelectItem>
						{versions
							?.sort((a, b) => {
								if (a[0] !== b[0]) return b[0] - a[0];
								if (a[1] !== b[1]) return b[1] - a[1];
								return b[2] - a[2];
							})
							.map((v) => (
								<SelectItem key={v.join(".")} value={v.join(".")}>
									{v.join(".")}
								</SelectItem>
							))}
					</SelectContent>
				</Select>
			</div>
			<div className="w-full flex flex-row gap-2 overflow-hidden">
				<DropdownMenu>
					<DropdownMenuTrigger asChild>
						<Button variant="secondary" className="w-1/3">
							{t('createVersion', 'Create Version')}
						</Button>
					</DropdownMenuTrigger>
					<DropdownMenuContent>
						<DropdownMenuLabel>{t('versionType', 'Version Type')}</DropdownMenuLabel>
						<DropdownMenuSeparator />
						<DropdownMenuItem onClick={() => createVersion(IVersionType.Major)}>
							{t('major', 'Major')}
						</DropdownMenuItem>
						<DropdownMenuItem onClick={() => createVersion(IVersionType.Minor)}>
							{t('minor', 'Minor')}
						</DropdownMenuItem>
						<DropdownMenuItem onClick={() => createVersion(IVersionType.Patch)}>
							{t('patch', 'Patch')}
						</DropdownMenuItem>
					</DropdownMenuContent>
				</DropdownMenu>

				<Button className="flex-grow" onClick={saveMeta}>
					{t('save', 'Save')}
				</Button>
			</div>
		</>
	);
}

interface PagesQueryResult {
	data?: PageListItem[];
	isLoading?: boolean;
	refetch: () => Promise<unknown>;
}

function PagesTab({
	appId,
	boardId,
	pages,
	onPageClick,
}: {
	appId: string;
	boardId: string;
	pages: PagesQueryResult;
	onPageClick?: (pageId: string) => void;
}) {
	const { t } = useTranslation("flow");
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const [createDialog, setCreateDialog] = useState(false);
	const [newPage, setNewPage] = useState({ name: "", route: "/" });
	const [isCreating, setIsCreating] = useState(false);

	const handleCreate = useCallback(async () => {
		if (!newPage.name.trim()) return;
		if (!appId) {
			console.error("Cannot create page: appId is missing");
			return;
		}
		setIsCreating(true);
		try {
			await backend.pageState.createPage(
				appId,
				createId(),
				newPage.name.trim(),
				newPage.route.trim() || "/",
				boardId,
			);
			await invalidate(backend.pageState.getPages, [appId, boardId]);
			await pages.refetch();
			setNewPage({ name: "", route: "/" });
			setCreateDialog(false);
		} finally {
			setIsCreating(false);
		}
	}, [appId, boardId, newPage, backend, invalidate, pages]);

	const handleDelete = useCallback(
		async (pageId: string) => {
			await backend.pageState.deletePage(appId, pageId, boardId);
			await invalidate(backend.pageState.getPages, [appId, boardId]);
			await pages.refetch();
		},
		[appId, boardId, backend, invalidate, pages],
	);

	return (
		<div className="space-y-4">
			<div className="flex items-center justify-between">
				<p className="text-sm text-muted-foreground">
					{pages.data?.length ?? 0} page
					{(pages.data?.length ?? 0) !== 1 ? "s" : ""}
				</p>
				<Button
					size="sm"
					onClick={() => setCreateDialog(true)}
					disabled={!appId}
					title={
						!appId ? t('cannotCreatePagesAppContextIsMissing', 'Cannot create pages: app context is missing') : undefined
					}
				>
					<PlusIcon className="h-4 w-4 mr-1" />
					{t('addPage', 'Add Page')}
				</Button>
			</div>

			<ScrollArea className="h-[280px] -mx-1 px-1">
				{pages.isLoading && (
					<div className="flex items-center justify-center py-8">
						<Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
					</div>
				)}

				{!pages.isLoading && (pages.data?.length ?? 0) === 0 && (
					<div className="text-center py-8 text-muted-foreground">
						<FileText className="h-10 w-10 mx-auto mb-3 opacity-50" />
						<p className="text-sm">{t('noPagesYet', 'No pages yet')}</p>
						<p className="text-xs mt-1">
							{`Pages let you build UIs that connect to this flow`}
						</p>
					</div>
				)}

				<div className="space-y-2 pr-2">
					{pages.data?.map((pageInfo) => (
						<PageCard
							key={pageInfo.pageId}
							pageId={pageInfo.pageId}
							name={pageInfo.name}
							description={pageInfo.description}
							onClick={() => onPageClick?.(pageInfo.pageId)}
							onDelete={() => handleDelete(pageInfo.pageId)}
						/>
					))}
				</div>
			</ScrollArea>

			<Dialog open={createDialog} onOpenChange={setCreateDialog}>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>{t('createPage', 'Create Page')}</DialogTitle>
						<DialogDescription>
							{`Add a new page to this flow. Pages can render UI components.`}
						</DialogDescription>
					</DialogHeader>
					<div className="space-y-4 py-4">
						<div className="space-y-2">
							<Label htmlFor="page-name">Name</Label>
							<Input
								id="page-name"
								placeholder={t('homePage', 'Home Page')}
								value={newPage.name}
								onChange={(e) =>
									setNewPage((p) => ({ ...p, name: e.target.value }))
								}
							/>
						</div>
						<div className="space-y-2">
							<Label htmlFor="page-route">{t('route', 'Route')}</Label>
							<Input
								id="page-route"
								placeholder="/"
								value={newPage.route}
								onChange={(e) =>
									setNewPage((p) => ({ ...p, route: e.target.value }))
								}
							/>
							<p className="text-xs text-muted-foreground">
								{t('theUrlPathForThisPageEgAboutContact', 'The URL path for this page (e.g., /about, /contact)')}
							</p>
						</div>
					</div>
					<DialogFooter>
						<Button variant="outline" onClick={() => setCreateDialog(false)}>
							{t('cancel', 'Cancel')}
						</Button>
						<Button
							onClick={handleCreate}
							disabled={isCreating || !newPage.name.trim()}
						>
							{isCreating && <Loader2 className="h-4 w-4 mr-2 animate-spin" />}
							{t('create', 'Create')}
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>
		</div>
	);
}

function PageCard({
	pageId,
	name,
	description,
	onClick,
	onDelete,
}: {
	pageId: string;
	name: string;
	description?: string;
	onClick?: () => void;
	onDelete?: () => void;
}) {
	const { t } = useTranslation("flow");
	return (
		<Card
			className="group cursor-pointer hover:bg-accent/50 transition-all hover:shadow-md border-muted/50"
			onClick={onClick}
		>
			<CardHeader className="p-3 space-y-2">
				<div className="flex items-start justify-between gap-2">
					<div className="flex items-center gap-2.5 min-w-0 flex-1">
						<div className="h-9 w-9 rounded-md bg-gradient-to-br from-primary/20 to-primary/5 flex items-center justify-center shrink-0 group-hover:from-primary/30 group-hover:to-primary/10 transition-colors">
							<FileText className="h-4 w-4 text-primary" />
						</div>
						<div className="min-w-0 flex-1">
							<CardTitle className="text-sm font-medium truncate">
								{name || pageId}
							</CardTitle>
						</div>
					</div>
					<DropdownMenu>
						<DropdownMenuTrigger asChild>
							<Button
								variant="ghost"
								size="sm"
								className="h-7 w-7 p-0 opacity-0 group-hover:opacity-100 transition-opacity"
								onClick={(e) => e.stopPropagation()}
							>
								<MoreHorizontal className="h-4 w-4" />
							</Button>
						</DropdownMenuTrigger>
						<DropdownMenuContent align="end">
							<DropdownMenuItem onClick={onClick}>
								<ExternalLink className="h-4 w-4 mr-2" />
								{t('openInBuilder', 'Open in Builder')}
							</DropdownMenuItem>
							<DropdownMenuSeparator />
							<DropdownMenuItem
								className="text-destructive focus:text-destructive"
								onClick={(e) => {
									e.stopPropagation();
									onDelete?.();
								}}
							>
								<Trash2 className="h-4 w-4 mr-2" />
								{t('deletePage', 'Delete Page')}
							</DropdownMenuItem>
						</DropdownMenuContent>
					</DropdownMenu>
				</div>
				{description && (
					<p className="text-xs text-muted-foreground line-clamp-2 pl-11">
						{description}
					</p>
				)}
			</CardHeader>
		</Card>
	);
}
