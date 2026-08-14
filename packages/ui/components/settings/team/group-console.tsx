"use client";

import { useTranslation } from "@flow-like/locales";
import {
	AlertTriangle,
	Archive,
	CheckCircle2,
	Clock,
	ImageIcon,
	Info,
	LogOut,
	Plus,
	Search,
	ShieldAlert,
	Trash2,
	Upload,
	X,
} from "lucide-react";
import type { ReactNode } from "react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { toast } from "sonner";
import type {
	IApp,
	IGroup,
	IGroupPublicationRequest,
	IMediaItem,
	IMemberReadiness,
	IMetadata,
} from "../../..";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
	AlertDialogTrigger,
	Avatar,
	AvatarFallback,
	AvatarImage,
	Badge,
	Button,
	IAppVisibility,
	Input,
	Label,
	ScrollArea,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Sheet,
	SheetContent,
	SheetDescription,
	SheetHeader,
	SheetTitle,
	Tabs,
	TabsContent,
	TabsList,
	TabsTrigger,
	Textarea,
	formatRelativeTime,
	initials,
	seedGradient,
	useBackend,
	useInvalidateInvoke,
	useInvoke,
	useSearch,
} from "../../..";
import {
	VISIBILITY_META,
	fromWireVisibility,
	getVisibilityTransitions,
} from "../visibility-status/visibility-meta";
import { EntityVisibilitySwitcher } from "../visibility-status/visibility-status-switcher";

const IMAGE_TYPES = ["image/jpeg", "image/jpg", "image/png", "image/webp"];
const MAX_IMAGE_MB = 20;

export interface GroupConsoleProps {
	/** The app whose settings the console is opened from. */
	appId: string;
	group: IGroup;
	open: boolean;
	onOpenChange: (open: boolean) => void;
	onChange: () => void;
	suggestions: { id: string; name: string }[];
}

export function GroupConsole({
	appId,
	group,
	open,
	onOpenChange,
	onChange,
	suggestions,
}: Readonly<GroupConsoleProps>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();

	const detail = useInvoke(
		backend.teamState.getGroup,
		backend.teamState,
		[appId, group.id],
		open,
	);
	const current = detail.data ?? group;
	const isAnchor = current.owner_app_id === appId;
	const visibility = fromWireVisibility(current.visibility);

	const refresh = useCallback(async () => {
		await invalidate(backend.teamState.getGroup, [appId, group.id]);
		onChange();
	}, [invalidate, backend.teamState.getGroup, appId, group.id, onChange]);

	return (
		<Sheet open={open} onOpenChange={onOpenChange}>
			<SheetContent
				side="right"
				className="w-full sm:max-w-3xl p-0 flex flex-col gap-0"
			>
				<SheetHeader className="border-b">
					<div className="flex items-start gap-3">
						<Avatar className="h-12 w-12 rounded-xl">
							{current.icon ? <AvatarImage src={current.icon} alt="" /> : null}
							<AvatarFallback
								className="rounded-xl text-white font-bold"
								style={{ backgroundImage: seedGradient(current.id) }}
							>
								{initials(current.name)}
							</AvatarFallback>
						</Avatar>
						<div className="min-w-0 flex-1">
							<SheetTitle className="truncate">
								{current.use_case || current.name || "Untitled suite"}
							</SheetTitle>
							<SheetDescription>
								{t('aSuiteIsAVisualCollectionOnlyItNeverGrantsRuntimePermissionsAndEveryMemberAppCanLeaveAtAnyTime', "A suite is a visual collection only — it never grants runtime permissions, and every member app can leave at any time.")}
							</SheetDescription>
						</div>
						<Badge variant="secondary" className="gap-1 text-[11px] shrink-0">
							<span
								className={`w-2 h-2 rounded-full ${VISIBILITY_META[visibility].color}`}
							/>
							{VISIBILITY_META[visibility].title}
						</Badge>
					</div>
				</SheetHeader>

				<Tabs
					defaultValue="branding"
					className="flex-1 min-h-0 flex flex-col gap-0"
				>
					<TabsList className="mx-4 mt-4 w-fit">
						<TabsTrigger value="branding">{t('branding', 'Branding')}</TabsTrigger>
						<TabsTrigger value="apps">{t('apps', 'Apps')}</TabsTrigger>
						<TabsTrigger value="visibility">{t('visibility', 'Visibility')}</TabsTrigger>
						<TabsTrigger value="danger">{t('dangerZone', 'Danger zone')}</TabsTrigger>
					</TabsList>
					<ScrollArea className="flex-1 min-h-0">
						<div className="p-4 pb-10">
							<TabsContent value="branding" className="mt-0">
								<BrandingTab
									appId={appId}
									group={current}
									canEdit={isAnchor}
									onSaved={refresh}
								/>
							</TabsContent>
							<TabsContent value="apps" className="mt-0">
								<AppsTab
									appId={appId}
									group={current}
									isAnchor={isAnchor}
									suggestions={suggestions}
									onChange={refresh}
									onLeft={() => {
										onOpenChange(false);
										onChange();
									}}
								/>
							</TabsContent>
							<TabsContent value="visibility" className="mt-0">
								<VisibilityTab
									appId={appId}
									group={current}
									isAnchor={isAnchor}
									onChange={refresh}
								/>
							</TabsContent>
							<TabsContent value="danger" className="mt-0">
								<DangerTab
									appId={appId}
									group={current}
									isAnchor={isAnchor}
									onChange={refresh}
									onDeleted={() => {
										onOpenChange(false);
										onChange();
									}}
								/>
							</TabsContent>
						</div>
					</ScrollArea>
				</Tabs>
			</SheetContent>
		</Sheet>
	);
}

