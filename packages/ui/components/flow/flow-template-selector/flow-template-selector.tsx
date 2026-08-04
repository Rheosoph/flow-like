"use client";
import {
	ArrowRight,
	ChevronLeft,
	ChevronRight,
	Grid3X3,
	LayoutTemplate,
	List,
	Loader2,
	Search,
	Sparkles,
	X,
} from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { useInvoke } from "../../../hooks";
import { useSearch } from "../../../hooks/use-search-index";
import {
	hashToGradient,
	useThemeInfo,
} from "../../../hooks/use-theme-gradient";
import { useBackend } from "../../../state/backend-state";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import { Dialog, DialogContent } from "../../ui/dialog";
import { Input } from "../../ui/input";
import { ScrollArea } from "../../ui/scroll-area";
import { FlowPreview } from "../flow-preview";
import type {
	AppWithTemplates,
	FlowTemplateSelectorProps,
	TemplateInfo,
} from "./types";

export function FlowTemplateSelector({
	onSelectTemplate,
	onDismiss,
}: FlowTemplateSelectorProps) {
	const backend = useBackend();
	const [browserDialogOpen, setBrowserDialogOpen] = useState(false);
	const [previewTemplate, setPreviewTemplate] = useState<TemplateInfo | null>(
		null,
	);
	const [browserPreview, setBrowserPreview] = useState<TemplateInfo | null>(
		null,
	);
	const [selectedCategory, setSelectedCategory] = useState<string | null>(null);
	const [searchQuery, setSearchQuery] = useState("");
	const [isApplying, setIsApplying] = useState(false);
	const [viewMode, setViewMode] = useState<"grid" | "list">("grid");

	const apps = useInvoke(backend.appState.getApps, backend.appState, []);
	const templates = useInvoke(
		backend.templateState.getTemplates,
		backend.templateState,
		[undefined],
	);

	const appsWithTemplates = useMemo<AppWithTemplates[]>(() => {
		if (!apps.data || !templates.data) return [];

		const templatesByApp = new Map<
			string,
			{ templateId: string; metadata?: any }[]
		>();

		for (const template of templates.data) {
			const [appId, templateId, metadata] = template;
			const existing = templatesByApp.get(appId) || [];
			existing.push({ templateId, metadata });
			templatesByApp.set(appId, existing);
		}

		const result: AppWithTemplates[] = [];

		for (const appData of apps.data) {
			const [app, appMeta] = appData;
			const appTemplates = templatesByApp.get(app.id);
			if (appTemplates && appTemplates.length > 0) {
				result.push({
					app,
					appMetadata: appMeta,
					templates: appTemplates.map((t) => ({
						appId: app.id,
						templateId: t.templateId,
						metadata: t.metadata,
					})),
				});
			}
		}

		return result;
	}, [apps.data, templates.data]);

	const allTemplates = useMemo(() => {
		const result: TemplateInfo[] = [];
		for (const item of appsWithTemplates) {
			result.push(...item.templates);
		}
		return result;
	}, [appsWithTemplates]);

	// Show diverse templates - prioritize one from each app, then fill remaining slots
	const quickTemplates = useMemo(() => {
		const result: TemplateInfo[] = [];
		const seenApps = new Set<string>();

		// First pass: one template per app
		for (const item of appsWithTemplates) {
			if (result.length >= 5) break;
			if (item.templates.length > 0 && !seenApps.has(item.app.id)) {
				result.push(item.templates[0]);
				seenApps.add(item.app.id);
			}
		}

		// Second pass: fill remaining slots with more templates
		if (result.length < 5) {
			for (const template of allTemplates) {
				if (result.length >= 5) break;
				if (
					!result.some(
						(t) =>
							t.appId === template.appId &&
							t.templateId === template.templateId,
					)
				) {
					result.push(template);
				}
			}
		}

		return result;
	}, [appsWithTemplates, allTemplates]);

	const getAppMetaForTemplate = useCallback(
		(template: TemplateInfo) => {
			return appsWithTemplates.find((a) => a.app.id === template.appId)
				?.appMetadata;
		},
		[appsWithTemplates],
	);

	const categoryTemplates = useMemo(() => {
		if (!selectedCategory) return allTemplates;
		const app = appsWithTemplates.find((a) => a.app.id === selectedCategory);
		return app?.templates ?? [];
	}, [allTemplates, appsWithTemplates, selectedCategory]);

	const filteredTemplates = useSearch(categoryTemplates, searchQuery, {
		fields: [
			"metadata.name",
			"templateId",
			"metadata.description",
			"metadata.long_description",
			"metadata.tags",
		],
		boost: { "metadata.name": 3, "metadata.tags": 1.5 },
	});

	const handleApplyTemplate = useCallback(
		async (template: TemplateInfo) => {
			setIsApplying(true);
			try {
				await onSelectTemplate(template.appId, template.templateId);
				setPreviewTemplate(null);
				setBrowserDialogOpen(false);
			} finally {
				setIsApplying(false);
			}
		},
		[onSelectTemplate],
	);

	const openPreview = useCallback((template: TemplateInfo) => {
		setPreviewTemplate(template);
	}, []);

	const isLoading = apps.isLoading || templates.isLoading;
	const isFetching = apps.isFetching || templates.isFetching;
	const hasError = apps.isError || templates.isError;
	const hasTemplates = appsWithTemplates.length > 0;

	if (isLoading || (isFetching && !hasTemplates)) {
		return (
			<div className="absolute right-6 top-1/2 -translate-y-1/2 pointer-events-auto z-60">
				<div className="bg-background/95 backdrop-blur-xl border border-border/40 rounded-2xl shadow-xl p-6">
					<div className="flex items-center gap-3">
						<Loader2 className="h-5 w-5 animate-spin text-primary" />
						<p className="text-sm text-muted-foreground">
							Loading templates...
						</p>
					</div>
				</div>
			</div>
		);
	}

	if (hasError) {
		return (
			<div className="absolute right-6 top-1/2 -translate-y-1/2 pointer-events-auto z-60">
				<div className="bg-background/95 backdrop-blur-xl border border-border/40 rounded-2xl shadow-xl p-6 w-72">
					<div className="flex flex-col items-center text-center">
						<div className="h-12 w-12 rounded-xl bg-destructive/10 flex items-center justify-center mb-3">
							<LayoutTemplate className="h-6 w-6 text-destructive/50" />
						</div>
						<p className="text-sm font-medium text-foreground mb-1">
							Failed to load templates
						</p>
						<p className="text-xs text-muted-foreground mb-4">
							Please try again
						</p>
						<Button
							variant="outline"
							size="sm"
							onClick={() => {
								apps.refetch();
								templates.refetch();
							}}
							className="w-full"
						>
							Retry
						</Button>
					</div>
				</div>
			</div>
		);
	}

	if (!hasTemplates) {
		return (
			<div className="absolute right-6 top-1/2 -translate-y-1/2 pointer-events-auto z-60">
				<div className="bg-background/95 backdrop-blur-xl border border-border/40 rounded-2xl shadow-xl p-6 w-72">
					<div className="flex flex-col items-center text-center">
						<div className="h-12 w-12 rounded-xl bg-muted/50 flex items-center justify-center mb-3">
							<LayoutTemplate className="h-6 w-6 text-muted-foreground/50" />
						</div>
						<p className="text-sm font-medium text-foreground mb-1">
							No templates yet
						</p>
						<p className="text-xs text-muted-foreground mb-4">
							Start building from scratch
						</p>
						{onDismiss && (
							<Button
								variant="default"
								size="sm"
								onClick={onDismiss}
								className="w-full"
							>
								<Sparkles className="h-4 w-4 mr-2" />
								Start building
							</Button>
						)}
					</div>
				</div>
			</div>
		);
	}

	return (
		<>
			{/* Clean panel - responsive positioning, expands when previewing */}
			<div className="absolute inset-x-4 bottom-20 md:inset-auto md:right-6 md:top-1/2 md:-translate-y-1/2 md:bottom-auto pointer-events-auto z-60">
				<div
					className={`bg-background/95 backdrop-blur-xl border border-border/40 rounded-2xl shadow-xl overflow-hidden flex flex-col transition-all duration-200 ${
						previewTemplate
							? "w-full md:w-96 max-h-[70vh]"
							: "w-full md:w-80 max-h-[60vh] md:max-h-none"
					}`}
				>
					{previewTemplate ? (
						/* Inline Preview Mode */
						<InlineTemplatePreview
							template={previewTemplate}
							appIcon={getAppMetaForTemplate(previewTemplate)?.icon}
							isApplying={isApplying}
							onApply={() => handleApplyTemplate(previewTemplate)}
							onBack={() => setPreviewTemplate(null)}
							onDismiss={onDismiss}
						/>
					) : (
						/* Template List Mode */
						<>
							{/* Header */}
							<div className="flex items-center justify-between px-4 pt-4 pb-2 shrink-0">
								<h3 className="text-sm font-semibold text-foreground">
									Start with a template
								</h3>
								{onDismiss && (
									<button
										type="button"
										onClick={onDismiss}
										className="p-1.5 -mr-1 rounded-lg text-muted-foreground hover:text-foreground hover:bg-muted/50 transition-colors"
									>
										<X className="h-4 w-4" />
									</button>
								)}
							</div>

							{/* Template list - scrollable */}
							<div className="px-2 pb-2 overflow-y-auto flex-1 min-h-0">
								{quickTemplates.map((template) => {
									const appMeta = getAppMetaForTemplate(template);
									return (
										<QuickTemplateItem
											key={`${template.appId}-${template.templateId}`}
											template={template}
											appIcon={appMeta?.icon}
											appName={
												appsWithTemplates.length > 1 ? appMeta?.name : undefined
											}
											onClick={() => openPreview(template)}
										/>
									);
								})}
							</div>

							{/* Browse all - simple link style like FigJam */}
							<div className="px-4 pb-4 pt-1 shrink-0 border-t border-border/30">
								<button
									type="button"
									onClick={() => setBrowserDialogOpen(true)}
									className="w-full flex items-center justify-between py-2 px-3 text-sm text-muted-foreground hover:text-foreground hover:bg-muted/50 rounded-lg transition-colors group"
								>
									<span className="flex items-center gap-2">
										<Grid3X3 className="h-4 w-4" />
										Browse all templates
									</span>
									<ArrowRight className="h-4 w-4 opacity-0 -translate-x-1 group-hover:opacity-100 group-hover:translate-x-0 transition-all md:opacity-0" />
								</button>
							</div>
						</>
					)}
				</div>
			</div>

			{/* Full Template Browser Dialog */}
			<Dialog
				open={browserDialogOpen}
				onOpenChange={(open) => {
					setBrowserDialogOpen(open);
					if (!open) setBrowserPreview(null);
				}}
			>
				<DialogContent
					className="min-w-[95vw] h-[85vh] p-0 gap-0 overflow-hidden z-100"
					showCloseButton={false}
				>
					<div className="flex flex-col md:flex-row h-full">
						{/* Sidebar - horizontal scroll on mobile, sidebar on desktop */}
						<div className="w-full max-h-[40vh] md:max-h-none md:w-56 border-b md:border-b-0 md:border-r border-border/50 flex flex-col bg-muted/20 shrink-0">
							{/* Header */}
							<div className="p-3 md:p-4 md:border-b border-border/50 shrink-0">
								<h2 className="text-base md:text-lg font-semibold mb-2 md:mb-3">
									Templates
								</h2>
								<div className="relative">
									<Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
									<Input
										value={searchQuery}
										onChange={(e) => setSearchQuery(e.target.value)}
										placeholder="Search templates..."
										className="pl-9 h-9 text-sm bg-background"
									/>
								</div>
							</div>

							{/* Categories - horizontal scroll on mobile, vertical list on desktop */}
							<div className="flex-1 overflow-x-auto overflow-y-auto md:overflow-x-visible">
								<div className="flex md:flex-col gap-1 p-2 md:space-y-1 min-w-max md:min-w-0">
									<button
										type="button"
										onClick={() => setSelectedCategory(null)}
										className={`shrink-0 md:w-full text-left px-3 py-2 rounded-lg text-sm font-medium transition-colors ${
											selectedCategory === null
												? "bg-primary text-primary-foreground"
												: "text-foreground hover:bg-muted"
										}`}
									>
										<div className="flex items-center gap-2 md:justify-between">
											<span>All</span>
											<span
												className={`text-xs ${selectedCategory === null ? "text-primary-foreground/70" : "text-muted-foreground"}`}
											>
												{allTemplates.length}
											</span>
										</div>
									</button>

									{appsWithTemplates.map((item) => (
										<button
											key={item.app.id}
											type="button"
											onClick={() => setSelectedCategory(item.app.id)}
											className={`shrink-0 md:w-full text-left px-3 py-2 rounded-lg text-sm transition-colors ${
												selectedCategory === item.app.id
													? "bg-primary text-primary-foreground"
													: "text-foreground hover:bg-muted"
											}`}
										>
											<div className="flex items-center gap-2 md:justify-between">
												<span className="truncate">
													{item.appMetadata?.name || item.app.id}
												</span>
												<span
													className={`text-xs ${selectedCategory === item.app.id ? "text-primary-foreground/70" : "text-muted-foreground"}`}
												>
													{item.templates.length}
												</span>
											</div>
										</button>
									))}
								</div>
							</div>
						</div>

						{/* Main content */}
						<div className="flex-1 flex flex-col min-w-0 bg-background overflow-hidden">
							{/* Toolbar */}
							<div className="flex items-center justify-between p-4 border-b border-border/50">
								<div>
									<h3 className="text-base font-semibold">
										{selectedCategory
											? (appsWithTemplates.find(
													(a) => a.app.id === selectedCategory,
												)?.appMetadata?.name ?? "Templates")
											: "All templates"}
									</h3>
									<p className="text-xs text-muted-foreground mt-0.5">
										{filteredTemplates.length} template
										{filteredTemplates.length !== 1 ? "s" : ""}
									</p>
								</div>
								<div className="flex items-center gap-2">
									<div className="flex items-center gap-1 bg-muted/50 p-1 rounded-lg">
										<button
											type="button"
											onClick={() => setViewMode("grid")}
											className={`p-1.5 rounded-md transition-colors ${viewMode === "grid" ? "bg-background shadow-sm" : "hover:bg-background/50"}`}
										>
											<Grid3X3 className="h-4 w-4" />
										</button>
										<button
											type="button"
											onClick={() => setViewMode("list")}
											className={`p-1.5 rounded-md transition-colors ${viewMode === "list" ? "bg-background shadow-sm" : "hover:bg-background/50"}`}
										>
											<List className="h-4 w-4" />
										</button>
									</div>
									<button
										type="button"
										onClick={() => setBrowserDialogOpen(false)}
										className="p-1.5 rounded-lg text-muted-foreground hover:text-foreground hover:bg-muted/50 transition-colors"
									>
										<X className="h-4 w-4" />
									</button>
								</div>
							</div>

							{/* Template Grid/List */}
							<ScrollArea className="flex-1 min-h-0">
								{viewMode === "grid" ? (
									<div className="p-4 grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4">
										{filteredTemplates.map((template) => {
											const appMeta = getAppMetaForTemplate(template);
											const isSelected =
												browserPreview?.appId === template.appId &&
												browserPreview?.templateId === template.templateId;
											return (
												<BrowserTemplateCard
													key={`${template.appId}-${template.templateId}`}
													template={template}
													appIcon={appMeta?.icon}
													appThumbnail={appMeta?.thumbnail}
													appName={
														!selectedCategory ? appMeta?.name : undefined
													}
													isSelected={isSelected}
													onClick={() => setBrowserPreview(template)}
												/>
											);
										})}
									</div>
								) : (
									<div className="p-4 space-y-2">
										{filteredTemplates.map((template) => {
											const appMeta = getAppMetaForTemplate(template);
											const isSelected =
												browserPreview?.appId === template.appId &&
												browserPreview?.templateId === template.templateId;
											return (
												<BrowserTemplateRow
													key={`${template.appId}-${template.templateId}`}
													template={template}
													appIcon={appMeta?.icon}
													appName={
														!selectedCategory ? appMeta?.name : undefined
													}
													isSelected={isSelected}
													onClick={() => setBrowserPreview(template)}
												/>
											);
										})}
									</div>
								)}

								{filteredTemplates.length === 0 && (
									<div className="flex flex-col items-center justify-center py-16 text-center">
										<Search className="h-12 w-12 text-muted-foreground/30 mb-4" />
										<p className="text-sm font-medium text-foreground mb-1">
											No templates found
										</p>
										<p className="text-xs text-muted-foreground">
											Try a different search term or category
										</p>
									</div>
								)}
							</ScrollArea>
						</div>

						{/* Preview Panel - shows when a template is selected */}
						{browserPreview && (
							<div className="w-full md:w-80 border-t md:border-t-0 md:border-l border-border/50 flex flex-col bg-background shrink-0">
								<BrowserPreviewPane
									template={browserPreview}
									appIcon={getAppMetaForTemplate(browserPreview)?.icon}
									isApplying={isApplying}
									onApply={() => handleApplyTemplate(browserPreview)}
									onClose={() => setBrowserPreview(null)}
								/>
							</div>
						)}
					</div>
				</DialogContent>
			</Dialog>
		</>
	);
}

