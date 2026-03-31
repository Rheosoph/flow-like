"use client";

import { createId } from "@paralleldrive/cuid2";
import {
	A2UIRenderer,
	Badge,
	Button,
	Card,
	CardContent,
	CardHeader,
	CardTitle,
	DataProvider,
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
	DialogTrigger,
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
	type IDate,
	Input,
	Label,
	ScrollArea,
	Separator,
	TextEditor,
	Textarea,
	formatRelativeTime,
	nowSystemTime,
	useBackend,
	useInvoke,
	useSetQueryParams,
} from "@tm9657/flow-like-ui";
import {
	ArrowLeft,
	Calendar,
	Edit,
	LayoutGridIcon,
	Loader2,
	MoreVertical,
	Plus,
	Save,
	Search,
	Settings,
	Trash2,
	X,
} from "lucide-react";
import { useSearchParams } from "next/navigation";
import { useCallback, useMemo, useState } from "react";
import { toast } from "sonner";

export default function WidgetsPage() {
	const backend = useBackend();
	const searchParams = useSearchParams();
	const appId = searchParams.get("id") ?? "";
	const widgetId = searchParams.get("widgetId");
	const setQueryParams = useSetQueryParams();
	const [searchTerm, setSearchTerm] = useState("");
	const [isCreateDialogOpen, setIsCreateDialogOpen] = useState(false);
	const [newWidget, setNewWidget] = useState({
		name: "",
		description: "",
	});

	const widgets = useInvoke(
		backend.widgetState.getWidgets,
		backend.widgetState,
		[appId],
		!!appId,
		[appId],
	);

	const filteredWidgets = useMemo(() => {
		if (!widgets.data) return [];
		if (!searchTerm) return widgets.data;
		return widgets.data.filter(
			([, , meta]) =>
				meta?.name?.toLowerCase().includes(searchTerm.toLowerCase()) ||
				meta?.description?.toLowerCase().includes(searchTerm.toLowerCase()),
		);
	}, [widgets.data, searchTerm]);

	const handleCreateWidget = useCallback(async () => {
		if (!appId || !newWidget.name.trim()) {
			toast.error("Please enter a widget name");
			return;
		}

		const widgetId = createId();
		await backend.widgetState.createWidget(
			appId,
			widgetId,
			newWidget.name.trim(),
			newWidget.description.trim() || undefined,
		);
		await backend.widgetState.pushWidgetMeta(appId, widgetId, {
			name: newWidget.name.trim(),
			description: newWidget.description.trim() || "",
			tags: [],
			long_description: "",
			created_at: nowSystemTime(),
			updated_at: nowSystemTime(),
			preview_media: [],
		});
		await widgets.refetch();
		toast.success("Widget created successfully");
		setIsCreateDialogOpen(false);
		setNewWidget({ name: "", description: "" });
		setQueryParams("widgetId", widgetId);
	}, [appId, newWidget, backend.widgetState, widgets, setQueryParams]);

	const handleDeleteWidget = useCallback(
		async (widgetId: string) => {
			if (!appId) return;
			try {
				await backend.widgetState.deleteWidget(appId, widgetId);
				await widgets.refetch();
				toast.success("Widget deleted");
			} catch (error) {
				console.error("Failed to delete widget:", error);
				toast.error("Failed to delete widget");
			}
		},
		[appId, backend.widgetState, widgets],
	);

	if (widgetId) {
		return <WidgetPreview appId={appId} widgetId={widgetId} />;
	}

	return (
		<main className="flex-col flex grow max-h-full p-6 pt-0 space-y-8 overflow-auto md:overflow-visible min-h-0">
			<div className="flex items-center justify-between py-4">
				<div className="space-y-1">
					<h1 className="text-2xl font-bold">Widgets</h1>
					<p className="text-muted-foreground text-sm">
						Build reusable UI blocks that users interact with when using your app
					</p>
				</div>
				<Dialog open={isCreateDialogOpen} onOpenChange={setIsCreateDialogOpen}>
					<DialogTrigger asChild>
						<Button className="shadow-sm">
							<Plus className="w-4 h-4 mr-2" />
							Create Widget
						</Button>
					</DialogTrigger>
					<DialogContent className="sm:max-w-md">
						<DialogHeader className="space-y-3">
							<div className="mx-auto flex h-12 w-12 items-center justify-center rounded-full bg-primary/10">
								<LayoutGridIcon className="h-6 w-6 text-primary" />
							</div>
							<DialogTitle className="text-center text-xl">
								Create New Widget
							</DialogTitle>
							<DialogDescription className="text-center">
								Design a reusable UI component for your application
							</DialogDescription>
						</DialogHeader>

						<div className="space-y-6 py-4">
							<div className="space-y-2">
								<Label htmlFor="widget-name" className="text-sm font-medium">
									Widget Name
								</Label>
								<Input
									id="widget-name"
									placeholder="Enter widget name"
									value={newWidget.name}
									onChange={(e) =>
										setNewWidget({ ...newWidget, name: e.target.value })
									}
								/>
							</div>

							<div className="space-y-2">
								<Label
									htmlFor="widget-description"
									className="text-sm font-medium"
								>
									Description
								</Label>
								<Textarea
									id="widget-description"
									placeholder="Describe what this widget does"
									value={newWidget.description}
									onChange={(e) =>
										setNewWidget({ ...newWidget, description: e.target.value })
									}
									className="min-h-20 resize-none"
								/>
							</div>

							<div className="flex gap-2 pt-4">
								<Button
									onClick={handleCreateWidget}
									disabled={!newWidget.name.trim()}
									className="flex-1"
								>
									Create Widget
								</Button>
								<Button
									variant="outline"
									onClick={() => setIsCreateDialogOpen(false)}
								>
									Cancel
								</Button>
							</div>
						</div>
					</DialogContent>
				</Dialog>
			</div>

			<div className="flex items-center gap-4">
				<div className="relative flex-1 max-w-md">
					<Search className="absolute left-3 top-1/2 transform -translate-y-1/2 text-muted-foreground w-4 h-4" />
					<Input
						placeholder="Search widgets..."
						value={searchTerm}
						onChange={(e) => setSearchTerm(e.target.value)}
						className="pl-10"
					/>
				</div>
			</div>

			{widgets.isLoading ? (
				<div className="flex items-center justify-center py-12">
					<Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
				</div>
			) : filteredWidgets.length > 0 ? (
				<div className="flex-1 overflow-auto md:overflow-visible">
					<div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
						{filteredWidgets.map(([, wId, meta]) => (
							<button
								type="button"
								key={wId}
								onClick={() => setQueryParams("widgetId", wId)}
								className="h-full text-left"
							>
								<Card className="group hover:shadow-xl transition-all duration-300 h-full flex flex-col">
									<CardHeader className="space-y-4">
										<div className="flex items-start justify-between">
											<div className="flex items-center gap-3">
												<div className="p-2 bg-primary/10 group-hover:bg-primary/20 rounded-lg transition-colors">
													<LayoutGridIcon className="w-5 h-5 text-primary" />
												</div>
												<div className="flex-1 min-w-0">
													<CardTitle className="text-lg font-semibold text-foreground group-hover:text-primary transition-colors truncate">
														{meta?.name || wId}
													</CardTitle>
												</div>
											</div>
											<DropdownMenu>
												<DropdownMenuTrigger asChild>
													<Button
														variant="ghost"
														size="sm"
														className="opacity-0 group-hover:opacity-100 transition-opacity"
														onClick={(e) => e.stopPropagation()}
													>
														<MoreVertical className="w-4 h-4" />
													</Button>
												</DropdownMenuTrigger>
												<DropdownMenuContent align="end">
													<DropdownMenuItem
														onClick={(e) => {
															e.stopPropagation();
															setQueryParams("widgetId", wId);
														}}
													>
														<Edit className="w-4 h-4 mr-2" />
														Edit
													</DropdownMenuItem>
													<DropdownMenuSeparator />
													<DropdownMenuItem
														className="text-destructive focus:text-destructive"
														onClick={(e) => {
															e.stopPropagation();
															handleDeleteWidget(wId);
														}}
													>
														<Trash2 className="w-4 h-4 mr-2" />
														Delete
													</DropdownMenuItem>
												</DropdownMenuContent>
											</DropdownMenu>
										</div>
									</CardHeader>
									<CardContent className="space-y-4 flex-1 flex flex-col">
										<p className="text-muted-foreground text-sm leading-relaxed line-clamp-2 flex-1">
											{meta?.description || "No description"}
										</p>

										{meta?.tags && meta.tags.length > 0 && (
											<div className="flex flex-wrap gap-1">
												{meta.tags.slice(0, 3).map((tag) => (
													<Badge
														key={tag}
														variant="outline"
														className="text-xs"
													>
														{tag}
													</Badge>
												))}
												{meta.tags.length > 3 && (
													<Badge variant="outline" className="text-xs">
														+{meta.tags.length - 3}
													</Badge>
												)}
											</div>
										)}

										<div className="pt-4 border-t mt-auto">
											<div className="flex items-center justify-between text-xs text-muted-foreground">
												<div className="flex items-center gap-1">
													<Calendar className="w-3 h-3" />
													{meta?.created_at && (
														<span>
															{formatRelativeTime(meta.created_at as IDate)}
														</span>
													)}
												</div>
											</div>
										</div>
									</CardContent>
								</Card>
							</button>
						))}
					</div>
				</div>
			) : (
				<div className="flex flex-col items-center text-center py-12 max-w-md mx-auto space-y-4">
					<div className="w-16 h-16 rounded-full bg-primary/10 flex items-center justify-center">
						<LayoutGridIcon className="w-8 h-8 text-primary" />
					</div>
					<h3 className="text-lg font-medium">
						{searchTerm ? "No widgets found" : "No widgets yet"}
					</h3>
					<p className="text-sm text-muted-foreground">
						{searchTerm
							? "Try adjusting your search terms."
							: "Widgets are the visual building blocks of your app — forms, dashboards, chat interfaces, and more. Create one to start designing."}
					</p>
					{!searchTerm && (
						<Button onClick={() => setIsCreateDialogOpen(true)} className="mt-2">
							<Plus className="w-4 h-4 mr-2" />
							Create Your First Widget
						</Button>
					)}
				</div>
			)}
		</main>
	);
}