function SectionHeading({
	title,
	hint,
}: Readonly<{ title: string; hint?: string }>) {
	return (
		<div className="space-y-0.5">
			<h3 className="text-sm font-semibold">{title}</h3>
			{hint && <p className="text-xs text-muted-foreground">{hint}</p>}
		</div>
	);
}

function InfoNote({
	children,
	tone = "info",
}: Readonly<{ children: ReactNode; tone?: "info" | "warning" }>) {
	const Icon = tone === "warning" ? AlertTriangle : Info;
	return (
		<div
			className={`flex items-start gap-2 rounded-lg border p-3 text-xs ${
				tone === "warning"
					? "border-orange-500/40 bg-orange-500/5 text-foreground"
					: "bg-muted/50 text-muted-foreground"
			}`}
		>
			<Icon className="w-3.5 h-3.5 mt-0.5 shrink-0" />
			<div className="min-w-0">{children}</div>
		</div>
	);
}

function ArtworkDrop({
	label,
	hint,
	aspect,
	preview,
	seed,
	disabled,
	onFile,
}: Readonly<{
	label: string;
	hint: string;
	aspect: string;
	preview?: string | null;
	seed: string;
	disabled: boolean;
	onFile: (file: File) => Promise<void>;
}>) {
	const { t } = useTranslation("settings");
	const [dragging, setDragging] = useState(false);
	const [busy, setBusy] = useState(false);

	const accept = useCallback(
		async (file?: File | null) => {
			if (!file || disabled) return;
			if (!IMAGE_TYPES.includes(file.type)) {
				toast.error("Use a PNG, JPG or WebP image");
				return;
			}
			if (file.size > MAX_IMAGE_MB * 1024 * 1024) {
				toast.error(`Image too large (max ${MAX_IMAGE_MB}MB)`);
				return;
			}
			setBusy(true);
			try {
				await onFile(file);
			} finally {
				setBusy(false);
			}
		},
		[disabled, onFile],
	);

	const pick = () => {
		const input = document.createElement("input");
		input.type = "file";
		input.accept = IMAGE_TYPES.join(",");
		input.onchange = (event) =>
			accept((event.target as HTMLInputElement).files?.[0]);
		input.click();
	};

	return (
		<div className="space-y-1.5">
			<Label>{label}</Label>
			<button
				type="button"
				disabled={disabled || busy}
				onClick={pick}
				onDragOver={(event) => {
					event.preventDefault();
					setDragging(true);
				}}
				onDragLeave={() => setDragging(false)}
				onDrop={(event) => {
					event.preventDefault();
					setDragging(false);
					accept(event.dataTransfer.files?.[0]);
				}}
				className={`relative w-full ${aspect} overflow-hidden rounded-xl border-2 border-dashed transition-colors disabled:opacity-60 ${
					dragging
						? "border-primary bg-primary/5"
						: "border-border hover:border-primary/50"
				}`}
				style={{
					backgroundImage: preview ? undefined : seedGradient(seed),
				}}
			>
				{preview && (
					// eslint-disable-next-line @next/next/no-img-element
					<img
						src={preview}
						alt=""
						className="absolute inset-0 h-full w-full object-cover"
					/>
				)}
				<div className="absolute inset-0 flex flex-col items-center justify-center gap-1 bg-background/60 opacity-0 hover:opacity-100 transition-opacity">
					{busy ? (
						<Upload className="w-4 h-4 animate-pulse" />
					) : (
						<ImageIcon className="w-4 h-4" />
					)}
					<span className="text-[11px] font-medium">
						{busy ? "Uploading…" : t('dropOrClickToReplace', 'Drop or click to replace')}
					</span>
				</div>
			</button>
			<p className="text-[11px] text-muted-foreground">{hint}</p>
		</div>
	);
}