function InlineTemplatePreview({
	template,
	appIcon,
	isApplying,
	onApply,
	onBack,
	onDismiss,
}: {
	template: TemplateInfo;
	appIcon?: string | null;
	isApplying: boolean;
	onApply: () => void;
	onBack: () => void;
	onDismiss?: () => void;
}) {
	const { primaryHue, isDark } = useThemeInfo();
	const backend = useBackend();
	const templateBoard = useInvoke(
		backend.templateState.getTemplate,
		backend.templateState,
		[template.appId, template.templateId],
		true,
	);

	const nodeCount = templateBoard.data
		? Object.keys(templateBoard.data.nodes).length
		: 0;
	const commentCount = templateBoard.data
		? Object.keys(templateBoard.data.comments).length
		: 0;

	return (
		<div className="flex flex-col h-full">
			{/* Header with back button */}
			<div className="flex items-center gap-2 px-3 pt-3 pb-2 shrink-0">
				<button
					type="button"
					onClick={onBack}
					className="p-1.5 rounded-lg text-muted-foreground hover:text-foreground hover:bg-muted/50 transition-colors"
				>
					<ChevronLeft className="h-4 w-4" />
				</button>
				<h3 className="text-sm font-semibold text-foreground flex-1 truncate">
					{template.metadata?.name || template.templateId}
				</h3>
				{onDismiss && (
					<button
						type="button"
						onClick={onDismiss}
						className="p-1.5 rounded-lg text-muted-foreground hover:text-foreground hover:bg-muted/50 transition-colors"
					>
						<X className="h-4 w-4" />
					</button>
				)}
			</div>

			{/* Preview area */}
			<div className="mx-3 h-40 rounded-lg bg-muted/30 border border-border/50 overflow-hidden">
				{templateBoard.isLoading ? (
					<div className="w-full h-full flex items-center justify-center">
						<Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
					</div>
				) : templateBoard.data ? (
					<FlowPreview
						nodes={Object.values(templateBoard.data.nodes)}
						comments={templateBoard.data.comments}
						layers={templateBoard.data.layers}
						variables={templateBoard.data.variables}
					/>
				) : (
					<div className="w-full h-full flex items-center justify-center">
						<p className="text-xs text-muted-foreground">
							Preview not available
						</p>
					</div>
				)}
			</div>

			{/* Content - scrollable */}
			<div className="flex-1 overflow-y-auto min-h-0 px-3 py-3 space-y-3">
				{/* Icon and tags */}
				<div className="flex items-start gap-3">
					{appIcon ? (
						<img
							src={appIcon}
							alt=""
							className="h-10 w-10 rounded-lg object-cover shrink-0"
						/>
					) : (
						(() => {
							const g = hashToGradient(template.appId, primaryHue, isDark);
							return (
								<div
									className="h-10 w-10 rounded-lg flex items-center justify-center shrink-0"
									style={{
										background: `linear-gradient(${g.angle}deg, ${g.from}, ${g.to})`,
									}}
								>
									<LayoutTemplate className="h-5 w-5 text-white/80" />
								</div>
							);
						})()
					)}
					<div className="flex-1 min-w-0">
						{template.metadata?.tags && template.metadata.tags.length > 0 && (
							<div className="flex flex-wrap gap-1">
								{template.metadata.tags.slice(0, 3).map((tag: string) => (
									<Badge key={tag} variant="secondary" className="text-xs">
										{tag}
									</Badge>
								))}
							</div>
						)}
					</div>
				</div>

				{/* Description */}
				{template.metadata?.description && (
					<p className="text-xs text-muted-foreground leading-relaxed">
						{template.metadata.description}
					</p>
				)}

				{/* Stats */}
				<div className="flex gap-2">
					<div className="flex-1 p-2 rounded-lg bg-muted/30 border border-border/50 text-center">
						<div className="text-lg font-semibold text-foreground">
							{templateBoard.isLoading ? "..." : nodeCount}
						</div>
						<div className="text-xs text-muted-foreground">Nodes</div>
					</div>
					<div className="flex-1 p-2 rounded-lg bg-muted/30 border border-border/50 text-center">
						<div className="text-lg font-semibold text-foreground">
							{templateBoard.isLoading ? "..." : commentCount}
						</div>
						<div className="text-xs text-muted-foreground">Comments</div>
					</div>
				</div>
			</div>

			{/* Footer */}
			<div className="p-3 border-t border-border/30 shrink-0">
				<Button
					size="sm"
					onClick={onApply}
					disabled={isApplying}
					className="w-full"
				>
					{isApplying ? (
						<>
							<Loader2 className="h-3.5 w-3.5 mr-1.5 animate-spin" />
							Applying...
						</>
					) : (
						<>
							<Sparkles className="h-3.5 w-3.5 mr-1.5" />
							Use template
						</>
					)}
				</Button>
			</div>
		</div>
	);
}

