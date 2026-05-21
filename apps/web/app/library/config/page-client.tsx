"use client";
import {
	AppReviewsSection,
	Avatar,
	AvatarFallback,
	AvatarImage,
	Badge,
	Button,
	Card,
	CardContent,
	CardTitle,
	Dialog,
	DialogContent,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	type IApp,
	IAppCategory,
	IAppStatus,
	IAppVisibility,
	type IBoard,
	type IMetadata,
	Input,
	Label,
	ScrollArea,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Separator,
	Skeleton,
	Tabs,
	TabsContent,
	TabsList,
	TabsTrigger,
	TextEditor,
	Textarea,
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
	VerificationDialog,
	formatAppCategory,
	sanitizeImageUrl,
	toastError,
	useBackend,
	useInvalidateInvoke,
	useInvoke,
} from "@flow-like/flow-like-ui";
import { AllowForkingCard } from "@flow-like/flow-like-ui/components/settings/forking/allow-forking-card";
import { ForkAppCard } from "@flow-like/flow-like-ui/components/settings/forking/fork-app-card";
import {
	hashToGradient,
	useThemeInfo,
} from "@flow-like/flow-like-ui/hooks/use-theme-gradient";
import { formatRelativeTime } from "@flow-like/flow-like-ui/lib";
import { isEqual } from "lodash-es";
import {
	AlertTriangleIcon,
	ArrowRightIcon,
	BombIcon,
	CalendarIcon,
	CrownIcon,
	DownloadIcon,
	ExternalLinkIcon,
	EyeIcon,
	GlobeIcon,
	ImageIcon,
	InfoIcon,
	LayoutGridIcon,
	LockIcon,
	PencilIcon,
	RocketIcon,
	RotateCcwIcon,
	SaveIcon,
	SettingsIcon,
	ShieldIcon,
	SparklesIcon,
	StarIcon,
	TagIcon,
	TrendingUpIcon,
	UsersRoundIcon,
	WifiOffIcon,
	WorkflowIcon,
	XIcon,
	ZapIcon,
} from "lucide-react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import { VisibilityStatusSwitcher } from "./visibility-status-switcher";