function BrandingTab({
	appId,
	group,
	canEdit,
	onSaved,
}: Readonly<{
	appId: string;
	group: IGroup;
	canEdit: boolean;
	onSaved: () => Promise<void>;
}>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const [name, setName] = useState(group.name ?? "");
	const [useCase, setUseCase] = useState(group.use_case ?? "");
	const [description, setDescription] = useState(group.description ?? "");
	const [tags, setTags] = useState<string[]>(group.tags ?? []);
	const [newTag, setNewTag] = useState("");
	const [busy, setBusy] = useState(false);

	useEffect(() => {
		setName(group.name ?? "");
		setUseCase(group.use_case ?? "");
		setDescription(group.description ?? "");
		setTags(group.tags ?? []);
	}, [group.name, group.use_case, group.description, group.tags]);

	const addTag = useCallback(
		(tag: string) => {
			if (!canEdit || !tag.trim()) return;
			const trimmed = tag.trim();
			setTags((previous) =>
				previous.includes(trimmed) ? previous : [...previous, trimmed],
			);
			setNewTag("");
		},
		[canEdit],
	);

	const uploadArtwork = useCallback(
		async (item: IMediaItem, file: File) => {
			try {
				await backend.teamState.pushGroupMedia(appId, group.id, item, file);
				toast.success(item === "icon" ? "Icon uploaded" : "Banner uploaded");
				await onSaved();
			} catch (error) {
				toast.error(
					error instanceof Error ? error.message : t('couldNotUploadTheImage', 'Could not upload the image'),
				);
			}
		},
		[appId, backend.teamState, group.id, onSaved],
	);

	const save = async () => {
		if (!name.trim()) return;
		setBusy(true);
		try {
			await backend.teamState.updateGroup(appId, group.id, {
				name: name.trim(),
				description: description.trim(),
				use_case: useCase.trim(),
				tags,
			});
			toast.success("Suite updated");
			await onSaved();
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('couldNotUpdateTheSuite', 'Could not update the suite'),
			);
		} finally {
			setBusy(false);
		}
	};

	return (
		<div className="space-y-6">
			<SectionHeading
				title="Identity"
				hint="How the suite reads in the store and in invitations."
			/>

			<div className="grid gap-4 sm:grid-cols-2">
				<ArtworkDrop
					label="Icon"
					hint="Square, 1:1. PNG, JPG or WebP."
					aspect="aspect-square max-w-[160px]"
					preview={group.icon}
					seed={group.id}
					disabled={!canEdit}
					onFile={(file) => uploadArtwork("icon", file)}
				/>
				<ArtworkDrop
					label="Banner"
					hint="Wide, 2:1. Shown behind the suite header."
					aspect="aspect-2/1"
					preview={group.banner}
					seed={`${group.id}-banner`}
					disabled={!canEdit}
					onFile={(file) => uploadArtwork("thumbnail", file)}
				/>
			</div>

			<div className="space-y-1.5">
				<Label htmlFor="console-suite-name">Name</Label>
				<Input
					id="console-suite-name"
					value={name}
					disabled={!canEdit}
					onChange={(event) => setName(event.target.value)}
					placeholder={t('coreSuite', 'Core Suite')}
				/>
			</div>

			<div className="space-y-1.5">
				<Label htmlFor="console-suite-usecase">
					{t('suiteLabel', 'Suite label')}{" "}
					<span className="text-muted-foreground font-normal">
						{t('optionalShownAboveTheAppName', '(optional, shown above the app name)')}
					</span>
				</Label>
				<Input
					id="console-suite-usecase"
					value={useCase}
					disabled={!canEdit}
					onChange={(event) => setUseCase(event.target.value)}
					placeholder={t('backofficePlatform', 'Back-office platform')}
				/>
			</div>

			<div className="space-y-1.5">
				<Label htmlFor="console-suite-desc">{t('description', 'Description')}</Label>
				<Textarea
					id="console-suite-desc"
					value={description}
					disabled={!canEdit}
					onChange={(event) => setDescription(event.target.value)}
					placeholder={t('whatThisSuiteOfAppsDoesTogether', 'What this suite of apps does together.')}
					rows={4}
				/>
			</div>

			<div className="space-y-2">
				<Label htmlFor="console-suite-tags">{t('tags', 'Tags')}</Label>
				<Input
					id="console-suite-tags"
					placeholder={t('typeATagAndPressEnter', 'Type a tag and press Enter...')}
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
				{tags.length > 0 && (
					<div className="flex flex-wrap gap-2">
						{tags.map((tag) => (
							<Badge
								key={tag}
								variant="secondary"
								className="flex items-center gap-1"
							>
								{tag}
								{canEdit && (
									<button
										type="button"
										onClick={() =>
											setTags((previous) =>
												previous.filter((entry) => entry !== tag),
											)
										}
										className="ml-1 hover:text-destructive"
									>
										<X className="w-3 h-3" />
									</button>
								)}
							</Badge>
						))}
					</div>
				)}
			</div>

			{canEdit ? (
				<Button onClick={save} disabled={busy || !name.trim()}>
					{t('saveChanges2', 'Save changes')}
				</Button>
			) : (
				<InfoNote>
					{t('onlyTheAnchorAppCanEditThisSuiteapossBranding', "Only the anchor app can edit this suite's branding.")}
				</InfoNote>
			)}
		</div>
	);
}

