"use client";

import {
	BombIcon,
	CalendarIcon,
	ExternalLinkIcon,
	EyeIcon,
	ImageIcon,
	InfoIcon,
	RotateCcwIcon,
	SaveIcon,
	SettingsIcon,
	ShieldIcon,
	TagIcon,
	XIcon,
} from "lucide-react";
import type { ReactNode } from "react";
import { useCallback, useEffect, useRef, useState } from "react";
import { formatAppCategory } from "../../../lib/app-category";
import type { IApp, IMetadata } from "../../../types";
import { IAppCategory, IAppStatus } from "../../../types";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../../ui/card";
import { Dialog, DialogContent } from "../../ui/dialog";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";
import { TextEditor } from "../../ui/text-editor";
import { Textarea } from "../../ui/textarea";
import { VerificationDialog } from "../../verification-dialog";

export interface AppGeneralSettingsProps {
	app: IApp;
	metadata: IMetadata;
	canEdit: boolean;
	hasChanges: boolean;
	onAppChange: (app: IApp) => void;
	onMetadataChange: (metadata: IMetadata) => void;
	onSave: () => Promise<void>;
	onReset: () => void;
	onDelete?: () => Promise<void>;
	onThumbnailUpload?: () => Promise<void>;
	onIconUpload?: () => Promise<void>;
	visibilityStatusSwitcher?: ReactNode;
}