export default function LibraryConfigPage() {
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const searchParams = useSearchParams();
	const router = useRouter();
	const id = searchParams.get("id");

	const { primaryHue, isDark } = useThemeInfo();
	const bannerGradient = useMemo(
		() => hashToGradient(id ?? "", primaryHue, isDark),
		[id, primaryHue, isDark],
	);
	const [canEdit] = useState(true);
	const [metaDialogOpen, setMetaDialogOpen] = useState(false);
	const [isLongDescEditorOpen, setLongDescEditorOpen] = useState(false);
	const [longDescInit, setLongDescInit] = useState("");
	const [longDescDraft, setLongDescDraft] = useState("");
	const editorAreaRef = useRef<HTMLDivElement | null>(null);

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
	const boards = useInvoke(
		backend.boardState.getBoards,
		backend.boardState,
		[id ?? ""],
		typeof id === "string",
	);
	const events = useInvoke(
		backend.eventState.getEvents,
		backend.eventState,
		[id ?? ""],
		typeof id === "string",
	);
	const pages = useInvoke(
		backend.pageState.getPages,
		backend.pageState,
		[id ?? ""],
		typeof id === "string",
	);

	const [localApp, setLocalApp] = useState<IApp | undefined>();
	const [localMetadata, setLocalMetadata] = useState<IMetadata | undefined>();
	const [hasChanges, setHasChanges] = useState(false);
	const [newTag, setNewTag] = useState("");
	const draftAppIdRef = useRef<string | null>(null);

	useEffect(() => {
		if (!id) {
			draftAppIdRef.current = null;
			setLocalApp(undefined);
			setLocalMetadata(undefined);
			setHasChanges(false);
			setNewTag("");
			return;
		}

		if (draftAppIdRef.current === id) return;

		if (app.isFetching || metadata.isFetching || !app.data || !metadata.data) {
			draftAppIdRef.current = null;
			setLocalApp(undefined);
			setLocalMetadata(undefined);
			setHasChanges(false);
			setNewTag("");
			return;
		}

		draftAppIdRef.current = id;
		setLocalApp(app.data);
		setLocalMetadata(metadata.data);
		setHasChanges(false);
		setNewTag("");
	}, [id, app.data, app.isFetching, metadata.data, metadata.isFetching]);

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

	useEffect(() => {
		if (!isLongDescEditorOpen) return;
		const prev = document.body.style.overflow;
		document.body.style.overflow = "hidden";
		return () => {
			document.body.style.overflow = prev;
		};
	}, [isLongDescEditorOpen]);

	const boardsWithContent = useMemo(() => {
		return (boards.data ?? []).filter((b) => Object.keys(b.nodes).length > 0);
	}, [boards.data]);

	const activeEvents = useMemo(() => {
		return (events.data ?? []).filter((e) => e.active);
	}, [events.data]);

	const isNewProject = useMemo(() => {
		return boardsWithContent.length === 0 && activeEvents.length === 0;
	}, [boardsWithContent, activeEvents]);

	const saveChanges = useCallback(async () => {
		if (!id || !localApp || !localMetadata) {
			toastError("Missing data.", <BombIcon />);
			return;
		}
		await backend.appState.pushAppMeta(id, localMetadata);
		await backend.appState.updateApp(localApp);
		await app.refetch();
		await metadata.refetch();
		await invalidate(backend.appState.getApps, []);
		toast.success("Changes saved!", {
			icon: <SaveIcon className="w-4 h-4" />,
		});
	}, [backend, id, localApp, localMetadata, app, metadata, invalidate]);

	const refreshReviews = useCallback(async () => {
		await app.refetch();
		await invalidate(backend.appState.getApps, []);
	}, [app, backend.appState, invalidate]);

	const resetChanges = useCallback(() => {
		if (!app.data || !metadata.data) return;
		setLocalApp(app.data);
		setLocalMetadata(metadata.data);
		toast("Changes reset.", { icon: <RotateCcwIcon className="w-4 h-4" /> });
	}, [app.data, metadata.data]);

	const handleMediaUpload = useCallback(
		async (type: "thumbnail" | "icon") => {
			if (!id || !canEdit) return;
			const input = document.createElement("input");
			input.type = "file";
			input.accept = "image/jpeg,image/jpg,image/png,image/webp";
			input.onchange = async (event) => {
				const file = (event.target as HTMLInputElement).files?.[0];
				if (!file) return;
				const validTypes = [
					"image/jpeg",
					"image/jpg",
					"image/png",
					"image/webp",
				];
				if (!validTypes.includes(file.type)) {
					toastError("Invalid image type.", <BombIcon />);
					return;
				}
				const maxSize = type === "thumbnail" ? 30 : 20;
				if (file.size > maxSize * 1024 * 1024) {
					toastError(`File too large (max ${maxSize}MB).`, <BombIcon />);
					return;
				}
				const loadingRef = toast.loading(`Uploading ${type}...`);
				await backend.appState.pushAppMedia(id, type, file);
				if (localMetadata) {
					setLocalMetadata({
						...localMetadata,
						[type]: URL.createObjectURL(file),
					});
				}
				toast.dismiss(loadingRef);
				toast.success(
					`${type === "thumbnail" ? "Thumbnail" : "Icon"} uploaded!`,
				);
				await metadata.refetch();
			};
			input.click();
		},
		[id, canEdit, backend.appState, localMetadata, metadata],
	);

	const addTag = useCallback(
		(tag: string) => {
			if (!localMetadata || !canEdit || !tag.trim()) return;
			const trimmed = tag.trim();
			if (localMetadata.tags?.includes(trimmed)) return;
			setLocalMetadata({
				...localMetadata,
				tags: [...(localMetadata.tags || []), trimmed],
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
				tags: localMetadata.tags?.filter((t) => t !== tagToRemove) || [],
			});
		},
		[localMetadata, canEdit],
	);

	async function deleteApp() {
		await backend.appState.deleteApp(id ?? "");
		await invalidate(backend.appState.getApps, []);
		router.push("/library");
	}

	if (!app.data || !metadata.data || !localApp || !localMetadata) {
		return (
			<div className="flex items-center justify-center h-full">
				<div className="space-y-3 w-full max-w-xl">
					<Skeleton className="h-24 w-full rounded-xl" />
					<div className="grid grid-cols-4 gap-3">
						{[...Array(4)].map((_, i) => (
							<Skeleton key={`stat-skel-${i}`} className="h-20 rounded-lg" />
						))}
					</div>
					<Skeleton className="h-48 w-full rounded-xl" />
				</div>
			</div>
		);
	}

	return (
		<TooltipProvider>
			<div className="w-full max-w-5xl mx-auto px-2 md:px-4 pb-2 md:pb-4 -mt-6 space-y-5 flex flex-col grow max-h-full min-h-0 overflow-auto md:overflow-visible">
				{hasChanges && canEdit && (
					<div className="sticky top-0 z-10">
						<Card className="border-orange-200 bg-orange-50 dark:border-orange-800 dark:bg-orange-950">
							<CardContent className="py-3">
								<div className="flex items-center justify-between">
									<div className="flex items-center gap-2">
										<InfoIcon className="w-4 h-4 text-orange-600" />
										<span className="text-sm font-medium text-orange-800 dark:text-orange-200">
											Unsaved changes
										</span>
									</div>
									<div className="flex gap-2">
										<Button variant="outline" size="sm" onClick={resetChanges}>
											<RotateCcwIcon className="w-3 h-3 mr-1" />
											Reset
										</Button>
										<Button size="sm" onClick={saveChanges}>
											<SaveIcon className="w-3 h-3 mr-1" />
											Save
										</Button>
									</div>
								</div>
							</CardContent>
						</Card>
					</div>
				)}

				{/* Banner */}
				<div className="-mx-8 md:-mx-10">
					<button
						type="button"
						className="h-32 w-full relative group cursor-pointer border-0 p-0"
						onClick={() => handleMediaUpload("thumbnail")}
					>
						{metadata.data.thumbnail ? (
							<>
								<img
									src={sanitizeImageUrl(
										metadata.data.thumbnail,
										"/placeholder-thumbnail.webp",
									)}
									alt=""
									className="w-full h-full object-cover absolute inset-0"
								/>
								<div className="absolute inset-0 bg-linear-to-t from-background/80 to-transparent" />
							</>
						) : (
							<>
								<div
									className="absolute inset-0"
									style={{
										background: `linear-gradient(${bannerGradient.angle}deg, ${bannerGradient.from}, ${bannerGradient.to})`,
										opacity: bannerGradient.opacity,
									}}
								/>
								<div className="absolute inset-0 bg-linear-to-r from-transparent to-card/80" />
							</>
						)}
						{canEdit && (
							<div className="absolute inset-0 bg-black/0 group-hover:bg-black/30 transition-all flex items-center justify-center">
								<div className="opacity-0 group-hover:opacity-100 transition-opacity text-white text-xs flex items-center gap-1">
									<ImageIcon className="w-3 h-3" />
									Change Banner
								</div>
							</div>
						)}
					</button>
				</div>

				{/* App Identity */}
				<div className="-mt-14 relative z-10 px-3">
					<div className="flex items-end gap-4">
						<button
							type="button"
							className="relative group cursor-pointer shrink-0 border-0 p-0 bg-transparent"
							onClick={() => handleMediaUpload("icon")}
						>
							<Avatar className="w-16 h-16 border-2 border-border shadow-md">
								<AvatarImage
									src={sanitizeImageUrl(
										metadata.data.icon ?? undefined,
										"/app-logo.webp",
									)}
									alt={metadata.data.name}
								/>
								<AvatarFallback className="text-lg font-bold">
									{metadata.data.name.substring(0, 2).toUpperCase()}
								</AvatarFallback>
							</Avatar>
							{canEdit && (
								<div className="absolute inset-0 rounded-full bg-black/0 group-hover:bg-black/40 transition-all flex items-center justify-center">
									<PencilIcon className="w-4 h-4 text-white opacity-0 group-hover:opacity-100 transition-opacity" />
								</div>
							)}
						</button>
						<div className="flex-1 min-w-0 pb-1">
							<div className="flex items-center gap-2 flex-wrap">
								<h1 className="text-xl font-bold truncate">
									{metadata.data.name}
								</h1>
								<VisibilityBadge visibility={app.data.visibility} />
								{app.data.version && (
									<Badge variant="outline" className="text-xs">
										v{app.data.version}
									</Badge>
								)}
							</div>
							{metadata.data.description && (
								<p className="text-sm text-muted-foreground truncate mt-0.5">
									{metadata.data.description}
								</p>
							)}
						</div>
						{canEdit && (
							<Button
								variant="outline"
								size="sm"
								className="shrink-0"
								onClick={() => setMetaDialogOpen(true)}
							>
								<PencilIcon className="w-3 h-3 mr-1.5" />
								Edit
							</Button>
						)}
					</div>
				</div>

				{/* Quick Stats */}
				<div className="grid grid-cols-2 md:grid-cols-4 gap-3">
					<StatCard
						label="Downloads"
						value={app.data.download_count}
						icon={<DownloadIcon className="w-4 h-4" />}
						color="text-blue-600"
					/>
					<StatCard
						label="Interactions"
						value={app.data.interactions_count}
						icon={<ZapIcon className="w-4 h-4" />}
						color="text-purple-600"
					/>
					<StatCard
						label="Ratings"
						value={app.data.rating_count}
						icon={<StarIcon className="w-4 h-4" />}
						color="text-green-600"
					/>
					<StatCard
						label="Avg Rating"
						value={
							app.data.avg_rating ? app.data.avg_rating.toFixed(1) : "\u2014"
						}
						icon={<TrendingUpIcon className="w-4 h-4" />}
						color="text-orange-600"
					/>
				</div>

				{/* Getting Started for new projects */}
				{isNewProject && (
					<Card className="border-primary/20 bg-linear-to-br from-primary/5 via-background to-purple-500/5">
						<CardContent className="py-6">
							<div className="flex flex-col items-center text-center gap-4">
								<div className="w-12 h-12 rounded-full bg-primary/10 flex items-center justify-center">
									<RocketIcon className="w-6 h-6 text-primary" />
								</div>
								<div>
									<h3 className="text-lg font-semibold">
										Let&apos;s get started!
									</h3>
									<p className="text-sm text-muted-foreground mt-1 max-w-md">
										Your app doesn&apos;t have any logic yet. Create your first
										flow to define what your app does, or set up events to
										handle interactions.
									</p>
								</div>
								<div className="flex flex-wrap gap-2 justify-center">
									<Link href={`/library/config/flows?id=${id}`}>
										<Button className="gap-2">
											<WorkflowIcon className="w-4 h-4" />
											Create a Flow
										</Button>
									</Link>
									<Link href={`/library/config/pages?id=${id}`}>
										<Button variant="outline" className="gap-2">
											<SparklesIcon className="w-4 h-4" />
											Set Up Events
										</Button>
									</Link>
								</div>
							</div>
						</CardContent>
					</Card>
				)}

				{/* Main Content Tabs */}
				<Tabs defaultValue="overview" className="space-y-4">
					<TabsList className="w-fit">
						<TabsTrigger value="overview" className="gap-1.5">
							<LayoutGridIcon className="w-3.5 h-3.5" />
							Overview
						</TabsTrigger>
						<TabsTrigger value="details" className="gap-1.5">
							<SettingsIcon className="w-3.5 h-3.5" />
							Details
						</TabsTrigger>
					</TabsList>

					{/* Overview Tab */}
					<TabsContent value="overview" className="space-y-4">
						{/* Flows */}
						<Card>
							<CardContent className="pt-5">
								<div className="flex items-center justify-between mb-3">
									<div className="flex items-center gap-2">
										<WorkflowIcon className="w-4 h-4 text-muted-foreground" />
										<CardTitle className="text-base">Flows</CardTitle>
										<Badge variant="secondary" className="text-xs">
											{boards.data?.length ?? 0}
										</Badge>
									</div>
									<Link href={`/library/config/flows?id=${id}`}>
										<Button variant="ghost" size="sm" className="gap-1 text-xs">
											View all
											<ArrowRightIcon className="w-3 h-3" />
										</Button>
									</Link>
								</div>
								{(boards.data?.length ?? 0) === 0 ? (
									<div className="text-center py-6 text-sm text-muted-foreground">
										No flows yet.{" "}
										<Link
											href={`/library/config/flows?id=${id}`}
											className="text-primary hover:underline"
										>
											Create your first flow
										</Link>
									</div>
								) : (
									<div className="space-y-1">
										{boards.data?.slice(0, 5).map((board) => (
											<BoardRow key={board.id} board={board} appId={id!} />
										))}
										{(boards.data?.length ?? 0) > 5 && (
											<p className="text-xs text-muted-foreground text-center pt-2">
												+{(boards.data?.length ?? 0) - 5} more flows
											</p>
										)}
									</div>
								)}
							</CardContent>
						</Card>

						{/* Events & Pages side by side */}
						<div className="grid grid-cols-1 md:grid-cols-2 gap-4">
							<Card>
								<CardContent className="pt-5">
									<div className="flex items-center justify-between mb-3">
										<div className="flex items-center gap-2">
											<SparklesIcon className="w-4 h-4 text-muted-foreground" />
											<CardTitle className="text-base">Events</CardTitle>
											<Badge variant="secondary" className="text-xs">
												{events.data?.length ?? 0}
											</Badge>
										</div>
										<Link href={`/library/config/pages?id=${id}`}>
											<Button
												variant="ghost"
												size="sm"
												className="gap-1 text-xs"
											>
												Manage
												<ArrowRightIcon className="w-3 h-3" />
											</Button>
										</Link>
									</div>
									{(events.data?.length ?? 0) === 0 ? (
										<div className="text-center py-4 text-sm text-muted-foreground">
											No events configured
										</div>
									) : (
										<div className="space-y-1">
											{events.data?.slice(0, 4).map((event) => (
												<div
													key={event.id}
													className="flex items-center justify-between py-1.5 px-2 rounded-md hover:bg-muted/50 text-sm"
												>
													<div className="flex items-center gap-2 min-w-0">
														<div
															className={`w-2 h-2 rounded-full shrink-0 ${event.active ? "bg-green-500" : "bg-gray-400"}`}
														/>
														<span className="truncate">{event.name}</span>
													</div>
													<Badge
														variant="outline"
														className="text-xs shrink-0 ml-2"
													>
														{event.event_type}
													</Badge>
												</div>
											))}
											{(events.data?.length ?? 0) > 4 && (
												<p className="text-xs text-muted-foreground text-center pt-1">
													+{(events.data?.length ?? 0) - 4} more
												</p>
											)}
										</div>
									)}
								</CardContent>
							</Card>

							<Card>
								<CardContent className="pt-5">
									<div className="flex items-center justify-between mb-3">
										<div className="flex items-center gap-2">
											<LayoutGridIcon className="w-4 h-4 text-muted-foreground" />
											<CardTitle className="text-base">Pages</CardTitle>
											<Badge variant="secondary" className="text-xs">
												{pages.data?.length ?? 0}
											</Badge>
										</div>
										<Link href={`/library/config/pages?id=${id}`}>
											<Button
												variant="ghost"
												size="sm"
												className="gap-1 text-xs"
											>
												Manage
												<ArrowRightIcon className="w-3 h-3" />
											</Button>
										</Link>
									</div>
									{(pages.data?.length ?? 0) === 0 ? (
										<div className="text-center py-4 text-sm text-muted-foreground">
											No pages created yet
										</div>
									) : (
										<div className="space-y-1">
											{pages.data?.slice(0, 4).map((page) => (
												<div
													key={page.pageId}
													className="flex items-center gap-2 py-1.5 px-2 rounded-md hover:bg-muted/50 text-sm"
												>
													<LayoutGridIcon className="w-3 h-3 text-muted-foreground shrink-0" />
													<span className="truncate">{page.name}</span>
												</div>
											))}
											{(pages.data?.length ?? 0) > 4 && (
												<p className="text-xs text-muted-foreground text-center pt-1">
													+{(pages.data?.length ?? 0) - 4} more
												</p>
											)}
										</div>
									)}
								</CardContent>
							</Card>
						</div>

						{/* Team & Roles */}
						<TeamRolesSection appId={id!} visibility={app.data.visibility} />
					</TabsContent>

					{/* Details Tab */}
					<TabsContent value="details" className="space-y-4">
						<VisibilityStatusSwitcher
							canEdit={canEdit}
							localApp={app.data}
							refreshApp={async () => {
								await app.refetch();
							}}
						/>

						<AllowForkingCard
							canEdit={canEdit}
							localApp={app.data}
							onChanged={async () => {
								await app.refetch();
							}}
						/>

						<ForkAppCard
							appId={app.data.id}
							appName={metadata.data?.name ?? id ?? "this app"}
							target="online"
						/>

						{app.data.visibility === IAppVisibility.Private && (
							<Card className="border-blue-200 dark:border-blue-800 bg-blue-50/50 dark:bg-blue-950/30">
								<CardContent className="py-3">
									<div className="flex items-start gap-3">
										<InfoIcon className="w-4 h-4 text-blue-600 mt-0.5 shrink-0" />
										<div>
											<p className="text-sm font-medium text-blue-800 dark:text-blue-200">
												Ready to share?
											</p>
											<p className="text-xs text-blue-700 dark:text-blue-300 mt-0.5">
												Change visibility to Prototype or Public to make your
												app available to others and enable team collaboration.
											</p>
										</div>
									</div>
								</CardContent>
							</Card>
						)}

						{/* Categories & Tags */}
						<Card>
							<CardContent className="pt-5 space-y-4">
								<CardTitle className="flex items-center gap-2 text-base">
									<TagIcon className="w-4 h-4" />
									Categories & Tags
								</CardTitle>
								<div className="grid grid-cols-1 md:grid-cols-2 gap-4">
									<div className="space-y-2">
										<Label className="text-sm">Primary Category</Label>
										<Select
											value={localApp?.primary_category ?? IAppCategory.Other}
											onValueChange={(value) => {
												if (localApp && canEdit)
													setLocalApp({
														...localApp,
														primary_category: value as IAppCategory,
													});
											}}
											disabled={!canEdit}
										>
											<SelectTrigger>
												<SelectValue placeholder="Select category" />
											</SelectTrigger>
											<SelectContent>
												{Object.values(IAppCategory).map((cat) => (
													<SelectItem key={cat} value={cat}>
														{formatAppCategory(cat)}
													</SelectItem>
												))}
											</SelectContent>
										</Select>
									</div>
									<div className="space-y-2">
										<Label className="text-sm">Secondary Category</Label>
										<Select
											value={localApp?.secondary_category ?? ""}
											onValueChange={(value) => {
												if (localApp && canEdit)
													setLocalApp({
														...localApp,
														secondary_category:
															value === "none" ? null : (value as IAppCategory),
													});
											}}
											disabled={!canEdit}
										>
											<SelectTrigger>
												<SelectValue placeholder="None" />
											</SelectTrigger>
											<SelectContent>
												<SelectItem value="none">None</SelectItem>
												{Object.values(IAppCategory).map((cat) => (
													<SelectItem key={cat} value={cat}>
														{formatAppCategory(cat)}
													</SelectItem>
												))}
											</SelectContent>
										</Select>
									</div>
								</div>
								<div className="space-y-2">
									<Label className="text-sm">Tags</Label>
									<Input
										placeholder="Type a tag and press Enter..."
										value={newTag}
										disabled={!canEdit}
										onChange={(e) => setNewTag(e.target.value)}
										onKeyDown={(e) => {
											if (e.key === "Enter") {
												e.preventDefault();
												addTag(newTag);
											}
										}}
									/>
									{localMetadata?.tags && localMetadata.tags.length > 0 && (
										<div className="flex flex-wrap gap-1.5 pt-1">
											{localMetadata.tags.map((tag) => (
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
															className="ml-0.5 hover:text-red-500"
														>
															<XIcon className="w-3 h-3" />
														</button>
													)}
												</Badge>
											))}
										</div>
									)}
								</div>
							</CardContent>
						</Card>

						{/* Support & Links */}
						<Card>
							<CardContent className="pt-5 space-y-3">
								<CardTitle className="flex items-center gap-2 text-base">
									<ExternalLinkIcon className="w-4 h-4" />
									Support & Links
								</CardTitle>
								<div className="space-y-2">
									<Label className="text-sm">Website</Label>
									<Input
										placeholder="https://yourapp.com"
										value={localMetadata?.website ?? ""}
										disabled={!canEdit}
										onChange={(e) => {
											if (localMetadata && canEdit)
												setLocalMetadata({
													...localMetadata,
													website: e.target.value,
												});
										}}
									/>
								</div>
								<div className="space-y-2">
									<Label className="text-sm">Documentation</Label>
									<Input
										placeholder="https://docs.yourapp.com"
										value={localMetadata?.docs_url ?? ""}
										disabled={!canEdit}
										onChange={(e) => {
											if (localMetadata && canEdit)
												setLocalMetadata({
													...localMetadata,
													docs_url: e.target.value,
												});
										}}
									/>
								</div>
								<div className="space-y-2">
									<Label className="text-sm">Support</Label>
									<Input
										placeholder="https://support.yourapp.com"
										value={localMetadata?.support_url ?? ""}
										disabled={!canEdit}
										onChange={(e) => {
											if (localMetadata && canEdit)
												setLocalMetadata({
													...localMetadata,
													support_url: e.target.value,
												});
										}}
									/>
								</div>
							</CardContent>
						</Card>

						{/* Application Settings */}
						<Card>
							<CardContent className="pt-5 space-y-4">
								<CardTitle className="flex items-center gap-2 text-base">
									<SettingsIcon className="w-4 h-4" />
									Application Settings
								</CardTitle>
								<div className="grid grid-cols-1 md:grid-cols-3 gap-4">
									<div className="space-y-2">
										<Label className="text-sm">Status</Label>
										<Select
											value={localApp?.status ?? IAppStatus.Active}
											onValueChange={(value) => {
												if (localApp && canEdit)
													setLocalApp({
														...localApp,
														status: value as IAppStatus,
													});
											}}
											disabled={!canEdit}
										>
											<SelectTrigger>
												<SelectValue />
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
										<Label className="text-sm">Version</Label>
										<Input
											placeholder="1.0.0"
											value={localApp?.version ?? ""}
											disabled={!canEdit}
											onChange={(e) => {
												if (localApp && canEdit)
													setLocalApp({
														...localApp,
														version: e.target.value,
													});
											}}
										/>
									</div>
									<div className="space-y-2">
										<Label className="text-sm">Price ($)</Label>
										<Input
											type="number"
											placeholder="0.00"
											value={localApp?.price ?? ""}
											disabled={!canEdit}
											onChange={(e) => {
												if (localApp && canEdit)
													setLocalApp({
														...localApp,
														price: Number.parseFloat(e.target.value) || null,
													});
											}}
										/>
									</div>
								</div>
							</CardContent>
						</Card>

						{/* Changelog */}
						<Card>
							<CardContent className="pt-5 space-y-3">
								<CardTitle className="flex items-center gap-2 text-base">
									<CalendarIcon className="w-4 h-4" />
									Changelog
								</CardTitle>
								<Textarea
									placeholder="What is new in this version..."
									rows={3}
									value={localApp?.changelog ?? ""}
									disabled={!canEdit}
									onChange={(e) => {
										if (localApp && canEdit)
											setLocalApp({ ...localApp, changelog: e.target.value });
									}}
								/>
							</CardContent>
						</Card>

						<AppReviewsSection
							appId={id ?? ""}
							onReviewChanged={refreshReviews}
						/>

						{/* Danger Zone */}
						{canEdit && (
							<Card className="border-red-200 dark:border-red-800">
								<CardContent className="pt-5 space-y-3">
									<CardTitle className="flex items-center gap-2 text-base text-red-600 dark:text-red-400">
										<ShieldIcon className="w-4 h-4" />
										Danger Zone
									</CardTitle>
									<VerificationDialog
										dialog="You cannot undo this action. This will permanently delete the app!"
										onConfirm={async () => {
											await deleteApp();
										}}
									>
										<Button variant="destructive" size="sm" className="gap-2">
											<BombIcon className="w-3 h-3" />
											Delete App
										</Button>
									</VerificationDialog>
								</CardContent>
							</Card>
						)}
					</TabsContent>
				</Tabs>

				{/* Metadata Edit Dialog */}
				<Dialog open={metaDialogOpen} onOpenChange={setMetaDialogOpen}>
					<DialogContent className="sm:max-w-[600px] max-h-[85vh] flex flex-col">
						<DialogHeader>
							<DialogTitle>Edit App Details</DialogTitle>
						</DialogHeader>
						<ScrollArea className="flex-1 pr-4">
							<div className="space-y-4 pb-2">
								<div className="space-y-2">
									<Label>Name</Label>
									<Input
										placeholder="Application name"
										value={localMetadata?.name ?? ""}
										onChange={(e) => {
											if (localMetadata)
												setLocalMetadata({
													...localMetadata,
													name: e.target.value,
												});
										}}
									/>
								</div>
								<div className="space-y-2">
									<Label>Version</Label>
									<Input
										placeholder="1.0.0"
										value={localApp?.version ?? ""}
										onChange={(e) => {
											if (localApp)
												setLocalApp({
													...localApp,
													version: e.target.value,
												});
										}}
									/>
								</div>
								<div className="space-y-2">
									<Label>Short Description</Label>
									<Textarea
										placeholder="Brief description in 1-2 sentences..."
										rows={2}
										value={localMetadata?.description ?? ""}
										onChange={(e) => {
											if (localMetadata)
												setLocalMetadata({
													...localMetadata,
													description: e.target.value,
												});
										}}
									/>
								</div>
								<div className="space-y-2">
									<div className="flex items-center justify-between">
										<Label>Long Description</Label>
										<Button
											variant="outline"
											size="sm"
											onClick={() => {
												const initial = localMetadata?.long_description || "";
												setLongDescInit(initial);
												setLongDescDraft(initial);
												setLongDescEditorOpen(true);
											}}
										>
											Open Markdown Editor
										</Button>
									</div>
									<div className="rounded-md border p-3 text-sm text-muted-foreground min-h-[60px]">
										{localMetadata?.long_description ? (
											<span className="line-clamp-3">
												{localMetadata.long_description.substring(0, 200)}
												{localMetadata.long_description.length > 200
													? "..."
													: ""}
											</span>
										) : (
											<span className="italic">No long description</span>
										)}
									</div>
								</div>
								<Separator />
								<div className="space-y-3">
									<Label className="text-sm font-medium">Visual Assets</Label>
									<div className="grid grid-cols-2 gap-4">
										<button
											type="button"
											className="border-2 border-dashed rounded-lg p-3 cursor-pointer hover:border-primary transition-colors text-center bg-transparent"
											onClick={() => handleMediaUpload("thumbnail")}
										>
											<ImageIcon className="w-6 h-6 mx-auto text-muted-foreground mb-1" />
											<p className="text-xs text-muted-foreground">
												{metadata.data?.thumbnail
													? "Change Thumbnail"
													: "Upload Thumbnail"}
											</p>
										</button>
										<button
											type="button"
											className="border-2 border-dashed rounded-lg p-3 cursor-pointer hover:border-primary transition-colors text-center bg-transparent"
											onClick={() => handleMediaUpload("icon")}
										>
											<ImageIcon className="w-6 h-6 mx-auto text-muted-foreground mb-1" />
											<p className="text-xs text-muted-foreground">
												{metadata.data?.icon ? "Change Icon" : "Upload Icon"}
											</p>
										</button>
									</div>
								</div>
							</div>
						</ScrollArea>
						<DialogFooter>
							<Button
								variant="outline"
								onClick={() => setMetaDialogOpen(false)}
							>
								Close
							</Button>
							{hasChanges && (
								<Button onClick={saveChanges}>
									<SaveIcon className="w-3 h-3 mr-1.5" />
									Save
								</Button>
							)}
						</DialogFooter>
					</DialogContent>
				</Dialog>

				{/* Long Description Editor */}
				<Dialog
					open={isLongDescEditorOpen}
					onOpenChange={setLongDescEditorOpen}
				>
					<DialogContent
						className="w-dvw min-w-dvw max-w-dvw min-h-svh max-h-svh flex flex-col"
						onEscapeKeyDown={(e) => {
							const target = e.target as Node | null;
							if (target && editorAreaRef.current?.contains(target))
								e.preventDefault();
						}}
					>
						<div className="flex items-center justify-between px-6 py-2 h-20 border-b bg-background">
							<div>
								<div className="text-lg font-semibold">
									Edit Long Description
								</div>
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
										if (localMetadata)
											setLocalMetadata({
												...localMetadata,
												long_description: longDescDraft,
											});
										setLongDescEditorOpen(false);
									}}
								>
									Done
								</Button>
							</div>
						</div>
						<div className="grow overflow-hidden relative">
							<div className="h-full overflow-auto p-6">
								<div
									ref={editorAreaRef}
									onKeyDown={(e) => {
										if (e.key === "Escape") e.stopPropagation();
									}}
								>
									<TextEditor
										editable={canEdit}
										isMarkdown
										initialContent={
											longDescInit || "*No detailed description available.*"
										}
										onChange={(content) => setLongDescDraft(content)}
									/>
								</div>
							</div>
						</div>
					</DialogContent>
				</Dialog>
			</div>
		</TooltipProvider>
	);
}