function useAppVisibilityMap() {
	const backend = useBackend();
	const apps = useInvoke(backend.appState.getApps, backend.appState, []);
	return useMemo(() => {
		const map = new Map<
			string,
			{ visibility: IAppVisibility; name?: string }
		>();
		for (const entry of apps.data ?? []) {
			const [app, metadata] = entry as [IApp, IMetadata | undefined];
			map.set(app.id, { visibility: app.visibility, name: metadata?.name });
		}
		return map;
	}, [apps.data]);
}

function isStoreVisible(visibility?: IAppVisibility): boolean {
	return (
		visibility === IAppVisibility.Public ||
		visibility === IAppVisibility.PublicRequestAccess
	);
}

function AppsTab({
	appId,
	group,
	isAnchor,
	suggestions,
	onChange,
	onLeft,
}: Readonly<{
	appId: string;
	group: IGroup;
	isAnchor: boolean;
	suggestions: { id: string; name: string }[];
	onChange: () => Promise<void>;
	onLeft: () => void;
}>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const [busy, setBusy] = useState(false);
	const [search, setSearch] = useState("");
	const [manualId, setManualId] = useState("");
	const appMap = useAppVisibilityMap();

	const memberIds = useMemo(
		() => new Set(group.members.map((member) => member.app_id)),
		[group.members],
	);
	const suiteIsPublic = isStoreVisible(fromWireVisibility(group.visibility));

	const candidates = useMemo(() => {
		const seen = new Set<string>();
		const list: { id: string; name: string }[] = [];
		for (const app of suggestions) {
			if (memberIds.has(app.id) || seen.has(app.id)) continue;
			seen.add(app.id);
			list.push(app);
		}
		for (const [id, meta] of appMap) {
			if (memberIds.has(id) || seen.has(id)) continue;
			seen.add(id);
			list.push({ id, name: meta.name ?? id });
		}
		return list;
	}, [suggestions, appMap, memberIds]);

	const matches = useSearch(candidates, search, {
		fields: ["name", "id"],
		boost: { name: 3 },
	});
	const filtered = useMemo(() => matches.slice(0, 8), [matches]);

	const quickAdd = useMemo(
		() => suggestions.filter((app) => !memberIds.has(app.id)).slice(0, 6),
		[suggestions, memberIds],
	);

	const addMember = async (targetId: string) => {
		const target = targetId.trim();
		if (!target) return;
		setBusy(true);
		try {
			await backend.teamState.addGroupMember(appId, group.id, target);
			toast.success("App added to the suite");
			setManualId("");
			setSearch("");
			await onChange();
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('couldNotAddTheApp', 'Could not add the app'),
			);
		} finally {
			setBusy(false);
		}
	};

	const removeMember = async (memberAppId: string) => {
		setBusy(true);
		try {
			await backend.teamState.removeGroupMember(appId, group.id, memberAppId);
			toast.success("App removed from the suite");
			await onChange();
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('couldNotRemoveTheApp', 'Could not remove the app'),
			);
		} finally {
			setBusy(false);
		}
	};

	const leave = async () => {
		setBusy(true);
		try {
			await backend.teamState.leaveGroup(appId, group.id);
			toast.success("Left the suite");
			onLeft();
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('couldNotLeaveTheSuite', 'Could not leave the suite'),
			);
		} finally {
			setBusy(false);
		}
	};

	const ownMembership = group.members.find(
		(member) => member.app_id === appId && member.kind !== "PRIMARY",
	);

	return (
		<div className="space-y-6">
			<SectionHeading
				title={t('member_countAppvalInThisSuite', '{{member_count}} app{{val}} in this suite', { member_count: group.member_count, val: group.member_count === 1 ? "" : "s" })}
				hint="Membership is presentation only. Each app keeps its own team, permissions and visibility."
			/>

			<div className="space-y-2">
				{group.members.map((member) => {
					const known = appMap.get(member.app_id);
					const hiddenMeta =
						suiteIsPublic && known && !isStoreVisible(known.visibility)
							? VISIBILITY_META[known.visibility]
							: undefined;
					return (
						<div key={member.id} className="rounded-lg border bg-card p-3">
							<div className="flex items-center gap-2.5">
								<Avatar className="h-8 w-8 rounded-md">
									{member.app_icon ? (
										<AvatarImage src={member.app_icon} alt="" />
									) : null}
									<AvatarFallback
										className="rounded-md text-white text-[10px] font-bold"
										style={{ backgroundImage: seedGradient(member.app_id) }}
									>
										{initials(member.app_name)}
									</AvatarFallback>
								</Avatar>
								<div className="min-w-0 flex-1">
									<p className="text-sm font-medium truncate">
										{member.app_name ?? member.app_id}
									</p>
									{member.app_description && (
										<p className="text-xs text-muted-foreground truncate">
											{member.app_description}
										</p>
									)}
								</div>
								{member.kind === "PRIMARY" && (
									<Badge variant="outline" className="text-[10px]">
										{t('anchor', 'Anchor')}
									</Badge>
								)}
								{member.status === "PENDING" && (
									<Badge variant="secondary" className="text-[10px]">
										{t('pending', 'Pending')}
									</Badge>
								)}
								{isAnchor && member.kind !== "PRIMARY" && (
									<Button
										size="icon"
										variant="ghost"
										className="h-7 w-7"
										disabled={busy}
										onClick={() => removeMember(member.app_id)}
									>
										<X className="w-3.5 h-3.5" />
									</Button>
								)}
							</div>
							{hiddenMeta && (
								<p className="mt-2 flex items-start gap-1.5 text-[11px] text-muted-foreground">
									<Info className="w-3 h-3 mt-0.5 shrink-0" />{t('thisAppIsTitleItStaysInTheSuiteButWillNotBeShownInTheStoreUntilItIsPublishedItself', "This app is {{title}} — it stays in the suite but will not be shown in the store until it is published itself.", { title: hiddenMeta.title })}</p>
							)}
						</div>
					);
				})}
			</div>

			{ownMembership && (
				<div className="rounded-lg border p-3 space-y-2">
					<p className="text-xs text-muted-foreground">
						{t('thisAppDecidesForItselfWhetherItStaysListedInsideTheSuite', "This app decides for itself whether it stays listed inside the suite.")}
					</p>
					<Button
						variant="outline"
						size="sm"
						disabled={busy}
						onClick={leave}
						className="text-destructive hover:text-destructive"
					>
						<LogOut className="w-3.5 h-3.5 mr-1.5" />
						{t('leaveSuite', 'Leave suite')}
					</Button>
				</div>
			)}

			{isAnchor && (
				<div className="space-y-3 border-t pt-4">
					<SectionHeading
						title={t('addAnApp', 'Add an app')}
						hint="Connected apps join instantly; everyone else receives an invite to accept."
					/>

					{quickAdd.length > 0 && (
						<div className="flex flex-wrap gap-1.5">
							<span className="text-[11px] text-muted-foreground w-full">
								{t('connectedAppsJoinInstantly', 'Connected apps — join instantly:')}
							</span>
							{quickAdd.map((app) => (
								<button
									key={app.id}
									type="button"
									disabled={busy}
									onClick={() => addMember(app.id)}
									className="inline-flex items-center gap-1 rounded-full border bg-background px-2.5 py-1 text-[11px] transition-colors hover:border-primary/50 hover:bg-primary/5 disabled:opacity-50"
								>
									<Plus className="w-3 h-3" />
									{app.name}
								</button>
							))}
						</div>
					)}

					<div className="space-y-2">
						<div className="relative">
							<Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground" />
							<Input
								value={search}
								onChange={(event) => setSearch(event.target.value)}
								placeholder={t('searchYourApps', 'Search your apps…')}
								className="pl-8 h-9"
							/>
						</div>
						<div className="rounded-lg border divide-y">
							{filtered.length === 0 ? (
								<p className="p-3 text-xs text-muted-foreground">
									{t('noAppsMatchThatSearch', 'No apps match that search.')}
								</p>
							) : (
								filtered.map((app) => (
									<div key={app.id} className="flex items-center gap-2.5 p-2.5">
										<Avatar className="h-7 w-7 rounded-md">
											<AvatarFallback
												className="rounded-md text-white text-[10px] font-bold"
												style={{ backgroundImage: seedGradient(app.id) }}
											>
												{initials(app.name)}
											</AvatarFallback>
										</Avatar>
										<div className="min-w-0 flex-1">
											<p className="text-sm truncate">{app.name}</p>
											<p className="text-[10px] text-muted-foreground truncate font-mono">
												{app.id}
											</p>
										</div>
										<Button
											size="sm"
											variant="secondary"
											disabled={busy}
											onClick={() => addMember(app.id)}
										>
											<Plus className="w-3.5 h-3.5 mr-1" />
											{t('add', 'Add')}
										</Button>
									</div>
								))
							)}
						</div>
					</div>

					<details className="text-xs text-muted-foreground">
						<summary className="cursor-pointer select-none">
							{t('addByAppId', 'Add by app ID')}
						</summary>
						<div className="flex items-center gap-2 pt-2">
							<Input
								value={manualId}
								onChange={(event) => setManualId(event.target.value)}
								placeholder={t('appId', 'App ID')}
								className="h-8 text-xs font-mono"
							/>
							<Button
								size="sm"
								variant="secondary"
								disabled={busy || !manualId.trim()}
								onClick={() => addMember(manualId)}
							>
								<Plus className="w-3.5 h-3.5" />
							</Button>
						</div>
					</details>
				</div>
			)}
		</div>
	);
}