function QuickTemplateItem({
	template,
	appIcon,
	appName,
	onClick,
}: {
	template: TemplateInfo;
	appIcon?: string | null;
	appName?: string;
	onClick: () => void;
}) {
	const { primaryHue, isDark } = useThemeInfo();
	return (
		<button
			type="button"
			onClick={onClick}
			className="w-full flex items-center gap-3 p-2 rounded-lg hover:bg-muted/50 transition-colors group text-left"
		>
			{appIcon ? (
				<img
					src={appIcon}
					alt=""
					className="h-10 w-10 rounded-lg object-cover shrink-0"
				/>
			) : (
				(() => {
					const g = hashToGradient(template.appId, primaryHue, isDark);
					return (
						<div
							className="h-10 w-10 rounded-lg flex items-center justify-center shrink-0 transition-opacity group-hover:opacity-80"
							style={{
								background: `linear-gradient(${g.angle}deg, ${g.from}, ${g.to})`,
							}}
						>
							<LayoutTemplate className="h-5 w-5 text-white/80 group-hover:text-white transition-colors" />
						</div>
					);
				})()
			)}
			<div className="flex-1 min-w-0">
				<div className="flex items-center gap-1.5">
					<p className="text-sm font-medium text-foreground truncate group-hover:text-primary transition-colors">
						{template.metadata?.name || template.templateId}
					</p>
					{appName && (
						<span className="text-[10px] text-muted-foreground/70 shrink-0">
							• {appName}
						</span>
					)}
				</div>
				{template.metadata?.description && (
					<p className="text-xs text-muted-foreground truncate">
						{template.metadata.description}
					</p>
				)}
			</div>
			<ChevronRight className="h-4 w-4 text-muted-foreground/30 group-hover:text-muted-foreground transition-colors" />
		</button>
	);
}