function WidgetPreview({
	appId,
	widgetId,
}: Readonly<{ appId: string; widgetId: string }>) {
	const backend = useBackend();
	const setQueryParams = useSetQueryParams();
	const [isEditing, setIsEditing] = useState(false);
	const [editState, setEditState] = useState<{
		name: string;
		description: string;
		long_description: string;
		tags: string[];
	} | null>(null);
	const [newTag, setNewTag] = useState("");

	const widget = useInvoke(
		backend.widgetState.getWidget,
		backend.widgetState,
		[appId, widgetId],
		!!appId && !!widgetId,
	);

	const metadata = useInvoke(
		backend.widgetState.getWidgetMeta,
		backend.widgetState,
		[appId, widgetId],
		!!appId && !!widgetId,
	);

	const currentData = useMemo(
		() =>
			editState || {
				name: metadata.data?.name || widget.data?.name || "",
				description:
					metadata.data?.description || widget.data?.description || "",
				long_description: metadata.data?.long_description || "",
				tags: metadata.data?.tags || widget.data?.tags || [],
			},
		[editState, metadata.data, widget.data],
	);

	const handleEdit = useCallback(() => {
		if (!isEditing) {
			setEditState({
				name: metadata.data?.name || widget.data?.name || "",
				description:
					metadata.data?.description || widget.data?.description || "",
				long_description: metadata.data?.long_description || "",
				tags: metadata.data?.tags || widget.data?.tags || [],
			});
		}
		setIsEditing(!isEditing);
	}, [isEditing, metadata.data, widget.data]);

	const handleSave = useCallback(async () => {
		if (!editState) return;

		await backend.widgetState.pushWidgetMeta(appId, widgetId, {
			...(metadata.data || {
				created_at: nowSystemTime(),
				preview_media: [],
			}),
			name: editState.name,
			description: editState.description,
			long_description: editState.long_description,
			tags: editState.tags,
			updated_at: nowSystemTime(),
		});

		await metadata.refetch();
		setIsEditing(false);
		setEditState(null);
		toast.success("Widget metadata saved");
	}, [editState, metadata, backend.widgetState, appId, widgetId]);

	const handleAddTag = useCallback(() => {
		if (!newTag.trim() || !editState) return;
		if (!editState.tags.includes(newTag.trim())) {
			setEditState({ ...editState, tags: [...editState.tags, newTag.trim()] });
		}
		setNewTag("");
	}, [newTag, editState]);

	const handleRemoveTag = useCallback(
		(tag: string) => {
			if (!editState) return;
			setEditState({
				...editState,
				tags: editState.tags.filter((t) => t !== tag),
			});
		},
		[editState],
	);

	const handleOpenBuilder = useCallback(() => {
		window.location.href = `/widget?id=${widgetId}&app=${appId}`;
	}, [widgetId, appId]);

	const previewSurface = useMemo(() => {
		if (!widget.data?.components || widget.data.components.length === 0) {
			return null;
		}
		// Convert components array to Record<string, SurfaceComponent>
		const componentsRecord = widget.data.components.reduce(
			(acc, comp) => {
				acc[comp.id] = comp;
				return acc;
			},
			{} as Record<string, (typeof widget.data.components)[number]>,
		);

		// Determine the root component ID - prefer 'root' (WidgetBuilder's ROOT_ID), then stored value, then first component
		const storedRootId = widget.data.rootComponentId;
		const rootComponentId = componentsRecord["root"]
			? "root"
			: storedRootId && componentsRecord[storedRootId]
				? storedRootId
				: widget.data.components[0]?.id || "";

		return {
			id: widgetId,
			rootComponentId,
			components: componentsRecord,
		};
	}, [widget.data, widgetId]);

	if (widget.isLoading || metadata.isLoading) {
		return (
			<div className="flex items-center justify-center h-full">
				<Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
			</div>
		);
	}

	return (
		<main className="flex flex-col h-full max-h-full overflow-hidden p-6 pt-0">
			<div className="flex items-center justify-between py-4 border-b mb-6">
				<div className="flex items-center gap-4">
					<Button
						variant="ghost"
						size="sm"
						onClick={() => setQueryParams("widgetId", undefined)}
					>
						<ArrowLeft className="w-4 h-4 mr-2" />
						Back
					</Button>
					<Separator orientation="vertical" className="h-6" />
					<div>
						<h1 className="text-xl font-semibold">{currentData.name}</h1>
						<p className="text-sm text-muted-foreground">Widget Details</p>
					</div>
				</div>
				<div className="flex items-center gap-2">
					{isEditing ? (
						<>
							<Button variant="outline" size="sm" onClick={handleEdit}>
								<X className="w-4 h-4 mr-2" />
								Cancel
							</Button>
							<Button size="sm" onClick={handleSave}>
								<Save className="w-4 h-4 mr-2" />
								Save
							</Button>
						</>
					) : (
						<>
							<Button variant="outline" size="sm" onClick={handleEdit}>
								<Edit className="w-4 h-4 mr-2" />
								Edit Metadata
							</Button>
							<Button size="sm" onClick={handleOpenBuilder}>
								<Settings className="w-4 h-4 mr-2" />
								Open Builder
							</Button>
						</>
					)}
				</div>
			</div>

			<div className="flex-1 grid grid-cols-1 lg:grid-cols-2 gap-6 min-h-0 overflow-hidden">
				<Card className="flex flex-col overflow-hidden">
					<CardHeader className="shrink-0">
						<CardTitle className="text-base">Widget Preview</CardTitle>
					</CardHeader>
					<CardContent className="flex-1 overflow-auto p-4 bg-muted/30 rounded-b-lg">
						{previewSurface ? (
							<DataProvider initialData={widget.data?.dataModel || []}>
								<A2UIRenderer surface={previewSurface} isPreviewMode={true} />
							</DataProvider>
						) : (
							<div className="flex flex-col items-center justify-center h-full text-muted-foreground">
								<LayoutGridIcon className="w-12 h-12 mb-4 opacity-50" />
								<p className="text-sm">No components in this widget</p>
								<Button
									variant="link"
									className="mt-2"
									onClick={handleOpenBuilder}
								>
									Open Builder to add components
								</Button>
							</div>
						)}
					</CardContent>
				</Card>

				<div className="flex flex-col gap-6 overflow-auto md:overflow-visible">
					<Card>
						<CardHeader>
							<CardTitle className="text-base">Metadata</CardTitle>
						</CardHeader>
						<CardContent className="space-y-4">
							<div className="space-y-2">
								<Label className="text-sm font-medium">Name</Label>
								{isEditing ? (
									<Input
										value={editState?.name || ""}
										onChange={(e) =>
											setEditState({ ...editState!, name: e.target.value })
										}
									/>
								) : (
									<p className="text-sm text-muted-foreground">
										{currentData.name || "Untitled"}
									</p>
								)}
							</div>

							<div className="space-y-2">
								<Label className="text-sm font-medium">Description</Label>
								{isEditing ? (
									<Textarea
										value={editState?.description || ""}
										onChange={(e) =>
											setEditState({
												...editState!,
												description: e.target.value,
											})
										}
										className="min-h-20 resize-none"
									/>
								) : (
									<p className="text-sm text-muted-foreground">
										{currentData.description || "No description"}
									</p>
								)}
							</div>

							<div className="space-y-2">
								<Label className="text-sm font-medium">Tags</Label>
								{isEditing ? (
									<div className="space-y-2">
										<div className="flex gap-2">
											<Input
												value={newTag}
												onChange={(e) => setNewTag(e.target.value)}
												placeholder="Add a tag..."
												onKeyDown={(e) => {
													if (e.key === "Enter") {
														e.preventDefault();
														handleAddTag();
													}
												}}
											/>
											<Button
												variant="outline"
												size="sm"
												onClick={handleAddTag}
											>
												Add
											</Button>
										</div>
										<div className="flex flex-wrap gap-1">
											{editState?.tags.map((tag) => (
												<Badge
													key={tag}
													variant="secondary"
													className="cursor-pointer"
													onClick={() => handleRemoveTag(tag)}
												>
													{tag}
													<X className="w-3 h-3 ml-1" />
												</Badge>
											))}
										</div>
									</div>
								) : (
									<div className="flex flex-wrap gap-1">
										{currentData.tags.length > 0 ? (
											currentData.tags.map((tag) => (
												<Badge key={tag} variant="outline">
													{tag}
												</Badge>
											))
										) : (
											<p className="text-sm text-muted-foreground">No tags</p>
										)}
									</div>
								)}
							</div>
						</CardContent>
					</Card>

					<Card>
						<CardHeader>
							<CardTitle className="text-base">Detailed Description</CardTitle>
						</CardHeader>
						<CardContent>
							{isEditing ? (
								<div className="min-h-[200px] border rounded-md">
									<TextEditor
										editable
										isMarkdown
										initialContent={
											editState?.long_description ||
											"*Add a detailed description...*"
										}
										onChange={(content) =>
											setEditState({ ...editState!, long_description: content })
										}
									/>
								</div>
							) : (
								<div className="min-h-[100px]">
									<TextEditor
										editable={false}
										isMarkdown
										initialContent={
											currentData.long_description ||
											"*No detailed description available.*"
										}
									/>
								</div>
							)}
						</CardContent>
					</Card>

					<Card>
						<CardHeader>
							<CardTitle className="text-base">
								Configurable Properties
							</CardTitle>
						</CardHeader>
						<CardContent>
							{widget.data?.exposedProps &&
							widget.data.exposedProps.length > 0 ? (
								<ScrollArea className="max-h-[300px]">
									<div className="space-y-3">
										{widget.data.exposedProps.map((prop) => (
											<div
												key={prop.id}
												className="flex items-start justify-between p-3 rounded-lg bg-muted/50"
											>
												<div className="flex-1">
													<p className="text-sm font-medium">{prop.label}</p>
													{prop.description && (
														<p className="text-xs text-muted-foreground mt-1">
															{prop.description}
														</p>
													)}
													<div className="flex items-center gap-2 mt-2">
														<Badge variant="outline" className="text-xs">
															{typeof prop.propType === "string"
																? prop.propType
																: "Enum"}
														</Badge>
														{prop.group && (
															<Badge variant="secondary" className="text-xs">
																{prop.group}
															</Badge>
														)}
													</div>
												</div>
											</div>
										))}
									</div>
								</ScrollArea>
							) : (
								<div className="text-center py-6 text-muted-foreground">
									<Settings className="w-8 h-8 mx-auto mb-2 opacity-50" />
									<p className="text-sm">No exposed properties</p>
									<p className="text-xs mt-1">
										Add props in the widget builder to make them configurable
									</p>
								</div>
							)}
						</CardContent>
					</Card>

					<Card>
						<CardHeader>
							<CardTitle className="text-base">Widget Info</CardTitle>
						</CardHeader>
						<CardContent className="space-y-3">
							<div className="flex justify-between text-sm">
								<span className="text-muted-foreground">Components</span>
								<span className="font-medium">
									{widget.data?.components?.length || 0}
								</span>
							</div>
							<div className="flex justify-between text-sm">
								<span className="text-muted-foreground">Data Entries</span>
								<span className="font-medium">
									{widget.data?.dataModel?.length || 0}
								</span>
							</div>
							<div className="flex justify-between text-sm">
								<span className="text-muted-foreground">Version</span>
								<span className="font-medium">
									{widget.data?.version
										? `v${widget.data.version.join(".")}`
										: "Draft"}
								</span>
							</div>
							{metadata.data?.created_at && (
								<div className="flex justify-between text-sm">
									<span className="text-muted-foreground">Created</span>
									<span className="font-medium">
										{formatRelativeTime(metadata.data.created_at as IDate)}
									</span>
								</div>
							)}
							{metadata.data?.updated_at && (
								<div className="flex justify-between text-sm">
									<span className="text-muted-foreground">Updated</span>
									<span className="font-medium">
										{formatRelativeTime(metadata.data.updated_at as IDate)}
									</span>
								</div>
							)}
						</CardContent>
					</Card>
				</div>
			</div>
		</main>
	);
}