function PublicationRequestPanel({
	request,
}: Readonly<{ request: IGroupPublicationRequest }>) {
	const { t } = useTranslation("settings");
	return (
		<div className="rounded-lg border border-primary/40 bg-primary/5 p-3 space-y-2">
			<div className="flex items-center gap-2">
				<Clock className="w-4 h-4 text-primary" />
				<p className="text-sm font-medium">{t('submittedForReview', 'Submitted for review')}</p>
				<Badge variant="secondary" className="text-[10px] ml-auto">
					{request.status}
				</Badge>
			</div>
			<p className="text-xs text-muted-foreground">
				{t('targetVisibility', 'Target visibility')}{" "}{t('titleSubmitted', "{{title}} · submitted ", { title: VISIBILITY_META[fromWireVisibility(request.targetVisibility)].title })}{formatRelativeTime(request.createdAt)}
			</p>
			{request.logs.length > 0 && (
				<ul className="space-y-1.5 border-t pt-2">
					{request.logs.map((log) => (
						<li key={log.id} className="text-xs text-muted-foreground">
							<span className="text-foreground">
								{log.message ?? "Status update"}
							</span>{" "}
							· {formatRelativeTime(log.createdAt)}
						</li>
					))}
				</ul>
			)}
		</div>
	);
}