function StatCard({
	label,
	value,
	icon,
	color,
}: Readonly<{
	label: string;
	value: string | number;
	icon: React.ReactNode;
	color: string;
}>) {
	return (
		<Card className="hover:shadow-sm transition-shadow">
			<CardContent className="p-4">
				<div className={`${color} opacity-70`}>{icon}</div>
				<div className={`text-2xl font-bold mt-2 ${color}`}>{value}</div>
				<div className="text-xs text-muted-foreground mt-0.5">{label}</div>
			</CardContent>
		</Card>
	);
}

function VisibilityBadge({
	visibility,
}: Readonly<{ visibility: IAppVisibility }>) {
	const config: Record<
		IAppVisibility,
		{
			label: string;
			variant: "default" | "secondary" | "outline" | "destructive";
			icon: React.ReactNode;
		}
	> = {
		[IAppVisibility.Offline]: {
			label: "Offline",
			variant: "secondary",
			icon: <WifiOffIcon className="w-3 h-3" />,
		},
		[IAppVisibility.Private]: {
			label: "Private",
			variant: "outline",
			icon: <LockIcon className="w-3 h-3" />,
		},
		[IAppVisibility.Prototype]: {
			label: "Prototype",
			variant: "outline",
			icon: <EyeIcon className="w-3 h-3" />,
		},
		[IAppVisibility.Public]: {
			label: "Public",
			variant: "default",
			icon: <GlobeIcon className="w-3 h-3" />,
		},
		[IAppVisibility.PublicRequestAccess]: {
			label: "Request Access",
			variant: "outline",
			icon: <LockIcon className="w-3 h-3" />,
		},
	};
	const c = config[visibility];
	return (
		<Badge variant={c.variant} className="gap-1 text-xs">
			{c.icon}
			{c.label}
		</Badge>
	);
}

