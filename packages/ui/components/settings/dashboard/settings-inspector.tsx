"use client";

import {
	BombIcon,
	DollarSignIcon,
	ImageIcon,
	PencilIcon,
	RotateCcwIcon,
	SaveIcon,
	ShieldIcon,
	SparklesIcon,
	StarIcon,
	type TagIcon,
	XIcon,
} from "lucide-react";
import type { ReactNode } from "react";
import { useCallback, useMemo, useState } from "react";
import { toast } from "sonner";
import { useDeveloperMode } from "../../../hooks/use-developer-mode";
import type { IApp, IMetadata } from "../../../lib";
import { IAppCategory, IAppStatus, type IAppType } from "../../../lib";
import { formatAppCategory } from "../../../lib/app-category";
import {
	APP_TYPE_META,
	APP_TYPE_ORDER,
	appTypeMeta,
} from "../../../lib/app-type";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import {
	Dialog,
	DialogContent,
	DialogHeader,
	DialogTitle,
} from "../../ui/dialog";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";
import { Sheet, SheetContent, SheetHeader, SheetTitle } from "../../ui/sheet";
import { TextEditor } from "../../ui/text-editor";
import { Textarea } from "../../ui/textarea";
import { VerificationDialog } from "../../verification-dialog";
import type { ProjectDraft } from "./use-project-draft";
import type { InspectorPanel } from "./use-project-signals";

export interface InspectorSlots {
	/** Visibility switcher, forking toggle and fork action. */
	access?: ReactNode;
	/** EU AI Act wizard and publication review. */
	compliance?: ReactNode;
	/** Reviews, shown under the listing panel once the app is listed. */
	reviews?: ReactNode;
	/** Host-specific extras in the advanced panel, e.g. export. */
	advanced?: ReactNode;
}

const PANELS: {
	id: InspectorPanel;
	label: string;
	icon: typeof TagIcon;
	description: string;
}[] = [
	{
		id: "identity",
		label: "Identity & media",
		icon: PencilIcon,
		description: "Name, summary, description and artwork",
	},
	{
		id: "access",
		label: "Access & sharing",
		icon: ShieldIcon,
		description: "Visibility, team unlock and forking",
	},
	{
		id: "listing",
		label: "Store listing",
		icon: StarIcon,
		description: "Categories, tags and links",
	},
	{
		id: "compliance",
		label: "Compliance",
		icon: ShieldIcon,
		description: "EU AI Act assessment and publication review",
	},
	{
		id: "release",
		label: "Pricing & release",
		icon: DollarSignIcon,
		description: "Status, version, price and changelog",
	},
	{
		id: "advanced",
		label: "Advanced",
		icon: BombIcon,
		description: "Export and deletion",
	},
];

function PanelHeader({
	title,
	description,
	dirty,
	onSave,
	onReset,
	saving,
}: Readonly<{
	title: string;
	description: string;
	dirty?: boolean;
	onSave?: () => void;
	onReset?: () => void;
	saving?: boolean;
}>) {
	return (
		<div className="flex flex-col gap-3 border-b pb-3 sm:flex-row sm:items-start">
			<div className="min-w-0 flex-1">
				<h3 className="text-sm font-semibold">{title}</h3>
				<p className="text-xs text-muted-foreground">{description}</p>
			</div>
			{dirty && (
				<div className="flex shrink-0 gap-2">
					<Button variant="outline" size="sm" onClick={onReset}>
						<RotateCcwIcon className="mr-1 h-3 w-3" />
						Revert
					</Button>
					<Button size="sm" onClick={onSave} disabled={saving}>
						<SaveIcon className="mr-1 h-3 w-3" />
						Save
					</Button>
				</div>
			)}
		</div>
	);
}

/**
 * Everything that used to live in the "Details" tab, split into six named
 * panels and opened from the thing it configures. Each editable panel commits
 * on its own, which removes the floating unsaved-changes bar and makes the
 * instant-commit controls (visibility, forking) consistent with the rest.
 */