function MemberReadinessList({
	readiness,
}: Readonly<{ readiness: IMemberReadiness[] }>) {
	const { t } = useTranslation("settings");
	if (readiness.length === 0) return null;
	return (
		<div className="space-y-2">
			<SectionHeading
				title={t('euAiActReadiness', 'EU AI Act readiness')}
				hint="Every active member app needs a submitted, non-blocked assessment before the suite can be published."
			/>
			<div className="rounded-lg border divide-y">
				{readiness.map((entry) => (
					<div key={entry.appId} className="flex items-center gap-2 p-2.5">
						{entry.ready ? (
							<CheckCircle2 className="w-4 h-4 text-emerald-500 shrink-0" />
						) : (
							<ShieldAlert className="w-4 h-4 text-orange-500 shrink-0" />
						)}
						<span className="text-xs font-mono truncate flex-1">
							{entry.appId}
						</span>
						<Badge
							variant={entry.ready ? "secondary" : "outline"}
							className="text-[10px]"
						>
							{entry.aiActStatus ?? t('notStarted', 'Not started')}
						</Badge>
					</div>
				))}
			</div>
		</div>
	);
}

function VisibilityTab({
	appId,
	group,
	isAnchor,
	onChange,
}: Readonly<{
	appId: string;
	group: IGroup;
	isAnchor: boolean;
	onChange: () => Promise<void>;
}>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const publication = useInvoke(
		backend.teamState.getGroupPublication,
		backend.teamState,
		[appId, group.id],
		isAnchor,
	);

	const visibility = fromWireVisibility(group.visibility);
	const status = publication.data;
	const canPublish = status?.canRequestPublication ?? true;
	const pending = status?.requests.find((request) =>
		["PENDING", "ONHOLD", "ON_HOLD"].includes(request.status.toUpperCase()),
	);
	const blockers = (status?.memberReadiness ?? []).filter(
		(entry) => !entry.ready,
	);

	const transitions = useMemo(() => {
		const all = getVisibilityTransitions(visibility);
		if (canPublish) return all;
		return all.filter(
			(target) =>
				!(
					(target === IAppVisibility.Public ||
						target === IAppVisibility.PublicRequestAccess) &&
					!isStoreVisible(visibility)
				),
		);
	}, [visibility, canPublish]);

	const handleChange = useCallback(
		async (_entityId: string, next: IAppVisibility) => {
			const result = await backend.teamState.changeGroupVisibility(
				appId,
				group.id,
				next,
			);
			await invalidate(backend.teamState.getGroupPublication, [
				appId,
				group.id,
			]);
			await onChange();
			return { reviewRequested: result.reviewRequested };
		},
		[appId, backend.teamState, group.id, invalidate, onChange],
	);

	return (
		<div className="space-y-4">
			{pending && <PublicationRequestPanel request={pending} />}

			<EntityVisibilitySwitcher
				entityId={group.id}
				visibility={visibility}
				canEdit={isAnchor}
				entityNoun="suite"
				onVisibilityChange={handleChange}
				availableTransitions={transitions}
			/>

			{!isAnchor && (
				<InfoNote>
					{t('onlyTheAnchorAppCanPublishThisSuiteYourAppapossOwnVisibilityIsNeverChangedByTheSuite', "Only the anchor app can publish this suite. Your app's own visibility is never changed by the suite.")}
				</InfoNote>
			)}

			{isAnchor && !canPublish && !isStoreVisible(visibility) && (
				<InfoNote tone="warning">
					{group.member_count === 0
						? t('aSuiteNeedsAtLeastOneMemberAppBeforeItCanBePublished', 'A suite needs at least one member app before it can be published.')
						: pending
							? t('aReviewIsAlreadyPendingForThisSuite', 'A review is already pending for this suite.')
							: t('publishingIsBlockedUntilEveryActiveMemberAppClearsTheEuAiActGateLengthOutstanding', 'Publishing is blocked until every active member app clears the EU AI Act gate ({{length}} outstanding).', { length: blockers.length })}
				</InfoNote>
			)}

			{isAnchor && isStoreVisible(visibility) && (
				<InfoNote>
					{`Member apps that are not published themselves stay in the suite but are hidden from the store listing.`}
				</InfoNote>
			)}

			{isAnchor && (
				<MemberReadinessList readiness={status?.memberReadiness ?? []} />
			)}
		</div>
	);
}