function BoardRow({
	board,
	appId,
}: Readonly<{ board: IBoard; appId: string }>) {
	const nodeCount = Object.keys(board.nodes).length;
	const updatedAt = board.updated_at
		? formatRelativeTime(board.updated_at)
		: null;

	return (
		<Link
			href={`/flow?id=${board.id}&app=${appId}`}
			className="flex items-center justify-between py-2 px-3 rounded-lg hover:bg-muted/50 transition-colors group"
		>
			<div className="flex items-center gap-3 min-w-0">
				<WorkflowIcon className="w-4 h-4 text-muted-foreground shrink-0" />
				<div className="min-w-0">
					<p className="text-sm font-medium truncate">{board.name}</p>
					{board.description && (
						<p className="text-xs text-muted-foreground truncate">
							{board.description}
						</p>
					)}
				</div>
			</div>
			<div className="flex items-center gap-3 shrink-0 ml-2">
				<span className="text-xs text-muted-foreground">
					{nodeCount} node{nodeCount !== 1 ? "s" : ""}
				</span>
				{updatedAt && (
					<span className="text-xs text-muted-foreground hidden md:block">
						{updatedAt}
					</span>
				)}
				<ArrowRightIcon className="w-3 h-3 text-muted-foreground opacity-0 group-hover:opacity-100 transition-opacity" />
			</div>
		</Link>
	);
}