function BrowserPreviewPane({
	template,
	appIcon,
	isApplying,
	onApply,
	onClose,
}: {
	template: TemplateInfo;
	appIcon?: string | null;
	isApplying: boolean;
	onApply: () => void;
	onClose: () => void;
}) {
	const { primaryHue, isDark } = useThemeInfo();
	const backend = useBackend();
	const templateBoard = useInvoke(
		backend.templateState.getTemplate,
		backend.templateState,
		[template.appId, template.templateId],
		true,
	);

	const nodeCount = templateBoard.data
		? Object.keys(templateBoard.data.nodes).length
		: 0;
	const commentCount = templateBoard.data
		? Object.keys(templateBoard.data.comments).length
		: 0;

	return (
		<div className="flex flex-col h-full">
			{/* Header with close button */}
			<div className="flex items-center justify-between p-4 border-b border-border/50 shrink-0">
				<h3 className="font-semibold text-foreground truncate">Preview</h3>
				<button
					type="button"
					onClick={onClose}
					className="p-1.5 rounded-lg text-muted-foreground hover:text-foreground hover:bg-muted/50 transition-colors"
				>
					<X className="h-4 w-4" />
				</button>
			</div>

			{/* Visual preview */}
			<div className="h-48 bg-muted/30 border-b border-border/50 overflow-hidden shrink-0">
				{templateBoard.isLoading ? (
					<div className="w-full h-full flex items-center justify-center">
						<Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
					</div>
				) : templateBoard.data ? (
					<FlowPreview
						nodes={Object.values(templateBoard.data.nodes)}
						comments={templateBoard.data.comments}
						layers={templateBoard.data.layers}
						variables={templateBoard.data.variables}
					/>
				) : (
					<div className="w-full h-full flex items-center justify-center">
						<p className="text-xs text-muted-foreground">
							Preview not available
						</p>
					</div>
				)}
			</div>

			{/* Content - scrollable */}
			<ScrollArea className="flex-1">
				<div className="p-4 space-y-4">
					{/* Title and icon */}
					<div className="flex items-start gap-3">
						{appIcon ? (
							<img
								src={appIcon}
								alt=""
								className="h-12 w-12 rounded-xl object-cover shrink-0"
							/>
						) : (
							(() => {
								const g = hashToGradient(template.appId, primaryHue, isDark);
								return (
									<div
										className="h-12 w-12 rounded-xl flex items-center justify-center shrink-0"
										style={{
											background: `linear-gradient(${g.angle}deg, ${g.from}, ${g.to})`,
										}}
									>
										<LayoutTemplate className="h-6 w-6 text-white/80" />
									</div>
								);
							})()
						)}
						<div className="flex-1 min-w-0">
							<h2 className="text-lg font-semibold text-foreground">
								{template.metadata?.name || template.templateId}
							</h2>
							{template.metadata?.tags && template.metadata.tags.length > 0 && (
								<div className="flex flex-wrap gap-1 mt-1.5">
									{template.metadata.tags.slice(0, 4).map((tag: string) => (
										<Badge key={tag} variant="secondary" className="text-xs">
											{tag}
										</Badge>
									))}
								</div>
							)}
						</div>
					</div>

					{/* Description */}
					{template.metadata?.description && (
						<p className="text-sm text-muted-foreground leading-relaxed">
							{template.metadata.description}
						</p>
					)}

					{/* Stats */}
					<div className="grid grid-cols-2 gap-2">
						<div className="p-3 rounded-lg bg-muted/30 border border-border/50 text-center">
							<div className="text-xl font-semibold text-foreground">
								{templateBoard.isLoading ? "..." : nodeCount}
							</div>
							<div className="text-xs text-muted-foreground">Nodes</div>
						</div>
						<div className="p-3 rounded-lg bg-muted/30 border border-border/50 text-center">
							<div className="text-xl font-semibold text-foreground">
								{templateBoard.isLoading ? "..." : commentCount}
							</div>
							<div className="text-xs text-muted-foreground">Comments</div>
						</div>
					</div>

					{/* Author */}
					{template.metadata?.author && (
						<p className="text-xs text-muted-foreground">
							By {template.metadata.author}
						</p>
					)}
				</div>
			</ScrollArea>

			{/* Footer */}
			<div className="p-4 border-t border-border/50 shrink-0">
				<Button onClick={onApply} disabled={isApplying} className="w-full">
					{isApplying ? (
						<>
							<Loader2 className="h-4 w-4 mr-2 animate-spin" />
							Applying...
						</>
					) : (
						<>
							<Sparkles className="h-4 w-4 mr-2" />
							Use template
						</>
					)}
				</Button>
			</div>
		</div>
	);
}

