"use client";
import {
	AppReviewsSection,
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	Dialog,
	DialogContent,
	type IApp,
	IAppCategory,
	IAppStatus,
	IAppVisibility,
	type IMetadata,
	Input,
	Label,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	TextEditor,
	Textarea,
	VerificationDialog,
	sanitizeImageUrl,
	toastError,
	useBackend,
	useInvalidateInvoke,
	useInvoke,
} from "@tm9657/flow-like-ui";
import { isEqual } from "lodash-es";
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
import { useRouter, useSearchParams } from "next/navigation";
import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { useTauriInvoke } from "../../../components/useInvoke";
import { VisibilityStatusSwitcher } from "./visibility-status-switcher";

export default function LibraryConfigPage() {
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const searchParams = useSearchParams();
	const router = useRouter();
	const id = searchParams.get("id");

	// Global permission state - you can modify this later
	const [canEdit, setCanEdit] = useState(true);

	const app = useInvoke(
		backend.appState.getApp,
		backend.appState,
		[id ?? ""],
		typeof id === "string",
	);
	const metadata = useInvoke(
		backend.appState.getAppMeta,
		backend.appState,
		[id ?? ""],
		typeof id === "string",
	);

	const [localApp, setLocalApp] = useState<IApp | undefined>();
	const [localMetadata, setLocalMetadata] = useState<IMetadata | undefined>();
	const [hasChanges, setHasChanges] = useState(false);
	const [newTag, setNewTag] = useState("");
	const currentProfile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
		true,
	);
	// Add state for fullscreen long description editor
	const [isLongDescEditorOpen, setLongDescEditorOpen] = useState(false);
	const [longDescInit, setLongDescInit] = useState<string>("");
	const [longDescDraft, setLongDescDraft] = useState<string>("");
	const editorAreaRef = useRef<HTMLDivElement | null>(null);

	const openLongDescEditor = useCallback(() => {
		if (!localMetadata) return;
		const initial = localMetadata.long_description || "";
		setLongDescInit(initial);
		setLongDescDraft(initial);
		setLongDescEditorOpen(true);
	}, [localMetadata]);

	// Lock document scroll when fullscreen editor is open
	useEffect(() => {
		if (!isLongDescEditorOpen) return;
		const prev = document.body.style.overflow;
		document.body.style.overflow = "hidden";
		return () => {
			document.body.style.overflow = prev;
		};
	}, [isLongDescEditorOpen]);

	useEffect(() => {
		if (!metadata.data) return;
		if (localMetadata) return;
		setLocalMetadata(metadata.data);
	}, [localMetadata, metadata.data]);

	useEffect(() => {
		if (!app.data) return;
		if (localApp) return;
		setLocalApp(app.data);
	}, [localApp, app.data]);

	// Check for changes
	useEffect(() => {
		if (!app.data || !metadata.data || !localApp || !localMetadata) {
			setHasChanges(false);
			return;
		}

		const editableAppFields: (keyof IApp)[] = [
			"version",
			"primary_category",
			"secondary_category",
			"status",
			"price",
			"changelog",
		];

		const editableMetadataFields: (keyof IMetadata)[] = [
			"name",
			"description",
			"long_description",
			"website",
			"docs_url",
			"support_url",
			"tags",
		];

		const appChanged = editableAppFields.some(
			(key) => localApp[key] !== app.data[key],
		);

		const metadataChanged = editableMetadataFields.some(
			(key) => !isEqual(localMetadata[key], metadata.data[key]),
		);

		setHasChanges(appChanged || metadataChanged);
	}, [app.data, metadata.data, localApp, localMetadata]);

	const isReady = useTauriInvoke<boolean>(
		"app_configured",
		{ appId: id },
		[id ?? ""],
		typeof id === "string",
	);
	const appSize = useTauriInvoke<number>(
		"get_app_size",
		{ appId: id },
		[id ?? ""],
		typeof id === "string",
	);

	const saveChanges = useCallback(async () => {
		if (!id) {
			toastError("App ID is not defined.", <BombIcon />);
			return;
		}

		if (!localApp || !localMetadata) {
			toastError("App or metadata is not loaded.", <BombIcon />);
			return;
		}

		await backend.appState.pushAppMeta(id, localMetadata);
		await backend.appState.updateApp(localApp);
		await app.refetch();
		await metadata.refetch();
		await isReady.refetch();
		await appSize.refetch();
		await invalidate(backend.appState.getApps, []);

		toast.success("Changes saved successfully!", {
			icon: <SaveIcon className="w-4 h-4" />,
		});
	}, [backend, id, localApp, localMetadata]);

	const resetChanges = useCallback(() => {
		if (!app.data || !metadata.data) {
			toastError("App or metadata is not loaded.", <BombIcon />);
			return;
		}
		setLocalApp(app.data);
		setLocalMetadata(metadata.data);
		toast("Changes reset to last saved state.", {
			icon: <RotateCcwIcon className="w-4 h-4" />,
		});
	}, [app.data, metadata.data]);

	const refreshReviews = useCallback(async () => {
		const result = await app.refetch();
		if (result.data) {
			setLocalApp(result.data);
		}
		await invalidate(backend.appState.getApps, []);
	}, [app, backend.appState, invalidate]);

	async function deleteApp() {
		await backend.appState.deleteApp(id ?? "");
		await invalidate(backend.appState.getApps, []);
		router.push("/library");
	}

	const handleThumbnailUpload = useCallback(async () => {
		if (!id || !canEdit) {
			toastError(
				"Cannot upload thumbnail. App ID is missing or editing is disabled.",
				<BombIcon />,
			);
			return;
		}

		const input = document.createElement("input");
		input.type = "file";
		input.accept = "image/jpeg,image/jpg,image/png,image/webp";
		input.onchange = async (event) => {
			const file = (event.target as HTMLInputElement).files?.[0];
			if (!file) return;

			const validTypes = ["image/jpeg", "image/jpg", "image/png", "image/webp"];
			if (!validTypes.includes(file.type)) {
				toastError(
					"Please select a valid image file (JPEG, PNG, WebP, or GIF).",
					<BombIcon />,
				);
				return;
			}

			const maxSize = 30 * 1024 * 1024;
			if (file.size > maxSize) {
				toastError(
					"Image file is too large. Please select a file smaller than 30MB.",
					<BombIcon />,
				);
				return;
			}

			try {
				const loadingRef = toast.loading("Uploading thumbnail...", {
					icon: <ImageIcon className="w-4 h-4" />,
				});

				await backend.appState.pushAppMedia(id, "thumbnail", file);

				if (localMetadata) {
					setLocalMetadata({
						...localMetadata,
						thumbnail: URL.createObjectURL(file),
					});
				}
				toast.dismiss(loadingRef);

				toast.success(
					"Thumbnail uploaded successfully, it can take up to a minute to process!",
					{
						icon: <ImageIcon className="w-4 h-4" />,
					},
				);

				await metadata.refetch();
			} catch (error) {
				toastError(
					"Failed to upload thumbnail. Please try again.",
					<BombIcon />,
				);
				console.error("Thumbnail upload error:", error);
			}
		};
		input.click();
	}, [id, canEdit, backend.appState, localMetadata, metadata]);

	const handleIconUpload = useCallback(async () => {
		if (!id || !canEdit) {
			toastError(
				"Cannot upload icon. App ID is missing or editing is disabled.",
				<BombIcon />,
			);
			return;
		}

		const input = document.createElement("input");
		input.type = "file";
		input.accept = "image/jpeg,image/jpg,image/png,image/webp";
		input.onchange = async (event) => {
			const file = (event.target as HTMLInputElement).files?.[0];
			if (!file) return;

			const validTypes = ["image/jpeg", "image/jpg", "image/png", "image/webp"];
			if (!validTypes.includes(file.type)) {
				toastError(
					"Please select a valid image file (JPEG, PNG, WebP).",
					<BombIcon />,
				);
				return;
			}

			const maxSize = 20 * 1024 * 1024; // 20MB
			if (file.size > maxSize) {
				toastError(
					"Icon file is too large. Please select a file smaller than 20MB.",
					<BombIcon />,
				);
				return;
			}

			try {
				const loadingRef = toast.loading("Uploading icon...", {
					icon: <ImageIcon className="w-4 h-4" />,
				});

				await backend.appState.pushAppMedia(id, "icon", file);

				if (localMetadata) {
					setLocalMetadata({
						...localMetadata,
						icon: URL.createObjectURL(file),
					});
				}

				toast.dismiss(loadingRef);

				toast.success(
					"Icon uploaded successfully, it can take up to a minute to process!",
					{
						icon: <ImageIcon className="w-4 h-4" />,
					},
				);

				await metadata.refetch();
			} catch (error) {
				toastError("Failed to upload icon. Please try again.", <BombIcon />);
				console.error("Icon upload error:", error);
			}
		};
		input.click();
	}, [id, canEdit, backend.appState, localMetadata, metadata]);

	const addTag = useCallback(
		(tag: string) => {
			if (!localMetadata || !canEdit || !tag.trim()) return;

			const trimmedTag = tag.trim();
			if (localMetadata.tags?.includes(trimmedTag)) return;

			setLocalMetadata({
				...localMetadata,
				tags: [...(localMetadata.tags || []), trimmedTag],
			});
			setNewTag("");
		},
		[localMetadata, canEdit],
	);

	const removeTag = useCallback(
		(tagToRemove: string) => {
			if (!localMetadata || !canEdit) return;

			setLocalMetadata({
				...localMetadata,
				tags: localMetadata.tags?.filter((tag) => tag !== tagToRemove) || [],
			});
		},
		[localMetadata, canEdit],
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

	if (!localApp || !localMetadata) {
		return (
			<div className="flex items-center justify-center h-full">Loading...</div>
		);
	}

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
									<Button
										variant="outline"
										onClick={resetChanges}
										className="gap-2"
									>
										<RotateCcwIcon className="w-4 h-4" />
										Reset
									</Button>
									<Button onClick={saveChanges} className="gap-2">
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
								value={localMetadata?.name ?? ""}
								disabled={!canEdit}
								onChange={(e) => {
									if (localMetadata && canEdit) {
										setLocalMetadata({
											...localMetadata,
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
								value={localApp?.version ?? ""}
								disabled={!canEdit}
								onChange={(e) => {
									if (localApp && canEdit) {
										setLocalApp({
											...localApp,
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
							value={localMetadata?.description ?? ""}
							disabled={!canEdit}
							onChange={(e) => {
								if (localMetadata && canEdit) {
									setLocalMetadata({
										...localMetadata,
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
									localMetadata?.long_description ||
									"*No detailed description available.*"
								}
							/>
						</div>
					</div>
				</CardContent>
			</Card>

			{/* Visibility Status */}
			{app.data && (
				<VisibilityStatusSwitcher
					canEdit={canEdit}
					localApp={app.data}
					refreshApp={async () => await app.refetch()}
				/>
			)}

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
									canEdit
										? "border-gray-300 dark:border-gray-700 hover:border-primary cursor-pointer"
										: "border-gray-200 dark:border-gray-800 cursor-not-allowed opacity-60"
								}`}
								style={{ aspectRatio: "2/1" }}
								onClick={canEdit ? handleThumbnailUpload : undefined}
							>
								{/* Current thumbnail or placeholder */}
								<div className="absolute inset-0">
									<img
										src={sanitizeImageUrl(
											localMetadata?.thumbnail ?? undefined,
											"/placeholder-thumbnail.webp",
										)}
										alt="App thumbnail"
										className="w-full h-full object-cover"
									/>
									{/* Overlay */}
									{canEdit && (
										<div className="absolute inset-0 bg-black/0 group-hover:bg-black/40 transition-all duration-200 flex items-center justify-center">
											<div className="opacity-0 group-hover:opacity-100 transition-opacity duration-200 flex flex-col items-center gap-2 text-white">
												<ImageIcon className="w-8 h-8" />
												<span className="text-sm font-medium">
													{localMetadata?.thumbnail
														? "Change Thumbnail"
														: "Upload Thumbnail"}
												</span>
											</div>
										</div>
									)}
								</div>
								{/* Upload indicator */}
								{!localMetadata?.thumbnail && (
									<div className="absolute inset-0 flex items-center justify-center">
										<div className="text-center text-gray-500 dark:text-gray-400">
											<ImageIcon className="w-12 h-12 mx-auto mb-2 opacity-50" />
											<p className="text-sm">
												{canEdit ? "Click to upload" : "No thumbnail"}
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
										canEdit
											? "border-gray-300 dark:border-gray-700 hover:border-primary cursor-pointer"
											: "border-gray-200 dark:border-gray-800 cursor-not-allowed opacity-60"
									}`}
									onClick={canEdit ? handleIconUpload : undefined}
								>
									{/* Current icon or placeholder */}
									<div className="absolute inset-0">
										<img
											src={sanitizeImageUrl(
												localMetadata?.icon ?? undefined,
												"/app-logo.webp",
											)}
											alt="App icon"
											className="w-full h-full object-cover rounded-lg"
										/>
										{/* Overlay */}
										{canEdit && (
											<div className="absolute inset-0 bg-black/0 group-hover:bg-black/40 transition-all duration-200 flex items-center justify-center rounded-lg">
												<div className="opacity-0 group-hover:opacity-100 transition-opacity duration-200 flex flex-col items-center gap-1 text-white">
													<ImageIcon className="w-6 h-6" />
													<span className="text-xs font-medium text-center">
														{localMetadata?.icon
															? "Change Icon"
															: "Upload Icon"}
													</span>
												</div>
											</div>
										)}
									</div>
									{/* Upload indicator */}
									{!localMetadata?.icon && (
										<div className="absolute inset-0 flex items-center justify-center">
											<div className="text-center text-gray-500 dark:text-gray-400">
												<ImageIcon className="w-8 h-8 mx-auto mb-1 opacity-50" />
												<p className="text-xs">
													{canEdit ? "Click to upload" : "No icon"}
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
								value={localApp?.primary_category ?? IAppCategory.Other}
								onValueChange={(value) => {
									if (localApp && canEdit) {
										setLocalApp({
											...localApp,
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
											{category}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
						</div>
						<div className="space-y-2">
							<Label htmlFor="secondary-category">Secondary Category</Label>
							<Select
								value={localApp?.secondary_category ?? ""}
								onValueChange={(value) => {
									if (localApp && canEdit) {
										setLocalApp({
											...localApp,
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
											{category}
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
							{localMetadata?.tags && localMetadata.tags.length > 0 && (
								<div className="flex flex-wrap gap-2">
									{localMetadata.tags.map((tag, index) => (
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
								value={localMetadata?.website ?? ""}
								disabled={!canEdit}
								onChange={(e) => {
									if (localMetadata && canEdit) {
										setLocalMetadata({
											...localMetadata,
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
								value={localMetadata?.docs_url ?? ""}
								disabled={!canEdit}
								onChange={(e) => {
									if (localMetadata && canEdit) {
										setLocalMetadata({
											...localMetadata,
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
								value={localMetadata?.support_url ?? ""}
								disabled={!canEdit}
								onChange={(e) => {
									if (localMetadata && canEdit) {
										setLocalMetadata({
											...localMetadata,
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
					<CardDescription>
						Configure application behavior and visibility
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-4">
					<div className="grid grid-cols-1 md:grid-cols-3 gap-4">
						<div className="space-y-2">
							<Label htmlFor="status">Status</Label>
							<Select
								value={localApp?.status ?? IAppStatus.Active}
								onValueChange={(value) => {
									if (localApp && canEdit) {
										setLocalApp({
											...localApp,
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
							<Label htmlFor="visibility">Visibility</Label>
							<Select
								value={localApp?.visibility ?? IAppVisibility.Offline}
								onValueChange={(value) => {
									if (localApp && canEdit) {
										setLocalApp({
											...localApp,
											visibility: value as IAppVisibility,
										});
									}
								}}
								disabled={!canEdit}
							>
								<SelectTrigger>
									<SelectValue placeholder="Select visibility" />
								</SelectTrigger>
								<SelectContent>
									{Object.values(IAppVisibility).map((visibility) => (
										<SelectItem key={visibility} value={visibility}>
											<div className="flex items-center gap-2">
												<EyeIcon className="w-4 h-4" />
												{visibility}
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
								value={localApp?.price ?? ""}
								disabled={!canEdit}
								onChange={(e) => {
									if (localApp && canEdit) {
										setLocalApp({
											...localApp,
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
							value={localApp?.changelog ?? ""}
							disabled={!canEdit}
							onChange={(e) => {
								if (localApp && canEdit) {
									setLocalApp({
										...localApp,
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
								{localApp.download_count}
							</div>
							<div className="text-sm text-gray-600 dark:text-gray-400">
								Downloads
							</div>
						</div>
						<div className="text-center p-4 border rounded-lg">
							<div className="text-2xl font-bold text-green-600">
								{localApp.rating_count}
							</div>
							<div className="text-sm text-gray-600 dark:text-gray-400">
								Ratings
							</div>
						</div>
						<div className="text-center p-4 border rounded-lg">
							<div className="text-2xl font-bold text-purple-600">
								{localApp.interactions_count}
							</div>
							<div className="text-sm text-gray-600 dark:text-gray-400">
								Interactions
							</div>
						</div>
						<div className="text-center p-4 border rounded-lg">
							<div className="text-2xl font-bold text-orange-600">
								{localApp.avg_rating ? localApp.avg_rating.toFixed(1) : "N/A"}
							</div>
							<div className="text-sm text-gray-600 dark:text-gray-400">
								Avg Rating
							</div>
						</div>
					</div>
				</CardContent>
			</Card>

			<AppReviewsSection appId={id ?? ""} onReviewChanged={refreshReviews} />

			{/* Danger Zone */}
			{canEdit && (
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
							onConfirm={async () => {
								await deleteApp();
							}}
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
									if (localMetadata) {
										setLocalMetadata({
											...localMetadata,
											long_description: longDescDraft,
										});
									}
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