export function SettingsInspector({
	appId,
	app,
	metadata,
	canEdit,
	draft,
	open,
	panel,
	onOpenChange,
	onPanelChange,
	onDeleted,
	onMediaChanged,
	suggestedType,
	slots,
}: Readonly<{
	appId: string;
	app: IApp;
	metadata: IMetadata;
	canEdit: boolean;
	draft: ProjectDraft;
	open: boolean;
	panel: InspectorPanel;
	onOpenChange: (open: boolean) => void;
	onPanelChange: (panel: InspectorPanel) => void;
	onDeleted: () => Promise<void> | void;
	onMediaChanged: () => Promise<void> | void;
	/** Guess derived from the app's contents, offered when no type is set. */
	suggestedType?: IAppType | null;
	slots?: InspectorSlots;
}>) {
	const backend = useBackend();
	const { developerMode } = useDeveloperMode();
	const [newTag, setNewTag] = useState("");
	const [longDescOpen, setLongDescOpen] = useState(false);
	const [longDescDraft, setLongDescDraft] = useState("");

	const { draftApp, draftMetadata, setDraftApp, setDraftMetadata } = draft;
	const panels = useMemo(
		() =>
			developerMode
				? PANELS
				: PANELS.filter(
						(entry) => !["listing", "compliance", "release"].includes(entry.id),
					),
		[developerMode],
	);
	// A deep link to a hidden panel falls back to the first visible one.
	const activePanel = panels.some((entry) => entry.id === panel)
		? panel
		: panels[0].id;
	const active = panels.find((entry) => entry.id === activePanel) ?? panels[0];

	const handleMediaUpload = useCallback(
		(type: "thumbnail" | "icon") => {
			if (!canEdit) return;
			const input = document.createElement("input");
			input.type = "file";
			input.accept = "image/jpeg,image/jpg,image/png,image/webp";
			input.onchange = async (event) => {
				const file = (event.target as HTMLInputElement).files?.[0];
				if (!file) return;
				const maxSize = type === "thumbnail" ? 30 : 20;
				if (file.size > maxSize * 1024 * 1024) {
					toast.error(`File too large (max ${maxSize}MB).`);
					return;
				}
				const loading = toast.loading(`Uploading ${type}…`);
				try {
					await backend.appState.pushAppMedia(appId, type, file);
					await onMediaChanged();
					toast.success(
						`${type === "thumbnail" ? "Banner" : "Icon"} uploaded`,
						{ id: loading },
					);
				} catch (error) {
					toast.error(
						error instanceof Error ? error.message : "Upload failed",
						{ id: loading },
					);
				} finally {
					toast.dismiss(loading);
				}
			};
			input.click();
		},
		[appId, canEdit, backend.appState, onMediaChanged],
	);

	const addTag = useCallback(
		(tag: string) => {
			const trimmed = tag.trim();
			if (!draftMetadata || !canEdit || !trimmed) return;
			if (draftMetadata.tags?.includes(trimmed)) return;
			setDraftMetadata({
				...draftMetadata,
				tags: [...(draftMetadata.tags ?? []), trimmed],
			});
			setNewTag("");
		},
		[draftMetadata, canEdit, setDraftMetadata],
	);

	const removeTag = useCallback(
		(tag: string) => {
			if (!draftMetadata || !canEdit) return;
			setDraftMetadata({
				...draftMetadata,
				tags: (draftMetadata.tags ?? []).filter((entry) => entry !== tag),
			});
		},
		[draftMetadata, canEdit, setDraftMetadata],
	);

	return (
		<Sheet open={open} onOpenChange={onOpenChange}>
			<SheetContent
				side="right"
				className="flex w-full flex-col gap-0 p-0 sm:max-w-xl"
			>
				<SheetHeader className="border-b px-5 py-4">
					<SheetTitle>Settings</SheetTitle>
				</SheetHeader>

				<div className="flex min-h-0 flex-1 flex-col sm:flex-row">
					<nav className="flex shrink-0 overflow-x-auto border-b py-2 sm:block sm:w-42 sm:overflow-x-hidden sm:overflow-y-auto sm:border-r sm:border-b-0">
						{panels.map((entry) => {
							const Icon = entry.icon;
							const dirty = draft.isPanelDirty(entry.id);
							return (
								<button
									key={entry.id}
									type="button"
									onClick={() => onPanelChange(entry.id)}
									className={cn(
										"flex w-auto shrink-0 items-center gap-2 whitespace-nowrap px-3 py-2 text-left text-xs transition-colors sm:w-full sm:whitespace-normal",
										entry.id === activePanel
											? "bg-primary/10 font-medium text-primary"
											: "text-muted-foreground hover:bg-muted hover:text-foreground",
										entry.id === "advanced" && "text-destructive",
									)}
								>
									<Icon className="h-3.5 w-3.5 shrink-0" />
									<span className="truncate">{entry.label}</span>
									{dirty && (
										<span className="ml-auto h-1.5 w-1.5 shrink-0 rounded-full bg-primary" />
									)}
								</button>
							);
						})}
					</nav>

					<div className="min-h-0 min-w-0 flex-1 overflow-y-auto p-3 sm:p-5">
						<div className="space-y-4">
							<PanelHeader
								title={active.label}
								description={active.description}
								dirty={draft.isPanelDirty(activePanel)}
								saving={draft.isSaving}
								onSave={() => draft.savePanel(activePanel)}
								onReset={() => draft.resetPanel(activePanel)}
							/>

							{activePanel === "identity" && draftMetadata && draftApp && (
								<div className="space-y-4">
									<div className="space-y-2">
										<Label>App type</Label>
										<Select
											value={draftApp.app_type ?? "unset"}
											disabled={!canEdit}
											onValueChange={(value) =>
												setDraftApp({
													...draftApp,
													app_type:
														value === "unset" ? null : (value as IAppType),
												})
											}
										>
											<SelectTrigger>
												<SelectValue placeholder="Choose a type" />
											</SelectTrigger>
											<SelectContent>
												<SelectItem value="unset">
													<span className="text-muted-foreground">
														Unclassified
													</span>
												</SelectItem>
												{APP_TYPE_ORDER.map((type) => {
													const meta = APP_TYPE_META[type];
													const Icon = meta.icon;
													return (
														<SelectItem key={type} value={type}>
															<span className="flex items-center gap-2">
																<Icon className="h-3.5 w-3.5" />
																{meta.label}
															</span>
														</SelectItem>
													);
												})}
											</SelectContent>
										</Select>
										<p className="text-xs text-muted-foreground">
											{appTypeMeta(draftApp.app_type).description} Shown as the
											shape of this app's icon in your library, in project
											config and in the store.
										</p>
										{!draftApp.app_type && suggestedType && (
											<button
												type="button"
												disabled={!canEdit}
												className="flex w-full items-center gap-2 rounded-md border border-dashed border-primary/40 bg-primary/5 px-3 py-2 text-left text-xs transition-colors hover:bg-primary/10"
												onClick={() =>
													setDraftApp({ ...draftApp, app_type: suggestedType })
												}
											>
												<SparklesIcon className="h-3.5 w-3.5 shrink-0 text-primary" />
												<span>
													Looks like a{" "}
													<span className="font-medium text-foreground">
														{appTypeMeta(suggestedType).label}
													</span>{" "}
													based on this app's triggers and pages — use that?
												</span>
											</button>
										)}
									</div>

									<div className="space-y-2">
										<Label>Name</Label>
										<Input
											value={draftMetadata.name}
											disabled={!canEdit}
											onChange={(event) =>
												setDraftMetadata({
													...draftMetadata,
													name: event.target.value,
												})
											}
										/>
									</div>
									<div className="space-y-2">
										<Label>Summary</Label>
										<Textarea
											rows={2}
											placeholder="One or two sentences shown under the name."
											value={draftMetadata.description}
											disabled={!canEdit}
											onChange={(event) =>
												setDraftMetadata({
													...draftMetadata,
													description: event.target.value,
												})
											}
										/>
									</div>
									<div className="space-y-2">
										<div className="flex items-center justify-between">
											<Label>Full description</Label>
											<Button
												variant="outline"
												size="sm"
												disabled={!canEdit}
												onClick={() => {
													setLongDescDraft(
														draftMetadata.long_description ?? "",
													);
													setLongDescOpen(true);
												}}
											>
												Open Markdown editor
											</Button>
										</div>
										<div className="min-h-15 rounded-md border p-3 text-sm text-muted-foreground">
											{draftMetadata.long_description ? (
												<span className="line-clamp-3">
													{draftMetadata.long_description.substring(0, 240)}
												</span>
											) : (
												<span className="italic">No full description yet</span>
											)}
										</div>
									</div>
									<div className="space-y-2">
										<Label>Artwork</Label>
										<div className="grid grid-cols-2 gap-3">
											<button
												type="button"
												disabled={!canEdit}
												className="rounded-lg border-2 border-dashed bg-transparent p-3 text-center transition-colors hover:border-primary"
												onClick={() => handleMediaUpload("icon")}
											>
												<ImageIcon className="mx-auto mb-1 h-5 w-5 text-muted-foreground" />
												<p className="text-xs text-muted-foreground">
													{metadata.icon ? "Change icon" : "Upload icon"}
												</p>
											</button>
											<button
												type="button"
												disabled={!canEdit}
												className="rounded-lg border-2 border-dashed bg-transparent p-3 text-center transition-colors hover:border-primary"
												onClick={() => handleMediaUpload("thumbnail")}
											>
												<ImageIcon className="mx-auto mb-1 h-5 w-5 text-muted-foreground" />
												<p className="text-xs text-muted-foreground">
													{metadata.thumbnail
														? "Change banner"
														: "Upload banner"}
												</p>
											</button>
										</div>
									</div>
								</div>
							)}

							{activePanel === "access" && (
								<div className="space-y-4">
									{slots?.access ?? (
										<p className="text-sm text-muted-foreground">
											Sharing controls are unavailable for this deployment.
										</p>
									)}
								</div>
							)}

							{activePanel === "listing" && draftApp && draftMetadata && (
								<div className="space-y-4">
									<div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
										<div className="space-y-2">
											<Label>Primary category</Label>
											<Select
												value={draftApp.primary_category ?? IAppCategory.Other}
												disabled={!canEdit}
												onValueChange={(value) =>
													setDraftApp({
														...draftApp,
														primary_category: value as IAppCategory,
													})
												}
											>
												<SelectTrigger>
													<SelectValue placeholder="Select category" />
												</SelectTrigger>
												<SelectContent>
													{Object.values(IAppCategory).map((category) => (
														<SelectItem key={category} value={category}>
															{formatAppCategory(category)}
														</SelectItem>
													))}
												</SelectContent>
											</Select>
										</div>
										<div className="space-y-2">
											<Label>Secondary category</Label>
											<Select
												value={draftApp.secondary_category ?? "none"}
												disabled={!canEdit}
												onValueChange={(value) =>
													setDraftApp({
														...draftApp,
														secondary_category:
															value === "none" ? null : (value as IAppCategory),
													})
												}
											>
												<SelectTrigger>
													<SelectValue placeholder="None" />
												</SelectTrigger>
												<SelectContent>
													<SelectItem value="none">None</SelectItem>
													{Object.values(IAppCategory).map((category) => (
														<SelectItem key={category} value={category}>
															{formatAppCategory(category)}
														</SelectItem>
													))}
												</SelectContent>
											</Select>
										</div>
									</div>

									<div className="space-y-2">
										<Label>Tags</Label>
										<Input
											placeholder="Type a tag and press Enter…"
											value={newTag}
											disabled={!canEdit}
											onChange={(event) => setNewTag(event.target.value)}
											onKeyDown={(event) => {
												if (event.key === "Enter") {
													event.preventDefault();
													addTag(newTag);
												}
											}}
										/>
										{(draftMetadata.tags?.length ?? 0) > 0 && (
											<div className="flex flex-wrap gap-1.5 pt-1">
												{draftMetadata.tags.map((tag) => (
													<Badge
														key={tag}
														variant="secondary"
														className="flex items-center gap-1"
													>
														{tag}
														{canEdit && (
															<button
																type="button"
																onClick={() => removeTag(tag)}
																aria-label={`Remove ${tag}`}
																className="ml-0.5 hover:text-destructive"
															>
																<XIcon className="h-3 w-3" />
															</button>
														)}
													</Badge>
												))}
											</div>
										)}
									</div>

									{(
										[
											["website", "Website", "https://yourapp.com"],
											["docs_url", "Documentation", "https://docs.yourapp.com"],
											["support_url", "Support", "https://support.yourapp.com"],
										] as const
									).map(([field, label, placeholder]) => (
										<div key={field} className="space-y-2">
											<Label>{label}</Label>
											<Input
												placeholder={placeholder}
												value={draftMetadata[field] ?? ""}
												disabled={!canEdit}
												onChange={(event) =>
													setDraftMetadata({
														...draftMetadata,
														[field]: event.target.value,
													})
												}
											/>
										</div>
									))}

									{slots?.reviews && (
										<div className="pt-2">{slots.reviews}</div>
									)}
								</div>
							)}

							{activePanel === "compliance" && (
								<div className="space-y-4">
									{slots?.compliance ?? (
										<p className="text-sm text-muted-foreground">
											Conformity checks are unavailable for this deployment.
										</p>
									)}
								</div>
							)}

							{activePanel === "release" && draftApp && (
								<div className="space-y-4">
									<div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
										<div className="space-y-2">
											<Label>Status</Label>
											<Select
												value={draftApp.status ?? IAppStatus.Active}
												disabled={!canEdit}
												onValueChange={(value) =>
													setDraftApp({
														...draftApp,
														status: value as IAppStatus,
													})
												}
											>
												<SelectTrigger>
													<SelectValue />
												</SelectTrigger>
												<SelectContent>
													{Object.values(IAppStatus).map((status) => (
														<SelectItem key={status} value={status}>
															{status}
														</SelectItem>
													))}
												</SelectContent>
											</Select>
										</div>
										<div className="space-y-2">
											<Label>Version</Label>
											<Input
												placeholder="1.0.0"
												value={draftApp.version ?? ""}
												disabled={!canEdit}
												onChange={(event) =>
													setDraftApp({
														...draftApp,
														version: event.target.value,
													})
												}
											/>
										</div>
										<div className="space-y-2">
											<Label>Price ($)</Label>
											<Input
												type="number"
												placeholder="0.00"
												value={draftApp.price ?? ""}
												disabled={!canEdit}
												onChange={(event) =>
													setDraftApp({
														...draftApp,
														price:
															Number.parseFloat(event.target.value) || null,
													})
												}
											/>
										</div>
									</div>
									<div className="space-y-2">
										<Label>Changelog</Label>
										<Textarea
											rows={4}
											placeholder="What is new in this version…"
											value={draftApp.changelog ?? ""}
											disabled={!canEdit}
											onChange={(event) =>
												setDraftApp({
													...draftApp,
													changelog: event.target.value,
												})
											}
										/>
									</div>
								</div>
							)}

							{activePanel === "advanced" && (
								<div className="space-y-4">
									{slots?.advanced}
									{canEdit && (
										<div className="space-y-2 rounded-lg border border-destructive/40 p-4">
											<h4 className="text-sm font-semibold text-destructive">
												Delete this app
											</h4>
											<p className="text-xs text-muted-foreground">
												Removes every flow, event, page and stored file. This
												cannot be undone.
											</p>
											<VerificationDialog
												dialog="You cannot undo this action. This will permanently delete the app!"
												onConfirm={async () => {
													await onDeleted();
												}}
											>
												<Button variant="destructive" size="sm">
													<BombIcon className="mr-1.5 h-3 w-3" />
													Delete app
												</Button>
											</VerificationDialog>
										</div>
									)}
								</div>
							)}
						</div>
					</div>
				</div>

				<Dialog open={longDescOpen} onOpenChange={setLongDescOpen}>
					<DialogContent className="flex max-h-svh min-h-svh w-dvw min-w-dvw max-w-dvw flex-col">
						<DialogHeader>
							<DialogTitle>Full description</DialogTitle>
						</DialogHeader>
						<div className="min-h-0 flex-1 overflow-auto p-2">
							<TextEditor
								appId={appId}
								editable={canEdit}
								isMarkdown
								initialContent={
									longDescDraft || "*No detailed description available.*"
								}
								onChange={(content: string) => setLongDescDraft(content)}
							/>
						</div>
						<div className="flex justify-end gap-2">
							<Button variant="outline" onClick={() => setLongDescOpen(false)}>
								Cancel
							</Button>
							<Button
								onClick={() => {
									if (draftMetadata) {
										setDraftMetadata({
											...draftMetadata,
											long_description: longDescDraft,
										});
									}
									setLongDescOpen(false);
								}}
							>
								Done
							</Button>
						</div>
					</DialogContent>
				</Dialog>
			</SheetContent>
		</Sheet>
	);
}
