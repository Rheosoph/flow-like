"use client";

import {
	Badge,
	Button,
	Input,
	Label,
	ScrollArea,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Separator,
	Tabs,
	TabsContent,
	TabsList,
	TabsTrigger,
	Textarea,
	WidgetBuilder,
	useBackend,
	useInvoke,
} from "@tm9657/flow-like-ui";
import type { IWidget, Version, VersionType } from "@tm9657/flow-like-ui";
import type {
	SurfaceComponent,
	WidgetAction,
} from "@tm9657/flow-like-ui/components/a2ui/types";
import {
	ArrowLeft,
	Check,
	GitBranchIcon,
	Loader2,
	Plus,
	Save,
	Settings,
	TagIcon,
	Trash2,
	X,
} from "lucide-react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";

export default function WidgetEditorPage() {
	const searchParams = useSearchParams();
	const router = useRouter();
	const backend = useBackend();

	const { widgetId, appId, version } = useMemo(() => {
		const widgetId = searchParams.get("id") ?? "";
		const appId = searchParams.get("app") ?? "";
		let version: Version | undefined;
		const versionStr = searchParams.get("version");
		if (versionStr) {
			const parts = versionStr.split("_").map(Number);
			if (parts.length === 3) {
				version = parts as Version;
			}
		}
		return { widgetId, appId, version };
	}, [searchParams]);

	const [widget, setWidget] = useState<IWidget | null>(null);
	const [isLoading, setIsLoading] = useState(true);
	const [isSaving, setIsSaving] = useState(false);
	const [hasUnsavedChanges, setHasUnsavedChanges] = useState(false);
	const [lastSavedAt, setLastSavedAt] = useState<Date | null>(null);
	const [showSettings, setShowSettings] = useState(false);
	const [isCreatingVersion, setIsCreatingVersion] = useState(false);

	// Auto-save debounce ref
	const saveTimeoutRef = useRef<NodeJS.Timeout | null>(null);
	const pendingComponentsRef = useRef<SurfaceComponent[] | null>(null);

	const versions = useInvoke(
		backend.widgetState.getWidgetVersions,
		backend.widgetState,
		[appId, widgetId],
		!!appId && !!widgetId,
		[appId, widgetId],
	);

	useEffect(() => {
		const loadWidget = async () => {
			if (!widgetId || !appId) {
				setIsLoading(false);
				return;
			}

			try {
				const loadedWidget = await backend.widgetState.getWidget(
					appId,
					widgetId,
					version,
				);
				setWidget(loadedWidget);
			} catch {
				const newWidget: IWidget = {
					id: widgetId,
					name: "New Widget",
					rootComponentId: "root",
					components: [],
					dataModel: [],
					customizationOptions: [],
					tags: [],
					createdAt: new Date().toISOString(),
					updatedAt: new Date().toISOString(),
				};
				setWidget(newWidget);
			} finally {
				setIsLoading(false);
			}
		};

		loadWidget();
	}, [widgetId, appId, version, backend.widgetState]);

	// Cleanup on unmount
	useEffect(() => {
		return () => {
			if (saveTimeoutRef.current) {
				clearTimeout(saveTimeoutRef.current);
			}
		};
	}, []);

	const performSave = useCallback(
		async (components: SurfaceComponent[]) => {
			if (!widget || !appId) return;

			setIsSaving(true);
			try {
				await backend.widgetState.updateWidget(appId, {
					...widget,
					components,
					updatedAt: new Date().toISOString(),
				});
				setWidget((prev) => (prev ? { ...prev, components } : prev));
				setHasUnsavedChanges(false);
				setLastSavedAt(new Date());
			} catch (error) {
				console.error("Failed to save widget:", error);
				toast.error("Failed to save widget");
			} finally {
				setIsSaving(false);
			}
		},
		[widget, appId, backend.widgetState],
	);

	// Manual save handler
	const handleSave = useCallback(
		async (components: SurfaceComponent[]) => {
			// Clear any pending auto-save
			if (saveTimeoutRef.current) {
				clearTimeout(saveTimeoutRef.current);
				saveTimeoutRef.current = null;
			}
			pendingComponentsRef.current = null;
			await performSave(components);
		},
		[performSave],
	);

	// Auto-save on change with debouncing
	const handleChange = useCallback(
		(components: SurfaceComponent[]) => {
			if (!widget || !appId) return;

			setHasUnsavedChanges(true);
			pendingComponentsRef.current = components;

			// Update local state immediately
			setWidget((prev) => (prev ? { ...prev, components } : prev));

			// Clear existing timeout
			if (saveTimeoutRef.current) {
				clearTimeout(saveTimeoutRef.current);
			}

			// Set new debounced save (1.5 seconds after last change)
			saveTimeoutRef.current = setTimeout(() => {
				if (pendingComponentsRef.current) {
					performSave(pendingComponentsRef.current);
					pendingComponentsRef.current = null;
				}
			}, 1500);
		},
		[widget, appId, performSave],
	);

	const updateWidgetProperty = useCallback(
		<K extends keyof IWidget>(key: K, value: IWidget[K]) => {
			setWidget((prev) => {
				if (!prev) return prev;
				return { ...prev, [key]: value };
			});
		},
		[],
	);

	const handleSaveMetadata = useCallback(async () => {
		if (!widget || !appId) return;

		setIsSaving(true);
		try {
			await backend.widgetState.updateWidget(appId, {
				...widget,
				updatedAt: new Date().toISOString(),
			});
		} catch (error) {
			console.error("Failed to save widget metadata:", error);
		} finally {
			setIsSaving(false);
		}
	}, [widget, appId, backend.widgetState]);

	const handleCreateVersion = useCallback(
		async (versionType: VersionType) => {
			if (!widget || !appId) return;

			setIsCreatingVersion(true);
			try {
				// First save any pending changes
				await backend.widgetState.updateWidget(appId, {
					...widget,
					updatedAt: new Date().toISOString(),
				});

				// Create the new version
				const newVersion = await backend.widgetState.createWidgetVersion(
					appId,
					widgetId,
					versionType,
				);

				// Reload the widget with the new version
				const updatedWidget = await backend.widgetState.getWidget(
					appId,
					widgetId,
				);
				setWidget(updatedWidget);

				// Refresh versions list
				versions.refetch();
			} catch (error) {
				console.error("Failed to create version:", error);
			} finally {
				setIsCreatingVersion(false);
			}
		},
		[widget, appId, widgetId, backend.widgetState, versions],
	);

	const handleSwitchVersion = useCallback(
		(versionStr: string) => {
			const newUrl = `/widget?id=${widgetId}&app=${appId}&version=${versionStr}`;
			router.push(newUrl);
		},
		[widgetId, appId, router],
	);

	if (!widgetId) {
		return (
			<div className="flex items-center justify-center h-full">
				<p className="text-muted-foreground">Widget not found</p>
			</div>
		);
	}

	if (isLoading) {
		return (
			<div className="flex items-center justify-center h-full gap-2">
				<Loader2 className="h-5 w-5 animate-spin" />
				<p className="text-muted-foreground">Loading widget...</p>
			</div>
		);
	}

	if (!widget) {
		return (
			<div className="flex items-center justify-center h-full">
				<p className="text-muted-foreground">Widget not found</p>
			</div>
		);
	}

	return (
		<div className="flex flex-col h-full">
			{/* Header */}
			<div className="flex items-center justify-between px-4 py-3 border-b bg-background/95 backdrop-blur supports-backdrop-filter:bg-background/60">
				<div className="flex items-center gap-4">
					<Link href={`/library/config/widgets?id=${appId}`}>
						<Button variant="ghost" size="icon">
							<ArrowLeft className="h-4 w-4" />
						</Button>
					</Link>
					<div>
						<h1 className="text-lg font-semibold">{widget.name}</h1>
						<p className="text-sm text-muted-foreground">
							{widget.description || "Visual Widget Builder"}
						</p>
					</div>
					{widget.version && (
						<Badge variant="secondary">
							v{widget.version[0]}.{widget.version[1]}.{widget.version[2]}
						</Badge>
					)}
				</div>
				<div className="flex items-center gap-3">
					{/* Auto-save status indicator */}
					<div className="flex items-center gap-2 text-sm text-muted-foreground">
						{isSaving ? (
							<>
								<Loader2 className="h-3 w-3 animate-spin" />
								<span>Saving...</span>
							</>
						) : hasUnsavedChanges ? (
							<span className="text-yellow-500">Unsaved changes</span>
						) : lastSavedAt ? (
							<>
								<Check className="h-3 w-3 text-green-500" />
								<span>Saved</span>
							</>
						) : null}
					</div>
					<Button
						variant="outline"
						size="sm"
						onClick={() => setShowSettings(!showSettings)}
					>
						<Settings className="h-4 w-4 mr-2" />
						Settings
					</Button>
					<Button
						onClick={() => handleSave(widget.components)}
						disabled={isSaving || !hasUnsavedChanges}
						size="sm"
						variant={hasUnsavedChanges ? "default" : "outline"}
					>
						{isSaving ? (
							<Loader2 className="h-4 w-4 mr-2 animate-spin" />
						) : (
							<Save className="h-4 w-4 mr-2" />
						)}
						Save Now
					</Button>
				</div>
			</div>

			{/* Main Content */}
			<div className="flex-1 min-h-0 flex">
				{/* Widget Builder */}
				<div className={`flex-1 min-h-0 ${showSettings ? "mr-80" : ""}`}>
					<WidgetBuilder
						initialComponents={widget.components}
						widgetId={widget.id}
						surfaceId={`widget-${widget.id}`}
						onSave={handleSave}
						onChange={handleChange}
						className="h-full"
						actionContext={{
							widgetActions: (widget.actions ?? []).map((a) => ({
								id: a.id,
								label: a.label,
								description: a.description,
							})),
						}}
					/>
				</div>

				{/* Settings Panel */}
				{showSettings && (
					<div className="w-80 border-l bg-background flex flex-col absolute right-0 top-[57px] bottom-0 z-10">
						<div className="p-4 border-b flex items-center justify-between">
							<h2 className="font-semibold">Widget Settings</h2>
							<Button
								variant="ghost"
								size="icon"
								onClick={() => setShowSettings(false)}
							>
								<X className="h-4 w-4" />
							</Button>
						</div>
						<ScrollArea className="flex-1">
							<WidgetSettingsPanel
								widget={widget}
								onUpdateWidget={updateWidgetProperty}
								onSave={handleSaveMetadata}
								isSaving={isSaving}
								versions={versions.data ?? []}
								currentVersion={version}
								onCreateVersion={handleCreateVersion}
								onSwitchVersion={handleSwitchVersion}
								isCreatingVersion={isCreatingVersion}
							/>
						</ScrollArea>
					</div>
				)}
			</div>
		</div>
	);
}

