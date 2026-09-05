"use client";

import { useQueryClient } from "@tanstack/react-query";
import {
	ArrowLeft,
	ArrowUpRight,
	Boxes,
	Check,
	Image,
	LayoutTemplate,
	Loader2,
	Plus,
	Save,
	Settings2,
	X,
} from "lucide-react";
import { useRouter, useSearchParams } from "next/navigation";
import {
	type ReactNode,
	Suspense,
	useEffect,
	useId,
	useRef,
	useState,
} from "react";
import { toast } from "sonner";
import {
	IConnectionMode,
	type IProfile,
} from "../../lib/schema/profile/profile";
import { HomeAppPicker } from "../home/home-widget-settings";
import {
	AlertDialog,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "../ui/alert-dialog";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import { Switch } from "../ui/switch";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../ui/tabs";
import { Textarea } from "../ui/textarea";
import { ProfileBitsPicker } from "./profile-bits-picker";
import { ProfileMediaField } from "./profile-media-field";
import {
	clearProfileTemplateDraft,
	readProfileTemplateDraft,
	writeProfileTemplateDraft,
} from "./profile-template-drafts";
import {
	createProfileTemplate,
	prepareProfileTemplate,
} from "./profile-template-model";
import { ProfileTemplatePreview } from "./profile-template-preview";
import { ProfileTemplateStatus } from "./profile-templates-page";
import { useProfileTemplates } from "./use-profile-templates";

export type ProfileMediaUpload = (url: string, file: Blob) => Promise<void>;
const browserUpload: ProfileMediaUpload = async (url, file) => {
	const headers: Record<string, string> = { "Content-Type": file.type };
	if (new URL(url).hostname.endsWith(".blob.core.windows.net"))
		headers["x-ms-blob-type"] = "BlockBlob";
	const response = await fetch(url, { method: "PUT", body: file, headers });
	if (!response.ok)
		throw new Error(`Image upload failed (${response.status}). Try again.`);
};

export function ProfileTemplateEditorPage({
	uploadMedia = browserUpload,
}: { uploadMedia?: ProfileMediaUpload }) {
	return (
		<Suspense
			fallback={<ProfileTemplateStatus message="Loading profile editor…" />}
		>
			<ProfileTemplateEditorRoute uploadMedia={uploadMedia} />
		</Suspense>
	);
}

function ProfileTemplateEditorRoute({
	uploadMedia,
}: { uploadMedia: ProfileMediaUpload }) {
	const context = useProfileTemplates();
	const params = useSearchParams();
	const id = params.get("id");
	const copy = params.get("copy");
	const sourceId = id ?? copy;
	const source = context.templates.data?.find(
		(profile) => profile.id === sourceId,
	);
	if (context.loading || (sourceId && context.templates.isLoading))
		return <ProfileTemplateStatus message="Loading profile editor…" />;
	if (
		context.info.isError ||
		context.profile.isError ||
		(sourceId && context.templates.isError)
	)
		return (
			<ProfileTemplateStatus
				message="The profile could not be loaded."
				retry={() => {
					void context.info.refetch();
					void context.profile.refetch();
					void context.templates.refetch();
				}}
			/>
		);
	if (!context.canWrite)
		return (
			<ProfileTemplateStatus message="You need permission to edit profile templates." />
		);
	if (sourceId && !source)
		return (
			<ProfileTemplateStatus message="This starter profile no longer exists. Return to Starter profiles and choose another." />
		);
	if (!context.profile.data)
		return (
			<ProfileTemplateStatus message="Your admin profile could not be loaded." />
		);
	return (
		<ProfileTemplateEditor
			key={`${context.scopeKey}:${id ?? `copy:${copy ?? "new"}`}`}
			draftKey={`${context.scopeKey}:${id ?? `copy:${copy ?? "new"}`}`}
			initial={
				id && source
					? source
					: createProfileTemplate(
							context.profile.data.hub,
							source,
							context.profile.data.secure,
						)
			}
			existing={!!id}
			context={context}
			uploadMedia={uploadMedia}
		/>
	);
}

function ProfileTemplateEditor({
	initial,
	existing,
	context,
	uploadMedia,
	draftKey,
}: {
	initial: IProfile;
	existing: boolean;
	context: ReturnType<typeof useProfileTemplates>;
	uploadMedia: ProfileMediaUpload;
	draftKey: string;
}) {
	const router = useRouter();
	const client = useQueryClient();
	const [restored] = useState(() => readProfileTemplateDraft(draftKey));
	const [draft, setDraft] = useState(restored?.draft ?? initial);
	const [baseline, setBaseline] = useState(restored?.baseline ?? initial);
	const [resumed, setResumed] = useState(Boolean(restored));
	const [saving, setSaving] = useState(false);
	const [uploads, setUploads] = useState(0);
	const [mediaBusy, setMediaBusy] = useState(false);
	const [pendingNavigation, setPendingNavigation] = useState<string | null>(
		null,
	);
	const [error, setError] = useState<string | null>(null);
	const [tab, setTab] = useState("identity");
	const formRef = useRef<HTMLFormElement>(null);
	const mounted = useRef(true);
	const savingRef = useRef(false);
	const mediaOperations = useRef(new Set<string>());
	const busyMedia = (field: string, active: boolean) => {
		if (active) mediaOperations.current.add(field);
		else mediaOperations.current.delete(field);
		if (mounted.current) setMediaBusy(mediaOperations.current.size > 0);
	};
	const dirty = JSON.stringify(draft) !== JSON.stringify(baseline);
	const busy = saving || uploads > 0 || mediaBusy;
	useEffect(() => {
		if (dirty) writeProfileTemplateDraft(draftKey, { draft, baseline });
		else clearProfileTemplateDraft(draftKey);
	}, [draftKey, draft, baseline, dirty]);
	const update = <K extends keyof IProfile>(key: K, value: IProfile[K]) => {
		if (mounted.current && !savingRef.current)
			setDraft((previous) => ({ ...previous, [key]: value }));
	};
	useEffect(() => {
		mounted.current = true;
		return () => {
			mounted.current = false;
		};
	}, []);
	useEffect(() => {
		if (!dirty && !busy) return;
		const prevent = (event: BeforeUnloadEvent) => {
			event.preventDefault();
			event.returnValue = "";
		};
		window.addEventListener("beforeunload", prevent);
		return () => window.removeEventListener("beforeunload", prevent);
	}, [dirty, busy]);
	const leave = (path: string) => {
		if (busy || savingRef.current || mediaOperations.current.size) return;
		if (dirty) setPendingNavigation(path);
		else router.push(path);
	};
	const upload = async (file: Blob) => {
		const admin = context.profile.data;
		if (!admin || !context.canWrite || !mounted.current || savingRef.current)
			throw new Error(
				"Your admin profile is not available. Reload and try again.",
			);
		setUploads((value) => value + 1);
		try {
			const format =
				file.type === "image/webp"
					? "webp"
					: file.type === "image/png"
						? "png"
						: file.type === "image/jpeg"
							? "jpeg"
							: null;
			if (!format) throw new Error("This image format cannot be uploaded.");
			const signed = await context.backend.apiState.get<{
				url: string;
				final_url?: string;
			}>(admin, `admin/profiles/media?format=${format}`);
			if (!mounted.current) throw new Error("The profile editor was closed.");
			if (!signed?.url) throw new Error("An upload URL could not be created.");
			await uploadMedia(signed.url, file);
			return signed.final_url || signed.url.split("?")[0];
		} finally {
			if (mounted.current) setUploads((value) => value - 1);
		}
	};
	const save = async () => {
		if (
			busy ||
			savingRef.current ||
			mediaOperations.current.size ||
			!context.canWrite ||
			!context.profile.data
		)
			return;
		if (!draft.name.trim()) {
			setTab("identity");
			setError("Give this profile a name before saving.");
			requestAnimationFrame(() =>
				formRef.current
					?.querySelector<HTMLInputElement>("#template-name")
					?.focus(),
			);
			return;
		}
		if (
			(draft.apps?.length ?? 0) > 500 ||
			draft.apps?.some(
				(app) =>
					!app.app_id.trim() ||
					app.app_id.length > 200 ||
					app.app_id.includes("\0"),
			)
		) {
			setTab("apps");
			setError(
				"Choose up to 500 apps with nonempty app IDs of at most 200 characters.",
			);
			return;
		}
		savingRef.current = true;
		setSaving(true);
		setError(null);
		try {
			const payload = prepareProfileTemplate(draft);
			const saved = await context.backend.apiState.put<IProfile>(
				context.profile.data,
				`admin/profiles/${encodeURIComponent(draft.id ?? "")}`,
				payload,
			);
			if (!saved?.id)
				throw new Error(
					"The server did not return the saved profile. Your draft is still here.",
				);
			client.setQueryData<IProfile[]>(context.queryKey, (current) => [
				...(current ?? []).filter((item) => item.id !== saved.id),
				saved,
			]);
			const currentDraft = readProfileTemplateDraft(draftKey);
			if (
				!currentDraft ||
				JSON.stringify(currentDraft.draft) === JSON.stringify(draft)
			)
				clearProfileTemplateDraft(draftKey);
			if (!mounted.current) return;
			setResumed(false);
			setDraft(saved);
			setBaseline(saved);
			toast.success(existing ? "Profile updated" : "Starter profile created");
			void Promise.all([
				client.invalidateQueries({ queryKey: ["profile-templates"] }),
				client.invalidateQueries({ queryKey: ["home-default-templates"] }),
				client.invalidateQueries({ queryKey: ["info", "profiles"] }),
			]);
			if (!existing || saved.id !== draft.id)
				router.replace(
					`/admin/profiles/add?id=${encodeURIComponent(saved.id)}`,
				);
		} catch (cause) {
			if (mounted.current)
				setError(
					cause instanceof Error
						? cause.message
						: "The profile could not be saved. Your changes are still here.",
				);
		} finally {
			savingRef.current = false;
			if (mounted.current) setSaving(false);
		}
	};
	return (
		<main className="mx-auto w-full max-w-7xl px-4 pb-12 pt-5 sm:px-8">
			<form
				ref={formRef}
				onSubmit={(event) => {
					event.preventDefault();
					void save();
				}}
			>
				<header className="sticky top-0 z-20 -mx-4 mb-7 flex flex-wrap items-center justify-between gap-3 border-b bg-background/95 px-4 py-4 backdrop-blur sm:-mx-8 sm:px-8">
					<div className="flex min-w-0 items-center gap-3">
						<Button
							type="button"
							size="icon"
							variant="outline"
							aria-label="Back to starter profiles"
							disabled={busy}
							onClick={() => leave("/admin/profiles")}
						>
							<ArrowLeft className="h-4 w-4" />
						</Button>
						<div className="min-w-0">
							<h1 className="truncate text-lg font-semibold">
								{existing ? "Edit starter profile" : "Create starter profile"}
							</h1>
							<output className="block text-xs text-muted-foreground">
								{mediaBusy || uploads
									? "Preparing or uploading image…"
									: saving
										? "Saving profile…"
										: dirty
											? resumed
												? "Restored unsaved changes"
												: "Unsaved changes"
											: existing
												? "All changes saved"
												: "Build a starting point for your users"}
							</output>
						</div>
					</div>
					<div className="ml-auto flex items-center gap-2">
						<Button
							type="button"
							variant="ghost"
							disabled={busy}
							onClick={() => leave("/admin/profiles")}
						>
							Cancel
						</Button>
						<Button type="submit" disabled={busy || (existing && !dirty)}>
							{busy ? (
								<Loader2 className="mr-2 h-4 w-4 animate-spin" />
							) : (
								<Save className="mr-2 h-4 w-4" />
							)}
							{existing ? "Save changes" : "Create profile"}
						</Button>
					</div>
				</header>
				{error && (
					<div
						role="alert"
						className="mb-5 rounded-xl border border-destructive/30 bg-destructive/5 p-4 text-sm text-destructive"
					>
						{error}
					</div>
				)}
				<div className="grid items-start gap-7 xl:grid-cols-[minmax(0,1fr)_320px]">
					<div className="min-w-0">
						<Tabs value={tab} onValueChange={setTab}>
							<TabsList className="mb-6 grid h-auto w-full grid-cols-4 gap-1 p-1">
								<TabsTrigger
									className="gap-1.5 px-1 py-2.5 text-xs sm:text-sm"
									value="identity"
								>
									<Image className="hidden h-4 w-4 sm:block" />
									Identity
								</TabsTrigger>
								<TabsTrigger
									className="gap-1.5 px-1 py-2.5 text-xs sm:text-sm"
									value="bits"
								>
									<Boxes className="hidden h-4 w-4 sm:block" />
									Bits
									<span className="text-muted-foreground">
										{draft.bits.length}
									</span>
								</TabsTrigger>
								<TabsTrigger
									className="gap-1.5 px-1 py-2.5 text-xs sm:text-sm"
									value="apps"
								>
									<LayoutTemplate className="hidden h-4 w-4 sm:block" />
									Apps
									<span className="text-muted-foreground">
										{draft.apps?.length ?? 0}
									</span>
								</TabsTrigger>
								<TabsTrigger
									className="gap-1.5 px-1 py-2.5 text-xs sm:text-sm"
									value="settings"
								>
									<Settings2 className="hidden h-4 w-4 sm:block" />
									Defaults
								</TabsTrigger>
							</TabsList>
							<TabsContent
								value="identity"
								forceMount
								hidden={tab !== "identity"}
								className="space-y-6 data-[state=inactive]:hidden"
							>
								<Section
									title="Make it recognizable"
									description="Tell people who this profile is for and what it helps them do."
								>
									<Field label="Profile name" htmlFor="template-name">
										<Input
											id="template-name"
											value={draft.name}
											maxLength={120}
											placeholder="e.g. Research & discovery"
											disabled={saving}
											onChange={(event) => update("name", event.target.value)}
										/>
									</Field>
									<Field label="Description" htmlFor="template-description">
										<Textarea
											id="template-description"
											value={draft.description ?? ""}
											maxLength={10000}
											rows={5}
											placeholder="A workspace for finding answers, exploring sources, and turning research into something useful."
											disabled={saving}
											onChange={(event) =>
												update("description", event.target.value)
											}
										/>
										<p className="text-xs text-muted-foreground">
											Shown when people choose a profile. Line breaks are
											supported.
										</p>
									</Field>
								</Section>
								<Section
									title="Images"
									description="Give your profile an icon and a cover that make it easy to find."
								>
									<div className="grid gap-6 sm:grid-cols-[minmax(0,1fr)_minmax(0,1.4fr)]">
										<ProfileMediaField
											label="Profile icon"
											kind="icon"
											value={draft.icon}
											disabled={saving}
											onChange={(value) => update("icon", value)}
											onBusyChange={(active) => busyMedia("icon", active)}
											upload={upload}
										/>
										<ProfileMediaField
											label="Cover image"
											kind="cover"
											value={draft.thumbnail}
											disabled={saving}
											onChange={(value) => update("thumbnail", value)}
											onBusyChange={(active) => busyMedia("cover", active)}
											upload={upload}
										/>
									</div>
								</Section>
								<Section
									title="Topics & discovery"
									description="Help people recognize a profile that matches their interests."
								>
									<TokenField
										label="Tags"
										value={draft.tags ?? []}
										onChange={(value) => update("tags", value)}
										placeholder="e.g. Research, Operations"
										disabled={saving}
									/>
									<TokenField
										label="Interests"
										value={draft.interests ?? []}
										onChange={(value) => update("interests", value)}
										placeholder="e.g. Data analysis"
										disabled={saving}
									/>
								</Section>
							</TabsContent>
							<TabsContent
								value="bits"
								forceMount
								hidden={tab !== "bits"}
								className="data-[state=inactive]:hidden"
							>
								<Section
									title="Choose the bits to start with"
									description="Bits are the models and capabilities included in this profile. Users can change their own selection later."
								>
									<ProfileBitsPicker
										value={draft.bits}
										onChange={(value) => update("bits", value)}
										disabled={saving}
									/>
								</Section>
							</TabsContent>
							<TabsContent
								value="apps"
								forceMount
								hidden={tab !== "apps"}
								className="data-[state=inactive]:hidden"
							>
								<Section
									title="A useful starting collection"
									description="Include apps and choose which ones appear in favorites or the sidebar. Each app keeps its own access requirements."
								>
									<fieldset
										disabled={saving}
										className="min-w-0 space-y-5"
										onKeyDown={(event) => {
											if (
												event.key === "Enter" &&
												event.target instanceof HTMLInputElement &&
												event.target.type === "text"
											)
												event.preventDefault();
										}}
									>
										<HomeAppPicker
											label="Included apps"
											value={(draft.apps ?? []).map((app) => app.app_id)}
											multiple
											onChange={(ids) =>
												update(
													"apps",
													ids.map(
														(id) =>
															draft.apps?.find((app) => app.app_id === id) ?? {
																app_id: id,
																favorite: false,
																pinned: false,
															},
													),
												)
											}
										/>
										{draft.apps?.map((app) => (
											<div
												key={app.app_id}
												className="flex flex-wrap items-center justify-between gap-3 rounded-lg border p-3"
											>
												<span className="min-w-0 flex-1 basis-40 break-all font-mono text-xs text-muted-foreground">
													{app.app_id}
												</span>
												<div className="flex items-center gap-4">
													<label
														htmlFor={`favorite-${app.app_id}`}
														className="flex items-center gap-2 text-xs"
													>
														<Switch
															id={`favorite-${app.app_id}`}
															disabled={saving}
															checked={app.favorite}
															onCheckedChange={(checked) =>
																update(
																	"apps",
																	draft.apps?.map((item) =>
																		item.app_id === app.app_id
																			? { ...item, favorite: checked }
																			: item,
																	),
																)
															}
														/>
														Favorite
													</label>
													<label
														htmlFor={`pinned-${app.app_id}`}
														className="flex items-center gap-2 text-xs"
													>
														<Switch
															id={`pinned-${app.app_id}`}
															disabled={saving}
															checked={app.pinned}
															onCheckedChange={(checked) =>
																update(
																	"apps",
																	draft.apps?.map((item) =>
																		item.app_id === app.app_id
																			? { ...item, pinned: checked }
																			: item,
																	),
																)
															}
														/>
														Pin
													</label>
												</div>
											</div>
										))}
									</fieldset>
								</Section>
							</TabsContent>
							<TabsContent
								value="settings"
								forceMount
								hidden={tab !== "settings"}
								className="space-y-6 data-[state=inactive]:hidden"
							>
								<Section
									title="Their first home"
									description="Profiles follow the latest main home unless you publish a home specifically for this template."
								>
									<div className="rounded-xl border bg-primary/5 p-5">
										<LayoutTemplate className="mb-3 h-6 w-6 text-primary" />
										<h3 className="font-medium">A home for this profile</h3>
										<p className="mb-4 mt-2 text-sm leading-relaxed text-muted-foreground">
											Arrange widgets, introduce the right apps, or embed an app
											directly. Users can personalize their home and reset to
											your latest default.
										</p>
										{existing && context.canEditHome ? (
											<Button
												type="button"
												variant="outline"
												disabled={busy}
												onClick={() =>
													leave(
														`/admin/home?default=${encodeURIComponent(draft.id ?? "")}`,
													)
												}
											>
												Edit default home
												<ArrowUpRight className="ml-2 h-4 w-4" />
											</Button>
										) : (
											<p className="text-xs text-muted-foreground">
												{existing
													? "An administrator with landing page permission can edit this home."
													: "Save this profile to configure its default home."}
											</p>
										)}
									</div>
								</Section>
								<Section
									title="Connection defaults"
									description="Choose the hub and editor settings new profiles begin with."
								>
									<Field label="Primary hub" htmlFor="template-hub">
										<Input
											id="template-hub"
											value={draft.hub ?? ""}
											maxLength={2048}
											disabled={saving}
											onChange={(event) => update("hub", event.target.value)}
											placeholder="hub.flow-like.com"
										/>
										<p className="text-xs text-muted-foreground">
											Optional hub address for this profile.
										</p>
									</Field>
									<TokenField
										label="Additional hubs"
										maxLength={2048}
										value={draft.hubs ?? []}
										onChange={(value) => update("hubs", value)}
										placeholder="Add a hub address"
										disabled={saving}
									/>
									<Field
										label="Flow connection style"
										htmlFor="template-connection"
									>
										<select
											id="template-connection"
											value={
												draft.settings?.connection_mode ??
												IConnectionMode.Simplebezier
											}
											disabled={saving}
											onChange={(event) =>
												update("settings", {
													...draft.settings,
													connection_mode: event.target
														.value as IConnectionMode,
												})
											}
											className="h-10 w-full rounded-md border bg-background px-3 text-sm"
										>
											{Object.entries({
												default: "Bezier",
												simplebezier: "Simple bezier",
												smoothstep: "Rounded steps",
												step: "Steps",
												straight: "Straight",
											}).map(([value, label]) => (
												<option key={value} value={value}>
													{label}
												</option>
											))}
										</select>
									</Field>
								</Section>
							</TabsContent>
						</Tabs>
					</div>
					<aside className="min-w-0 space-y-4 xl:sticky xl:top-28">
						<div className="flex items-center justify-between">
							<p className="text-xs font-medium uppercase tracking-[0.16em] text-muted-foreground">
								Profile preview
							</p>
							<span className="flex items-center gap-1 text-xs text-muted-foreground">
								<span className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
								Live
							</span>
						</div>
						<ProfileTemplatePreview profile={draft} />
						<div className="space-y-3 rounded-xl border border-dashed p-4 text-xs text-muted-foreground">
							<p className="flex items-start gap-2">
								<Check className="mt-0.5 h-3.5 w-3.5 shrink-0 text-primary" />
								New users receive the bits, apps, and settings you choose.
							</p>
							<p className="flex items-start gap-2">
								<Check className="mt-0.5 h-3.5 w-3.5 shrink-0 text-primary" />
								Existing users keep their profile settings. Their default home
								can follow your updates.
							</p>
						</div>
					</aside>
				</div>
			</form>
			<AlertDialog
				open={Boolean(pendingNavigation)}
				onOpenChange={(open) => {
					if (!open) setPendingNavigation(null);
				}}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>Discard unsaved changes?</AlertDialogTitle>
						<AlertDialogDescription>
							Your changes to this starter profile have not been saved.
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel>Keep editing</AlertDialogCancel>
						<Button
							type="button"
							variant="destructive"
							disabled={busy}
							onClick={() => {
								if (
									pendingNavigation &&
									!busy &&
									!savingRef.current &&
									!mediaOperations.current.size
								) {
									const path = pendingNavigation;
									clearProfileTemplateDraft(draftKey);
									setPendingNavigation(null);
									router.push(path);
								}
							}}
						>
							Discard changes
						</Button>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</main>
	);
}

function Section({
	title,
	description,
	children,
}: { title: string; description: string; children: ReactNode }) {
	return (
		<section className="min-w-0 rounded-2xl border bg-card p-4 sm:p-6">
			<h2 className="text-lg font-semibold tracking-tight">{title}</h2>
			<p className="mb-6 mt-1 text-sm leading-relaxed text-muted-foreground">
				{description}
			</p>
			<div className="space-y-5">{children}</div>
		</section>
	);
}
function Field({
	label,
	htmlFor,
	children,
}: { label: string; htmlFor: string; children: ReactNode }) {
	return (
		<div className="space-y-2">
			<Label htmlFor={htmlFor}>{label}</Label>
			{children}
		</div>
	);
}
function TokenField({
	label,
	value,
	onChange,
	placeholder,
	disabled,
	maxLength = 120,
}: {
	label: string;
	value: string[];
	onChange: (value: string[]) => void;
	placeholder: string;
	disabled?: boolean;
	maxLength?: number;
}) {
	const id = useId();
	const [input, setInput] = useState("");
	const add = () => {
		const next = input.trim();
		if (
			!disabled &&
			next &&
			next.length <= maxLength &&
			!value.includes(next) &&
			value.length < 50
		) {
			onChange([...value, next]);
			setInput("");
		}
	};
	return (
		<Field label={label} htmlFor={id}>
			<div className="flex gap-2">
				<Input
					id={id}
					value={input}
					maxLength={maxLength}
					placeholder={placeholder}
					disabled={disabled || value.length >= 50}
					onChange={(event) => setInput(event.target.value)}
					onKeyDown={(event) => {
						if (event.key === "Enter") {
							event.preventDefault();
							add();
						}
					}}
				/>
				<Button
					type="button"
					variant="outline"
					aria-label={`Add ${label.toLowerCase()}`}
					disabled={disabled || !input.trim() || value.length >= 50}
					onClick={add}
				>
					<Plus className="h-4 w-4" />
				</Button>
			</div>
			<div className="flex flex-wrap gap-2">
				{value.map((item) => (
					<span
						key={item}
						className="inline-flex max-w-full items-center gap-1 rounded-lg bg-muted px-2 py-1 text-xs"
					>
						<span className="break-all">{item}</span>
						<button
							type="button"
							aria-label={`Remove ${item}`}
							disabled={disabled}
							className="shrink-0 rounded p-1 hover:bg-background focus-visible:ring-2 focus-visible:ring-ring"
							onClick={() => onChange(value.filter((entry) => entry !== item))}
						>
							<X className="h-3 w-3" />
						</button>
					</span>
				))}
			</div>
		</Field>
	);
}