function BrowserTemplateCard({
	template,
	appIcon,
	appThumbnail,
	appName,
	isSelected,
	onClick,
}: {
	template: TemplateInfo;
	appIcon?: string | null;
	appThumbnail?: string | null;
	appName?: string;
	isSelected?: boolean;
	onClick: () => void;
}) {
	const { primaryHue, isDark } = useThemeInfo();
	return (
		<button
			type="button"
			onClick={onClick}
			className={`text-left rounded-xl border bg-card overflow-hidden hover:border-primary/50 hover:shadow-lg transition-all duration-200 group flex flex-col h-full ${
				isSelected
					? "border-primary ring-2 ring-primary/20"
					: "border-border/50"
			}`}
		>
			{/* Preview placeholder */}
			<div className="aspect-video relative overflow-hidden flex items-center justify-center w-full">
				{appThumbnail ? (
					<img
						src={appThumbnail}
						alt=""
						className="absolute inset-0 w-full h-full object-cover"
					/>
				) : (
					<>
						{(() => {
							const g = hashToGradient(template.appId, primaryHue, isDark);
							return (
								<div
									className="absolute inset-0"
									style={{
										background: `linear-gradient(${g.angle}deg, ${g.from}, ${g.to})`,
									}}
								/>
							);
						})()}
						<div className="relative flex flex-col items-center gap-2">
							{appIcon ? (
								<img
									src={appIcon}
									alt=""
									className="h-12 w-12 rounded-xl object-cover"
								/>
							) : (
								<LayoutTemplate className="h-8 w-8 text-white/80" />
							)}
						</div>
					</>
				)}
				{/* App name badge */}
				{appName && (
					<div className="absolute top-2 left-2">
						<Badge
							variant="secondary"
							className="text-[10px] bg-background/90 backdrop-blur-sm"
						>
							{appName}
						</Badge>
					</div>
				)}
			</div>

			{/* Info */}
			<div className="p-3 flex flex-col flex-1">
				<p className="font-medium text-sm text-foreground truncate group-hover:text-primary transition-colors">
					{template.metadata?.name || template.templateId}
				</p>
				{template.metadata?.description && (
					<p className="text-xs text-muted-foreground line-clamp-2 mt-1">
						{template.metadata.description}
					</p>
				)}
				{template.metadata?.tags && template.metadata.tags.length > 0 && (
					<div className="flex flex-wrap gap-1 mt-auto pt-2">
						{template.metadata.tags.slice(0, 3).map((tag: string) => (
							<Badge
								key={tag}
								variant="outline"
								className="text-[10px] px-1.5 py-0"
							>
								{tag}
							</Badge>
						))}
					</div>
				)}
			</div>
		</button>
	);
}