function WidgetSettingsPanel({
	widget,
	onUpdateWidget,
	onSave,
	isSaving,
	versions,
	currentVersion,
	onCreateVersion,
	onSwitchVersion,
	isCreatingVersion,
}: Readonly<{
	widget: IWidget;
	onUpdateWidget: <K extends keyof IWidget>(key: K, value: IWidget[K]) => void;
	onSave: () => void;
	isSaving: boolean;
	versions: Version[];
	currentVersion?: Version;
	onCreateVersion: (type: VersionType) => void;
	onSwitchVersion: (version: string) => void;
	isCreatingVersion: boolean;
}>) {
	const [newTag, setNewTag] = useState("");

	const handleAddTag = () => {
		if (newTag.trim() && !widget.tags.includes(newTag.trim())) {
			onUpdateWidget("tags", [...widget.tags, newTag.trim()]);
			setNewTag("");
		}
	};

	const handleRemoveTag = (tag: string) => {
		onUpdateWidget(
			"tags",
			widget.tags.filter((t) => t !== tag),
		);
	};

	const formatVersion = (v: Version) => `${v[0]}.${v[1]}.${v[2]}`;
	const versionKey = (v: Version) => `${v[0]}_${v[1]}_${v[2]}`;

	return (
		<Tabs defaultValue="general" className="w-full">
			<TabsList className="w-full justify-start px-4 pt-2">
				<TabsTrigger value="general">General</TabsTrigger>
				<TabsTrigger value="events">Events</TabsTrigger>
				<TabsTrigger value="versions">Versions</TabsTrigger>
				<TabsTrigger value="advanced">Advanced</TabsTrigger>
			</TabsList>
			<TabsContent value="general" className="p-4 space-y-4">
				<div className="space-y-2">
					<Label htmlFor="name">Name</Label>
					<Input
						id="name"
						value={widget.name}
						onChange={(e) => onUpdateWidget("name", e.target.value)}
					/>
				</div>
				<div className="space-y-2">
					<Label htmlFor="description">Description</Label>
					<Textarea
						id="description"
						value={widget.description || ""}
						onChange={(e) =>
							onUpdateWidget("description", e.target.value || undefined)
						}
						className="min-h-20"
						placeholder="Describe what this widget does..."
					/>
				</div>
				<Separator />
				<div className="space-y-2">
					<Label>Tags</Label>
					<div className="flex flex-wrap gap-1 mb-2">
						{widget.tags.map((tag) => (
							<Badge
								key={tag}
								variant="secondary"
								className="cursor-pointer hover:bg-destructive hover:text-destructive-foreground"
								onClick={() => handleRemoveTag(tag)}
							>
								{tag} ×
							</Badge>
						))}
						{widget.tags.length === 0 && (
							<span className="text-sm text-muted-foreground">No tags</span>
						)}
					</div>
					<div className="flex gap-2">
						<Input
							placeholder="Add a tag..."
							value={newTag}
							onChange={(e) => setNewTag(e.target.value)}
							onKeyDown={(e) => e.key === "Enter" && handleAddTag()}
						/>
						<Button variant="outline" size="sm" onClick={handleAddTag}>
							Add
						</Button>
					</div>
				</div>
				<Separator />
				<Button onClick={onSave} disabled={isSaving} className="w-full">
					{isSaving ? (
						<Loader2 className="h-4 w-4 mr-2 animate-spin" />
					) : (
						<Save className="h-4 w-4 mr-2" />
					)}
					Save Metadata
				</Button>
			</TabsContent>
			<TabsContent value="versions" className="p-4 space-y-4">
				<div className="space-y-2">
					<Label>Current Version</Label>
					{versions.length > 0 ? (
						<Select
							value={
								currentVersion
									? versionKey(currentVersion)
									: widget.version
										? versionKey(widget.version)
										: "latest"
							}
							onValueChange={onSwitchVersion}
						>
							<SelectTrigger>
								<SelectValue placeholder="Select version" />
							</SelectTrigger>
							<SelectContent>
								{versions.map((v) => (
									<SelectItem key={versionKey(v)} value={versionKey(v)}>
										v{formatVersion(v)}
									</SelectItem>
								))}
							</SelectContent>
						</Select>
					) : (
						<p className="text-sm text-muted-foreground">
							No versions created yet
						</p>
					)}
				</div>
				<Separator />
				<div className="space-y-2">
					<Label>Create New Version</Label>
					<p className="text-xs text-muted-foreground mb-2">
						Creating a version will save current changes and create a snapshot
					</p>
					<div className="flex gap-2">
						<Button
							variant="outline"
							size="sm"
							onClick={() => onCreateVersion("Patch")}
							disabled={isCreatingVersion}
							className="flex-1"
						>
							<TagIcon className="h-3 w-3 mr-1" />
							Patch
						</Button>
						<Button
							variant="outline"
							size="sm"
							onClick={() => onCreateVersion("Minor")}
							disabled={isCreatingVersion}
							className="flex-1"
						>
							<TagIcon className="h-3 w-3 mr-1" />
							Minor
						</Button>
						<Button
							variant="outline"
							size="sm"
							onClick={() => onCreateVersion("Major")}
							disabled={isCreatingVersion}
							className="flex-1"
						>
							<TagIcon className="h-3 w-3 mr-1" />
							Major
						</Button>
					</div>
					{isCreatingVersion && (
						<div className="flex items-center gap-2 text-sm text-muted-foreground">
							<Loader2 className="h-4 w-4 animate-spin" />
							Creating version...
						</div>
					)}
				</div>
				<Separator />
				<div className="space-y-2">
					<Label>Version History</Label>
					{versions.length > 0 ? (
						<div className="space-y-1 max-h-40 overflow-y-auto">
							{versions.map((v) => (
								<div
									key={versionKey(v)}
									className="flex items-center justify-between p-2 rounded-md bg-muted/50 text-sm"
								>
									<div className="flex items-center gap-2">
										<GitBranchIcon className="h-3 w-3" />v{formatVersion(v)}
									</div>
									<Button
										variant="ghost"
										size="sm"
										className="h-6 px-2 text-xs"
										onClick={() => onSwitchVersion(versionKey(v))}
									>
										Load
									</Button>
								</div>
							))}
						</div>
					) : (
						<p className="text-sm text-muted-foreground">
							No versions in history
						</p>
					)}
				</div>
			</TabsContent>
			<TabsContent value="events" className="p-4 space-y-4">
				<WidgetEventsEditor
					actions={widget.actions ?? []}
					onChange={(actions) => onUpdateWidget("actions", actions)}
				/>
				<Separator />
				<Button onClick={onSave} disabled={isSaving} className="w-full">
					{isSaving ? (
						<Loader2 className="h-4 w-4 mr-2 animate-spin" />
					) : (
						<Save className="h-4 w-4 mr-2" />
					)}
					Save Events
				</Button>
			</TabsContent>
			<TabsContent value="advanced" className="p-4 space-y-4">
				<div className="space-y-2">
					<Label>Widget ID</Label>
					<Input value={widget.id} disabled />
				</div>
				<div className="space-y-2">
					<Label>Root Component ID</Label>
					<Input value={widget.rootComponentId} disabled />
				</div>
				<div className="space-y-2">
					<Label>Version</Label>
					<Input
						value={
							widget.version
								? `${widget.version[0]}.${widget.version[1]}.${widget.version[2]}`
								: "Not versioned"
						}
						disabled
					/>
				</div>
				<div className="space-y-2">
					<Label>Created</Label>
					<Input value={new Date(widget.createdAt).toLocaleString()} disabled />
				</div>
				<div className="space-y-2">
					<Label>Last Updated</Label>
					<Input value={new Date(widget.updatedAt).toLocaleString()} disabled />
				</div>
				<Separator />
				<div className="space-y-2">
					<Label>Components</Label>
					<p className="text-sm text-muted-foreground">
						{widget.components.length} component
						{widget.components.length !== 1 ? "s" : ""}
					</p>
				</div>
				<div className="space-y-2">
					<Label>Data Model Entries</Label>
					<p className="text-sm text-muted-foreground">
						{widget.dataModel.length} entr
						{widget.dataModel.length !== 1 ? "ies" : "y"}
					</p>
				</div>
			</TabsContent>
		</Tabs>
	);
}