const STATUS_OPTIONS = [
	{ value: "ACTIVE", label: "Active — visible wherever the suite is shared" },
	{ value: "INACTIVE", label: "Inactive — hidden, keeps its members" },
	{ value: "ARCHIVED", label: "Archived — retired, read-only" },
];

function DangerTab({
	appId,
	group,
	isAnchor,
	onChange,
	onDeleted,
}: Readonly<{
	appId: string;
	group: IGroup;
	isAnchor: boolean;
	onChange: () => Promise<void>;
	onDeleted: () => void;
}>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const [busy, setBusy] = useState(false);

	if (!isAnchor) {
		return (
			<InfoNote>
				{`Only the anchor app can archive or delete this suite. Your app can leave it from the Apps tab at any time.`}
			</InfoNote>
		);
	}

	const changeStatus = async (status: string) => {
		setBusy(true);
		try {
			await backend.teamState.updateGroup(appId, group.id, { status });
			toast.success("Suite status updated");
			await onChange();
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('couldNotUpdateTheStatus', 'Could not update the status'),
			);
		} finally {
			setBusy(false);
		}
	};

	const deleteGroup = async () => {
		setBusy(true);
		try {
			await backend.teamState.deleteGroup(appId, group.id);
			toast.success("Suite deleted");
			onDeleted();
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('couldNotDeleteTheSuite', 'Could not delete the suite'),
			);
		} finally {
			setBusy(false);
		}
	};

	return (
		<div className="space-y-6">
			<div className="space-y-2">
				<SectionHeading
					title="Lifecycle"
					hint="Retiring a suite never touches the member apps themselves."
				/>
				<Select
					value={group.status}
					disabled={busy}
					onValueChange={changeStatus}
				>
					<SelectTrigger className="w-full sm:w-96">
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						{STATUS_OPTIONS.map((option) => (
							<SelectItem key={option.value} value={option.value}>
								{option.label}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
				<p className="text-[11px] text-muted-foreground flex items-center gap-1.5">
					<Archive className="w-3 h-3" />
					{`Archived suites disappear from the store but stay recoverable.`}
				</p>
			</div>

			<div className="space-y-2 border-t pt-4">
				<SectionHeading
					title={t('deleteSuite', 'Delete suite')}
					hint="This cannot be undone. The member apps are not affected."
				/>
				<AlertDialog>
					<AlertDialogTrigger asChild>
						<Button variant="destructive" size="sm" disabled={busy}>
							<Trash2 className="w-3.5 h-3.5 mr-1.5" />
							{t('deleteSuite', 'Delete suite')}
						</Button>
					</AlertDialogTrigger>
					<AlertDialogContent>
						<AlertDialogHeader>
							<AlertDialogTitle>{t('deleteThisSuite', 'Delete this suite?')}</AlertDialogTitle>
							<AlertDialogDescription>
								{t('theSuiteAndItsCurationAreRemovedTheMemberAppsThemselvesAreNotAffected', "The suite and its curation are removed. The member apps themselves are not affected.")}
							</AlertDialogDescription>
						</AlertDialogHeader>
						<AlertDialogFooter>
							<AlertDialogCancel>{t('cancel', 'Cancel')}</AlertDialogCancel>
							<AlertDialogAction onClick={deleteGroup}>
								{t('delete', 'Delete')}
							</AlertDialogAction>
						</AlertDialogFooter>
					</AlertDialogContent>
				</AlertDialog>
			</div>
		</div>
	);
}