export function AppGeneralSettings({
	app,
	metadata,
	canEdit,
	hasChanges,
	onAppChange,
	onMetadataChange,
	onSave,
	onReset,
	onDelete,
	onThumbnailUpload,
	onIconUpload,
	visibilityStatusSwitcher,
}: AppGeneralSettingsProps) {
	const [newTag, setNewTag] = useState("");
	const [isLongDescEditorOpen, setLongDescEditorOpen] = useState(false);
	const [longDescInit, setLongDescInit] = useState<string>("");
	const [longDescDraft, setLongDescDraft] = useState<string>("");
	const editorAreaRef = useRef<HTMLDivElement | null>(null);

	const openLongDescEditor = useCallback(() => {
		const initial = metadata.long_description || "";
		setLongDescInit(initial);
		setLongDescDraft(initial);
		setLongDescEditorOpen(true);
	}, [metadata]);

	useEffect(() => {
		if (!isLongDescEditorOpen) return;
		const prev = document.body.style.overflow;
		document.body.style.overflow = "hidden";
		return () => {
			document.body.style.overflow = prev;
		};
	}, [isLongDescEditorOpen]);

	const addTag = useCallback(
		(tag: string) => {
			if (!canEdit || !tag.trim()) return;
			const trimmedTag = tag.trim();
			if (metadata.tags?.includes(trimmedTag)) return;
			onMetadataChange({
				...metadata,
				tags: [...(metadata.tags || []), trimmedTag],
			});
			setNewTag("");
		},
		[metadata, canEdit, onMetadataChange],
	);

	const removeTag = useCallback(
		(tagToRemove: string) => {
			if (!canEdit) return;
			onMetadataChange({
				...metadata,
				tags: metadata.tags?.filter((tag) => tag !== tagToRemove) || [],
			});
		},
		[metadata, canEdit, onMetadataChange],
	);

	const handleTagInputKeyDown = useCallback(
		(e: React.KeyboardEvent) => {
			if (e.key === "Enter") {
				e.preventDefault();
				addTag(newTag);
			}
		},
		[newTag, addTag],
	);

	return (
		<div className="w-full max-w-6xl mx-auto p-2 md:p-6 pt-0 space-y-6 flex flex-col flex-grow max-h-full min-h-0 overflow-auto md:overflow-visible">
			{/* Header with Save Button - Made Sticky */}
			{hasChanges && canEdit && (
				<div className="sticky top-0 z-10 mb-6">
					<Card className="border-orange-200 bg-orange-50 dark:border-orange-800 dark:bg-orange-950">
						<CardContent>
							<div className="flex items-center justify-between">
								<div className="flex items-center gap-2">
									<InfoIcon className="w-5 h-5 text-orange-600" />
									<span className="font-medium text-orange-800 dark:text-orange-200">
										You have unsaved changes
									</span>
								</div>
								<div className="flex gap-2">
									<Button variant="outline" onClick={onReset} className="gap-2">
										<RotateCcwIcon className="w-4 h-4" />
										Reset
									</Button>
									<Button onClick={onSave} className="gap-2">
										<SaveIcon className="w-4 h-4" />
										Save Changes
									</Button>
								</div>
							</div>
						</CardContent>
					</Card>
				</div>
			)}

			{/* Basic Information */}
			<Card>
				<CardHeader>
					<CardTitle className="flex items-center gap-2">
						<InfoIcon className="w-5 h-5" />
						Basic Information
					</CardTitle>
					<CardDescription>
						Configure the basic details of your application
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-4">
					<div className="grid grid-cols-1 md:grid-cols-2 gap-4">
						<div className="space-y-2">
							<Label htmlFor="name">Name</Label>
							<Input
								id="name"
								placeholder="Application name"
								value={metadata?.name ?? ""}
								disabled={!canEdit}
								onChange={(e) => {
									if (canEdit) {
										onMetadataChange({
											...metadata,
											name: e.target.value,
										});
									}
								}}
							/>
						</div>
						<div className="space-y-2">
							<Label htmlFor="version">Version</Label>
							<Input
								id="version"
								placeholder="1.0.0"
								value={app?.version ?? ""}
								disabled={!canEdit}
								onChange={(e) => {
									if (canEdit) {
										onAppChange({
											...app,
											version: e.target.value,
										});
									}
								}}
							/>
						</div>
					</div>
					<div className="space-y-2">
						<Label htmlFor="description">Short Description</Label>
						<Textarea
							id="description"
							placeholder="Brief description in 1-2 sentences..."
							rows={2}
							value={metadata?.description ?? ""}
							disabled={!canEdit}
							onChange={(e) => {
								if (canEdit) {
									onMetadataChange({
										...metadata,
										description: e.target.value,
									});
								}
							}}
						/>
					</div>
					{/* Long Description with fullscreen markdown editor trigger */}
					<div className="space-y-2">
						<div className="flex items-center justify-between">
							<Label htmlFor="long-description">Long Description</Label>
							{canEdit && (
								<Button
									variant="outline"
									size="sm"
									onClick={openLongDescEditor}
								>
									Open Markdown Editor
								</Button>
							)}
						</div>
						{/* Preview-only renderer for the long description */}
						<div id="long-description" className="rounded-md">
							<TextEditor
								isMarkdown
								editable={false}
								initialContent={
									metadata?.long_description ||
									"*No detailed description available.*"
								}
							/>
						</div>
					</div>
				</CardContent>
			</Card>

			{/* Visibility Status */}
			{visibilityStatusSwitcher}

			{/* Visual Assets */}
			<Card>
				<CardHeader>
					<CardTitle className="flex items-center gap-2">
						<ImageIcon className="w-5 h-5" />
						Visual Assets
					</CardTitle>
					<CardDescription>
						Upload thumbnail and icon for your application
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-6">
					<div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
						{/* Thumbnail Upload */}
						<div className="space-y-3">
							<div className="flex items-center justify-between">
								<Label className="text-sm font-medium">Thumbnail</Label>
								<Badge variant="outline" className="text-xs">
									1280 × 640px
								</Badge>
							</div>
							<div
								className={`relative group border-2 border-dashed rounded-lg overflow-hidden transition-all duration-200 ${
									canEdit && onThumbnailUpload
										? "border-gray-300 dark:border-gray-700 hover:border-primary cursor-pointer"
										: "border-gray-200 dark:border-gray-800 cursor-not-allowed opacity-60"
								}`}
								style={{ aspectRatio: "2/1" }}
								onClick={canEdit ? onThumbnailUpload : undefined}
							>
								{/* Current thumbnail or placeholder */}
								<div className="absolute inset-0">
									<img
										src={metadata?.thumbnail ?? "/placeholder-thumbnail.webp"}
										alt="App thumbnail"
										className="w-full h-full object-cover"
									/>
									{/* Overlay */}
									{canEdit && onThumbnailUpload && (
										<div className="absolute inset-0 bg-black/0 group-hover:bg-black/40 transition-all duration-200 flex items-center justify-center">
											<div className="opacity-0 group-hover:opacity-100 transition-opacity duration-200 flex flex-col items-center gap-2 text-white">
												<ImageIcon className="w-8 h-8" />
												<span className="text-sm font-medium">
													{metadata?.thumbnail
														? "Change Thumbnail"
														: "Upload Thumbnail"}
												</span>
											</div>
										</div>
									)}
								</div>
								{/* Upload indicator */}
								{!metadata?.thumbnail && (
									<div className="absolute inset-0 flex items-center justify-center">
										<div className="text-center text-gray-500 dark:text-gray-400">
											<ImageIcon className="w-12 h-12 mx-auto mb-2 opacity-50" />
											<p className="text-sm">
												{canEdit && onThumbnailUpload
													? "Click to upload"
													: "No thumbnail"}
											</p>
										</div>
									</div>
								)}
							</div>
						</div>

						{/* Icon Upload */}
						<div className="space-y-3">
							<div className="flex items-center justify-between">
								<Label className="text-sm font-medium">Icon</Label>
								<Badge variant="outline" className="text-xs">
									1024 × 1024px
								</Badge>
							</div>
							<div className="flex justify-center">
								<div
									className={`relative group border-2 border-dashed rounded-lg overflow-hidden transition-all duration-200 w-40 h-40 ${
										canEdit && onIconUpload
											? "border-gray-300 dark:border-gray-700 hover:border-primary cursor-pointer"
											: "border-gray-200 dark:border-gray-800 cursor-not-allowed opacity-60"
									}`}
									onClick={canEdit ? onIconUpload : undefined}
								>
									{/* Current icon or placeholder */}
									<div className="absolute inset-0">
										<img
											src={metadata?.icon ?? "/app-logo.webp"}
											alt="App icon"
											className="w-full h-full object-cover rounded-lg"
										/>
										{/* Overlay */}
										{canEdit && onIconUpload && (
											<div className="absolute inset-0 bg-black/0 group-hover:bg-black/40 transition-all duration-200 flex items-center justify-center rounded-lg">
												<div className="opacity-0 group-hover:opacity-100 transition-opacity duration-200 flex flex-col items-center gap-1 text-white">
													<ImageIcon className="w-6 h-6" />
													<span className="text-xs font-medium text-center">
														{metadata?.icon ? "Change Icon" : "Upload Icon"}
													</span>
												</div>
											</div>
										)}
									</div>
									{/* Upload indicator */}
									{!metadata?.icon && (
										<div className="absolute inset-0 flex items-center justify-center">
											<div className="text-center text-gray-500 dark:text-gray-400">
												<ImageIcon className="w-8 h-8 mx-auto mb-1 opacity-50" />
												<p className="text-xs">
													{canEdit && onIconUpload
														? "Click to upload"
														: "No icon"}
												</p>
											</div>
										</div>
									)}
								</div>
							</div>
						</div>
					</div>
				</CardContent>
			</Card>

			{/* Categories and Tags */}
			<Card>
				<CardHeader>
					<CardTitle className="flex items-center gap-2">
						<TagIcon className="w-5 h-5" />
						Categories & Tags
					</CardTitle>
					<CardDescription>
						Organize your application with categories and tags
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-4">
					<div className="grid grid-cols-1 md:grid-cols-2 gap-4">
						<div className="space-y-2">
							<Label htmlFor="primary-category">Primary Category</Label>
							<Select
								value={app?.primary_category ?? IAppCategory.Other}
								onValueChange={(value) => {
									if (canEdit) {
										onAppChange({
											...app,
											primary_category: value as IAppCategory,
										});
									}
								}}
								disabled={!canEdit}
							>
								<SelectTrigger>
									<SelectValue placeholder="Select primary category" />
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
							<Label htmlFor="secondary-category">Secondary Category</Label>
							<Select
								value={app?.secondary_category ?? ""}
								onValueChange={(value) => {
									if (canEdit) {
										onAppChange({
											...app,
											secondary_category:
												value === "none" ? null : (value as IAppCategory),
										});
									}
								}}
								disabled={!canEdit}
							>
								<SelectTrigger>
									<SelectValue placeholder="Select secondary category" />
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

					{/* Tags Section */}
					<div className="space-y-2">
						<Label htmlFor="tags">Tags</Label>
						<div className="space-y-2">
							<Input
								id="tags"
								placeholder="Type a tag and press Enter..."
								value={newTag}
								disabled={!canEdit}
								onChange={(e) => setNewTag(e.target.value)}
								onKeyDown={handleTagInputKeyDown}
							/>
							{metadata?.tags && metadata.tags.length > 0 && (
								<div className="flex flex-wrap gap-2">
									{metadata.tags.map((tag, index) => (
										<Badge
											key={index}
											variant="secondary"
											className="flex items-center gap-1"
										>
											{tag}
											{canEdit && (
												<button
													onClick={() => removeTag(tag)}
													className="ml-1 hover:text-red-500"
												>
													<XIcon className="w-3 h-3" />
												</button>
											)}
										</Badge>
									))}
								</div>
							)}
						</div>
					</div>
				</CardContent>
			</Card>

			{/* Support & Links */}
			<Card>
				<CardHeader>
					<CardTitle className="flex items-center gap-2">
						<ExternalLinkIcon className="w-5 h-5" />
						Support & Links
					</CardTitle>
					<CardDescription>
						Provide helpful links for users and support
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-4">
					<div className="grid grid-cols-1 gap-4">
						<div className="space-y-2">
							<Label htmlFor="website">Website</Label>
							<Input
								id="website"
								placeholder="https://yourapp.com"
								value={metadata?.website ?? ""}
								disabled={!canEdit}
								onChange={(e) => {
									if (canEdit) {
										onMetadataChange({
											...metadata,
											website: e.target.value,
										});
									}
								}}
							/>
						</div>
						<div className="space-y-2">
							<Label htmlFor="docs-url">Documentation URL</Label>
							<Input
								id="docs-url"
								placeholder="https://docs.yourapp.com"
								value={metadata?.docs_url ?? ""}
								disabled={!canEdit}
								onChange={(e) => {
									if (canEdit) {
										onMetadataChange({
											...metadata,
											docs_url: e.target.value,
										});
									}
								}}
							/>
						</div>
						<div className="space-y-2">
							<Label htmlFor="support-url">Support URL</Label>
							<Input
								id="support-url"
								placeholder="https://support.yourapp.com"
								value={metadata?.support_url ?? ""}
								disabled={!canEdit}
								onChange={(e) => {
									if (canEdit) {
										onMetadataChange({
											...metadata,
											support_url: e.target.value,
										});
									}
								}}
							/>
						</div>
					</div>
				</CardContent>
			</Card>

			{/* App Settings */}
			<Card>
				<CardHeader>
					<CardTitle className="flex items-center gap-2">
						<SettingsIcon className="w-5 h-5" />
						Application Settings
					</CardTitle>
					<CardDescription>Configure application behavior</CardDescription>
				</CardHeader>
				<CardContent className="space-y-4">
					<div className="grid grid-cols-1 md:grid-cols-2 gap-4">
						<div className="space-y-2">
							<Label htmlFor="status">Status</Label>
							<Select
								value={app?.status ?? IAppStatus.Active}
								onValueChange={(value) => {
									if (canEdit) {
										onAppChange({
											...app,
											status: value as IAppStatus,
										});
									}
								}}
								disabled={!canEdit}
							>
								<SelectTrigger>
									<SelectValue placeholder="Select status" />
								</SelectTrigger>
								<SelectContent>
									{Object.values(IAppStatus).map((status) => (
										<SelectItem key={status} value={status}>
											<div className="flex items-center gap-2">
												<div
													className={`w-2 h-2 rounded-full ${
														status === IAppStatus.Active
															? "bg-green-500"
															: status === IAppStatus.Inactive
																? "bg-yellow-500"
																: "bg-gray-500"
													}`}
												/>
												{status}
											</div>
										</SelectItem>
									))}
								</SelectContent>
							</Select>
						</div>
						<div className="space-y-2">
							<Label htmlFor="price">Price ($)</Label>
							<Input
								id="price"
								type="number"
								placeholder="0.00"
								value={app?.price ?? ""}
								disabled={!canEdit}
								onChange={(e) => {
									if (canEdit) {
										onAppChange({
											...app,
											price: Number.parseFloat(e.target.value) || null,
										});
									}
								}}
							/>
						</div>
					</div>
				</CardContent>
			</Card>

			{/* Changelog */}
			<Card>
				<CardHeader>
					<CardTitle className="flex items-center gap-2">
						<CalendarIcon className="w-5 h-5" />
						Changelog
					</CardTitle>
					<CardDescription>
						Document what&apos;s new in this version
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-4">
					<div className="space-y-2">
						<Label htmlFor="changelog">What&apos;s New</Label>
						<Textarea
							id="changelog"
							placeholder="What's new in this version..."
							rows={4}
							value={app?.changelog ?? ""}
							disabled={!canEdit}
							onChange={(e) => {
								if (canEdit) {
									onAppChange({
										...app,
										changelog: e.target.value,
									});
								}
							}}
						/>
					</div>
				</CardContent>
			</Card>

			{/* Statistics (Read-only) */}
			<Card>
				<CardHeader>
					<CardTitle className="flex items-center gap-2">
						<CalendarIcon className="w-5 h-5" />
						Statistics
					</CardTitle>
					<CardDescription>
						Application performance and engagement metrics
					</CardDescription>
				</CardHeader>
				<CardContent>
					<div className="grid grid-cols-2 md:grid-cols-4 gap-4">
						<div className="text-center p-4 border rounded-lg">
							<div className="text-2xl font-bold text-blue-600">
								{app.download_count}
							</div>
							<div className="text-sm text-gray-600 dark:text-gray-400">
								Downloads
							</div>
						</div>
						<div className="text-center p-4 border rounded-lg">
							<div className="text-2xl font-bold text-green-600">
								{app.rating_count}
							</div>
							<div className="text-sm text-gray-600 dark:text-gray-400">
								Ratings
							</div>
						</div>
						<div className="text-center p-4 border rounded-lg">
							<div className="text-2xl font-bold text-purple-600">
								{app.interactions_count}
							</div>
							<div className="text-sm text-gray-600 dark:text-gray-400">
								Interactions
							</div>
						</div>
						<div className="text-center p-4 border rounded-lg">
							<div className="text-2xl font-bold text-orange-600">
								{app.avg_rating ? app.avg_rating.toFixed(1) : "N/A"}
							</div>
							<div className="text-sm text-gray-600 dark:text-gray-400">
								Avg Rating
							</div>
						</div>
					</div>
				</CardContent>
			</Card>

			{/* Danger Zone */}
			{canEdit && onDelete && (
				<Card className="border-red-200 dark:border-red-800">
					<CardHeader>
						<CardTitle className="flex items-center gap-2 text-red-600 dark:text-red-400">
							<ShieldIcon className="w-5 h-5" />
							Danger Zone
						</CardTitle>
						<CardDescription>
							Irreversible actions that will permanently affect your application
						</CardDescription>
					</CardHeader>
					<CardContent>
						<VerificationDialog
							dialog="You cannot undo this action. This will remove the app from your System!"
							onConfirm={onDelete}
						>
							<Button variant="destructive" className="gap-2">
								<BombIcon className="w-4 h-4" />
								Delete App
							</Button>
						</VerificationDialog>
					</CardContent>
				</Card>
			)}

			{/* Permission Notice */}
			{!canEdit && (
				<Card className="border-blue-200 bg-blue-50 dark:border-blue-800 dark:bg-blue-950">
					<CardContent className="pt-6">
						<div className="flex items-center gap-2 text-blue-800 dark:text-blue-200">
							<EyeIcon className="w-5 h-5" />
							<span className="font-medium">Read-only mode</span>
							<span className="text-sm">
								You don&apos;t have edit permissions for this application
							</span>
						</div>
					</CardContent>
				</Card>
			)}

			{/* Fullscreen Markdown Editor Overlay */}
			<Dialog open={isLongDescEditorOpen} onOpenChange={setLongDescEditorOpen}>
				<DialogContent
					className="w-dvw min-w-dvw max-w-dvw min-h-[100svh] max-h-[100svh] flex flex-col"
					onEscapeKeyDown={(e) => {
						const target = e.target as Node | null;
						if (target && editorAreaRef.current?.contains(target)) {
							e.preventDefault();
						}
					}}
				>
					<div className="flex items-center justify-between px-6 py-2 h-20 border-b bg-background">
						<div>
							<div className="text-lg font-semibold">Edit Long Description</div>
							<div className="text-sm text-muted-foreground">
								Markdown supported
							</div>
						</div>
						<div className="flex gap-2">
							<Button
								variant="outline"
								onClick={() => setLongDescEditorOpen(false)}
							>
								Cancel
							</Button>
							<Button
								onClick={() => {
									onMetadataChange({
										...metadata,
										long_description: longDescDraft,
									});
									setLongDescEditorOpen(false);
								}}
							>
								Done
							</Button>
						</div>
					</div>
					<div className="flex-grow overflow-hidden relative">
						<div className="h-full overflow-auto p-6">
							<div
								ref={editorAreaRef}
								onKeyDown={(e) => {
									if (e.key === "Escape") {
										e.stopPropagation();
									}
								}}
							>
								<TextEditor
									appId={app.id}
									editable={canEdit}
									isMarkdown
									initialContent={
										longDescInit || "*No detailed description available.*"
									}
									onChange={(content) => {
										setLongDescDraft(content);
									}}
								/>
							</div>
						</div>
					</div>
				</DialogContent>
			</Dialog>
		</div>
	);
}