function WidgetEventsEditor({
	actions,
	onChange,
}: Readonly<{
	actions: WidgetAction[];
	onChange: (actions: WidgetAction[]) => void;
}>) {
	const addAction = () => {
		const id = `action_${Date.now()}`;
		onChange([...actions, { id, label: "New Event", contextSchema: [] }]);
	};

	const updateAction = (index: number, updates: Partial<WidgetAction>) => {
		const updated = actions.map((a, i) =>
			i === index ? { ...a, ...updates } : a,
		);
		onChange(updated);
	};

	const removeAction = (index: number) => {
		onChange(actions.filter((_, i) => i !== index));
	};

	return (
		<div className="space-y-3">
			<div className="flex items-center justify-between">
				<Label>Widget Events</Label>
				<Button variant="outline" size="sm" onClick={addAction}>
					<Plus className="h-3 w-3 mr-1" />
					Add Event
				</Button>
			</div>
			<p className="text-xs text-muted-foreground">
				Define named events that this widget can trigger (e.g. on button press).
				These can be bound to workflows when the widget is instantiated.
			</p>
			{actions.length === 0 && (
				<p className="text-sm text-muted-foreground text-center py-4">
					No events defined
				</p>
			)}
			{actions.map((action, index) => (
				<div key={action.id} className="border rounded-md p-3 space-y-2">
					<div className="flex items-start justify-between gap-2">
						<div className="flex-1 space-y-2">
							<Input
								placeholder="Event label (e.g. On Button Press)"
								value={action.label}
								onChange={(e) => updateAction(index, { label: e.target.value })}
								className="h-8 text-sm"
							/>
							<Input
								placeholder="Description (optional)"
								value={action.description ?? ""}
								onChange={(e) =>
									updateAction(index, {
										description: e.target.value || undefined,
									})
								}
								className="h-8 text-sm"
							/>
						</div>
						<Button
							variant="ghost"
							size="icon"
							className="h-8 w-8 shrink-0 text-destructive hover:text-destructive"
							onClick={() => removeAction(index)}
						>
							<Trash2 className="h-4 w-4" />
						</Button>
					</div>
					<div className="text-xs text-muted-foreground font-mono">
						ID: {action.id}
					</div>
				</div>
			))}
		</div>
	);
}