function TeamRolesSection({
	appId,
	visibility,
}: Readonly<{ appId: string; visibility: IAppVisibility }>) {
	const isOnline = [
		IAppVisibility.Public,
		IAppVisibility.Prototype,
		IAppVisibility.PublicRequestAccess,
	].includes(visibility);

	return (
		<div className="grid grid-cols-1 md:grid-cols-2 gap-4">
			<TeamRoleCard
				title="Team"
				description="Manage members and permissions"
				icon={<UsersRoundIcon className="w-4 h-4" />}
				href={`/library/config/team?id=${appId}`}
				locked={!isOnline}
				visibility={visibility}
			/>
			<TeamRoleCard
				title="Roles"
				description="Define access levels"
				icon={<CrownIcon className="w-4 h-4" />}
				href={`/library/config/roles?id=${appId}`}
				locked={!isOnline}
				visibility={visibility}
			/>
		</div>
	);
}

function TeamRoleCard({
	title,
	description,
	icon,
	href,
	locked,
	visibility,
}: Readonly<{
	title: string;
	description: string;
	icon: React.ReactNode;
	href: string;
	locked: boolean;
	visibility: IAppVisibility;
}>) {
	if (locked) {
		return (
			<Tooltip>
				<TooltipTrigger asChild>
					<Card className="opacity-50 cursor-not-allowed">
						<CardContent className="py-4">
							<div className="flex items-center gap-3">
								<div className="w-8 h-8 rounded-lg bg-muted flex items-center justify-center text-muted-foreground">
									{icon}
								</div>
								<div className="flex-1 min-w-0">
									<div className="flex items-center gap-2">
										<p className="text-sm font-medium">{title}</p>
										<LockIcon className="w-3 h-3 text-muted-foreground" />
									</div>
									<p className="text-xs text-muted-foreground">{description}</p>
								</div>
							</div>
						</CardContent>
					</Card>
				</TooltipTrigger>
				<TooltipContent side="bottom" className="max-w-xs">
					<div className="space-y-1">
						<div className="flex items-center gap-1.5 font-medium">
							<AlertTriangleIcon className="w-3.5 h-3.5 text-amber-500" />
							{visibility === IAppVisibility.Offline
								? "Offline Project"
								: "Private Project"}
						</div>
						<p className="text-xs">
							{visibility === IAppVisibility.Offline
								? "Team management is not available for offline projects. Change your project visibility to enable collaboration."
								: "Change your project visibility to Prototype or Public to enable team features."}
						</p>
						<p className="text-xs text-muted-foreground">
							Go to Details tab, then Visibility to change this.
						</p>
					</div>
				</TooltipContent>
			</Tooltip>
		);
	}

	return (
		<Link href={href}>
			<Card className="hover:shadow-sm hover:border-primary/20 transition-all cursor-pointer group">
				<CardContent className="py-4">
					<div className="flex items-center gap-3">
						<div className="w-8 h-8 rounded-lg bg-primary/10 flex items-center justify-center text-primary">
							{icon}
						</div>
						<div className="flex-1 min-w-0">
							<p className="text-sm font-medium">{title}</p>
							<p className="text-xs text-muted-foreground">{description}</p>
						</div>
						<ArrowRightIcon className="w-4 h-4 text-muted-foreground opacity-0 group-hover:opacity-100 transition-opacity" />
					</div>
				</CardContent>
			</Card>
		</Link>
	);
}