function BrowserTemplateRow({
	template,
	appIcon,
	appName,
	isSelected,
	onClick,
}: {
	template: TemplateInfo;
	appIcon?: string | null;
	appName?: string;
	isSelected?: boolean;
	onClick: () => void;
}) {
	const { primaryHue, isDark } = useThemeInfo();
	return (
		<button
			type="button"
			onClick={onClick}
			className={`w-full text-left flex items-center gap-4 p-3 rounded-xl border bg-card hover:border-primary/50 hover:bg-accent/30 transition-all group ${
				isSelected
					? "border-primary ring-2 ring-primary/20 bg-accent/20"
					: "border-border/50"
			}`}
		>
			{appIcon ? (
				<img
					src={appIcon}
					alt=""
					className="h-12 w-12 rounded-lg object-cover shrink-0"
				/>
			) : (
				(() => {
					const g = hashToGradient(template.appId, primaryHue, isDark);
					return (
						<div
							className="h-12 w-12 rounded-lg flex items-center justify-center shrink-0"
							style={{
								background: `linear-gradient(${g.angle}deg, ${g.from}, ${g.to})`,
							}}
						>
							<LayoutTemplate className="h-6 w-6 text-white/80" />
						</div>
					);
				})()
			)}
			<div className="flex-1 min-w-0">
				<div className="flex items-center gap-2">
					<p className="font-medium text-sm text-foreground truncate group-hover:text-primary transition-colors">
						{template.metadata?.name || template.templateId}
					</p>
					{appName && (
						<Badge
							variant="outline"
							className="text-[10px] px-1.5 py-0 shrink-0"
						>
							{appName}
						</Badge>
					)}
				</div>
				{template.metadata?.description && (
					<p className="text-xs text-muted-foreground truncate mt-0.5">
						{template.metadata.description}
					</p>
				)}
			</div>
			{template.metadata?.tags && template.metadata.tags.length > 0 && (
				<div className="flex gap-1 shrink-0">
					{template.metadata.tags.slice(0, 2).map((tag: string) => (
						<Badge key={tag} variant="secondary" className="text-xs">
							{tag}
						</Badge>
					))}
				</div>
			)}
			<ChevronRight className="h-5 w-5 text-muted-foreground/30 group-hover:text-muted-foreground transition-colors shrink-0" />
		</button>
	);
}

export default FlowTemplateSelector;
