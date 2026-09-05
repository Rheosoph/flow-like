"use client";

import { useTranslation } from "@flow-like/locales";
import { Camera, Check, Loader2, Trash2, Upload, X } from "lucide-react";
import Link from "next/link";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { apiErrorMessage } from "../../../lib/api-error";
import { parseDateValue } from "../../../lib/date";
import {
	IConnectionMode,
	type ISettingsProfile,
	IThemes,
} from "../../../types";
import {
	AlertDialog,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
	AlertDialogTrigger,
} from "../../ui/alert-dialog";
import { Avatar, AvatarFallback, AvatarImage } from "../../ui/avatar";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";
import { Switch } from "../../ui/switch";
import { Textarea } from "../../ui/textarea";
import type { ProfileSaveStatus } from "./profile-draft";
import {
	parseProfileTheme,
	profileThemeCss,
	themeSelection,
} from "./profile-theme";

export interface ProfileSettingsPageProps {
	profile: ISettingsProfile;
	isCustomTheme: boolean;
	hasChanges: boolean;
	saveStatus?: ProfileSaveStatus;
	saveError?: string | null;
	onRetrySave?: () => void;
	themeTranslation: Record<IThemes, unknown>;
	onProfileUpdate: (updates: Partial<ISettingsProfile>) => void;
	onProfileImageChange?: () => Promise<void>;
	onProfileDelete?: () => Promise<void>;
	canDeleteProfile?: boolean;
	deleteScope?: "cloud" | "local";
	supportsExecutionSettings?: boolean;
}

export function ProfileSettingsPage({
	profile,
	isCustomTheme,
	hasChanges,
	saveStatus = hasChanges ? "pending" : "saved",
	saveError,
	onRetrySave,
	themeTranslation,
	onProfileUpdate,
	onProfileImageChange,
	onProfileDelete,
	canDeleteProfile = false,
	deleteScope = "cloud",
	supportsExecutionSettings = true,
}: ProfileSettingsPageProps) {
	const { t } = useTranslation("settings");
	const theme = profile.hub_profile.theme;
	const [selectedTheme, setSelectedTheme] = useState(() =>
		themeSelection(theme),
	);
	const [customCss, setCustomCss] = useState(() =>
		isCustomTheme ? profileThemeCss(theme) : "",
	);
	const [customName, setCustomName] = useState(() =>
		isCustomTheme ? theme.id : "My theme",
	);
	const [importError, setImportError] = useState<string | null>(null);
	const [imageError, setImageError] = useState<string | null>(null);
	const [changingImage, setChangingImage] = useState(false);
	const imageBusy = useRef(false);
	const themeId = theme?.id;
	useEffect(() => {
		setSelectedTheme(themeSelection({ id: themeId }));
	}, [themeId]);

	const updateHub = (updates: Partial<ISettingsProfile["hub_profile"]>) =>
		onProfileUpdate({ hub_profile: { ...profile.hub_profile, ...updates } });
	const changeImage = async () => {
		if (!onProfileImageChange || imageBusy.current) return;
		imageBusy.current = true;
		setChangingImage(true);
		setImageError(null);
		try {
			await onProfileImageChange();
		} catch (error) {
			setImageError(
				apiErrorMessage(
					error,
					error instanceof Error
						? error.message
						: "Could not update the image. Try again.",
				),
			);
		} finally {
			imageBusy.current = false;
			setChangingImage(false);
		}
	};
	const nameInvalid =
		!profile.hub_profile.name.trim() || profile.hub_profile.name.length > 100;
	const context = profile.execution_settings.max_context_size;
	const contextInvalid =
		!Number.isInteger(context) || context < 0 || context > 4294967295;

	return (
		<main className="flex-1 min-h-0 overflow-y-auto bg-background">
			<div className="mx-auto w-full max-w-5xl px-4 py-6 sm:px-6 lg:px-8">
				<header className="space-y-4 border-b pb-6">
					<div className="flex items-start justify-between gap-4 flex-wrap">
						<div className="min-w-0 space-y-2">
							<p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
								{t("settings", "Settings")}
							</p>
							<h1 className="text-2xl font-semibold tracking-tight sm:text-3xl">
								{t("workspaceProfile", "Workspace profile")}
							</h1>
							<p className="max-w-xl text-sm leading-6 text-muted-foreground">
								{t(
									"workspaceProfileDescription",
									"Organize your apps and personalize this workspace. Changes save automatically.",
								)}
							</p>
						</div>
						<output
							aria-live="polite"
							className="flex min-h-9 items-center gap-2 text-sm text-muted-foreground"
						>
							{saveStatus === "saved" ? (
								<Check className="h-4 w-4" aria-hidden="true" />
							) : saveStatus === "saving" ? (
								<Loader2 className="h-4 w-4 animate-spin" aria-hidden="true" />
							) : null}
							{saveStatus === "saved"
								? t("allChangesSaved", "All changes saved")
								: saveStatus === "saving"
									? t("savingChanges", "Saving changes…")
									: saveStatus === "error"
										? t("changesNotSaved", "Changes not saved")
										: t("unsavedChanges", "Unsaved changes")}
						</output>
					</div>
					<p className="text-sm text-muted-foreground">
						{t(
							"accountDetailsSeparate",
							"Your public name, email and password are in",
						)}{" "}
						<Link
							className="font-medium text-foreground underline underline-offset-4"
							href="/account"
						>
							{t("accountSettings", "account settings")}
						</Link>
						.
					</p>
					{saveError && (
						<div
							role="alert"
							className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm"
						>
							<p className="min-w-0 break-words">
								{saveError} {t("draftRetained", "Your changes are still here.")}
							</p>
							<Button
								variant="outline"
								size="sm"
								onClick={onRetrySave}
								disabled={nameInvalid || contextInvalid}
							>
								{t("retrySave", "Retry save")}
							</Button>
						</div>
					)}
				</header>

				<fieldset disabled={changingImage} className="min-w-0">
					<SettingsSection
						title={t("profileDetails", "Profile details")}
						description={t(
							"profileDetailsDescription",
							"A name and image to recognize this workspace in the profile switcher.",
						)}
					>
						<div className="flex flex-wrap items-center gap-4">
							<Avatar className="h-20 w-20 rounded-xl border">
								<AvatarImage
									className="object-cover"
									src={profile.hub_profile.icon ?? undefined}
									alt={profile.hub_profile.name || "Workspace profile"}
								/>
								<AvatarFallback className="rounded-xl text-xl">
									{profile.hub_profile.name.trim().slice(0, 2).toUpperCase() ||
										"WP"}
								</AvatarFallback>
							</Avatar>
							{onProfileImageChange && (
								<div className="space-y-2">
									<Button
										variant="outline"
										onClick={changeImage}
										disabled={changingImage}
									>
										{changingImage ? (
											<Loader2 className="h-4 w-4 animate-spin" />
										) : (
											<Camera className="h-4 w-4" />
										)}
										{changingImage
											? t("updatingImage", "Updating image…")
											: t("changeImage", "Change image")}
									</Button>
									<p className="text-xs text-muted-foreground">
										{t(
											"profileImageHelp",
											"Choose a PNG, JPEG or WebP image up to 10 MB.",
										)}
									</p>
								</div>
							)}
						</div>
						{imageError && (
							<p role="alert" className="text-sm text-destructive">
								{imageError}
							</p>
						)}
						<div className="space-y-2">
							<Label htmlFor="workspace-profile-name">
								{t("profileName", "Profile name")}
							</Label>
							<Input
								id="workspace-profile-name"
								value={profile.hub_profile.name}
								maxLength={100}
								aria-invalid={nameInvalid}
								aria-describedby={
									nameInvalid ? "workspace-profile-name-error" : undefined
								}
								onChange={(event) => updateHub({ name: event.target.value })}
							/>
							{nameInvalid && (
								<p
									id="workspace-profile-name-error"
									className="text-sm text-destructive"
								>
									{t(
										"profileNameRequired",
										"Enter a name with 1 to 100 characters.",
									)}
								</p>
							)}
						</div>
						<div className="space-y-2">
							<Label htmlFor="workspace-profile-description">
								{t("description", "Description")}
							</Label>
							<Textarea
								id="workspace-profile-description"
								value={profile.hub_profile.description ?? ""}
								placeholder={t(
									"profileDescriptionExample",
									"What do you use this workspace for?",
								)}
								rows={3}
								onChange={(event) =>
									updateHub({ description: event.target.value })
								}
							/>
						</div>
						<div className="grid gap-5 sm:grid-cols-2">
							<TagsInput
								id="workspace-tags"
								label={t("tags", "Tags")}
								tags={profile.hub_profile.tags ?? []}
								onChange={(tags) => updateHub({ tags })}
							/>
							<TagsInput
								id="workspace-interests"
								label={t("interests", "Interests")}
								tags={profile.hub_profile.interests ?? []}
								onChange={(interests) => updateHub({ interests })}
							/>
						</div>
					</SettingsSection>

					<SettingsSection
						title={t("appearance", "Appearance")}
						description={t(
							"profileAppearanceDescription",
							"Choose the colors used when this workspace is active.",
						)}
					>
						<div className="space-y-2">
							<Label htmlFor="workspace-theme">
								{t("themeLabel", "Theme")}
							</Label>
							<Select
								value={selectedTheme}
								onValueChange={(value) => {
									setSelectedTheme(value);
									setImportError(null);
									if (value !== "CUSTOM")
										updateHub({
											theme: themeTranslation[value as IThemes] ?? null,
										});
								}}
							>
								<SelectTrigger id="workspace-theme">
									<SelectValue placeholder={t("selectTheme", "Select theme")} />
								</SelectTrigger>
								<SelectContent className="max-h-64">
									{Object.values(IThemes).map((value) => (
										<SelectItem key={value} value={value}>
											{value}
										</SelectItem>
									))}
									<SelectItem value="CUSTOM">
										{isCustomTheme
											? theme.id
											: t("customImport", "Custom (import)")}
									</SelectItem>
								</SelectContent>
							</Select>
						</div>
						{selectedTheme === "CUSTOM" && (
							<div className="space-y-4 rounded-lg border bg-muted/20 p-4">
								<div className="space-y-2">
									<Label htmlFor="custom-theme-name">
										{t("themeName", "Theme name")}
									</Label>
									<Input
										id="custom-theme-name"
										value={customName}
										onChange={(event) => setCustomName(event.target.value)}
									/>
								</div>
								<div className="space-y-2">
									<Label htmlFor="custom-theme-css">
										{t("themeCss", "Theme CSS")}
									</Label>
									<Textarea
										id="custom-theme-css"
										className="font-mono text-xs"
										rows={8}
										value={customCss}
										onChange={(event) => setCustomCss(event.target.value)}
										aria-describedby="custom-theme-help"
									/>
									<p
										id="custom-theme-help"
										className="text-xs text-muted-foreground"
									>
										{t(
											"customThemeHelp",
											"Paste a tweakcn export with both :root and .dark blocks.",
										)}
									</p>
								</div>
								{importError && (
									<p role="alert" className="text-sm text-destructive">
										{importError}
									</p>
								)}
								<Button
									variant="outline"
									onClick={() => {
										try {
											updateHub({
												theme: parseProfileTheme(customCss, customName),
											});
											setImportError(null);
										} catch (error) {
											setImportError(
												error instanceof Error
													? error.message
													: "Could not import theme.",
											);
										}
									}}
								>
									<Upload className="h-4 w-4" />
									{t("applyTheme", "Apply theme")}
								</Button>
							</div>
						)}
					</SettingsSection>

					<SettingsSection
						title={t("flowEditor", "Flow editor")}
						description={t(
							"flowEditorDescription",
							"Set how connections appear between nodes.",
						)}
					>
						<div className="space-y-2">
							<Label htmlFor="workspace-connection-mode">
								{t("connectionStyle", "Connection style")}
							</Label>
							<Select
								value={
									profile.hub_profile.settings?.connection_mode ??
									IConnectionMode.Simplebezier
								}
								onValueChange={(value: IConnectionMode) =>
									updateHub({
										settings: {
											...profile.hub_profile.settings,
											connection_mode: value,
										},
									})
								}
							>
								<SelectTrigger id="workspace-connection-mode">
									<SelectValue />
								</SelectTrigger>
								<SelectContent>
									<SelectItem value={IConnectionMode.Default}>
										{t("default", "Default")}
									</SelectItem>
									<SelectItem value={IConnectionMode.Straight}>
										{t("straight", "Straight")}
									</SelectItem>
									<SelectItem value={IConnectionMode.Step}>
										{t("step", "Step")}
									</SelectItem>
									<SelectItem value={IConnectionMode.Smoothstep}>
										{t("smoothStep", "Smooth step")}
									</SelectItem>
									<SelectItem value={IConnectionMode.Simplebezier}>
										{t("simpleBezier", "Simple Bézier")}
									</SelectItem>
								</SelectContent>
							</Select>
						</div>
					</SettingsSection>

					<SettingsSection
						title={t("localExecution", "Local execution")}
						description={t(
							"localExecutionDescription",
							"Model performance preferences are stored on each desktop device.",
						)}
					>
						{supportsExecutionSettings ? (
							<>
								<div className="space-y-2">
									<Label htmlFor="workspace-context-size">
										{t("maxContextSize", "Maximum context size")}
									</Label>
									<Input
										id="workspace-context-size"
										type="number"
										min={0}
										max={4294967295}
										step={1}
										value={Number.isFinite(context) ? context : ""}
										aria-invalid={contextInvalid}
										aria-describedby="workspace-context-help"
										onChange={(event) =>
											onProfileUpdate({
												execution_settings: {
													...profile.execution_settings,
													max_context_size:
														event.target.value === ""
															? 0
															: Number(event.target.value),
												},
											})
										}
									/>
									<p
										id="workspace-context-help"
										className={`text-xs ${contextInvalid ? "text-destructive" : "text-muted-foreground"}`}
									>
										{contextInvalid
											? t(
													"contextSizeInvalid",
													"Enter a whole number of zero or more.",
												)
											: t(
													"contextSizeHelp",
													"0 uses the default limit of 32,000 tokens. Higher limits use more memory.",
												)}
									</p>
								</div>
								<div className="flex items-center justify-between gap-4">
									<div className="space-y-1">
										<Label htmlFor="workspace-gpu">
											{t("gpuAcceleration", "GPU acceleration")}
										</Label>
										<p
											id="workspace-gpu-help"
											className="text-xs text-muted-foreground"
										>
											{t(
												"gpuHelp",
												"Use a supported GPU for local models when available.",
											)}
										</p>
									</div>
									<Switch
										id="workspace-gpu"
										aria-describedby="workspace-gpu-help"
										checked={profile.execution_settings.gpu_mode}
										onCheckedChange={(gpu_mode) =>
											onProfileUpdate({
												execution_settings: {
													...profile.execution_settings,
													gpu_mode,
												},
											})
										}
									/>
								</div>
							</>
						) : (
							<p className="text-sm leading-6 text-muted-foreground">
								{t(
									"desktopExecutionOnly",
									"Open this profile in the desktop app to adjust GPU acceleration and context size. These preferences do not change cloud execution.",
								)}
							</p>
						)}
					</SettingsSection>
				</fieldset>
				<div className="flex flex-wrap gap-x-6 gap-y-2 py-5 text-xs text-muted-foreground">
					{profile.hub_profile.hub && (
						<span className="break-all">
							{t("hub", "Hub")}: {profile.hub_profile.hub}
						</span>
					)}
					<span>
						{t("created", "Created")}:{" "}
						{parseDateValue(profile.created)?.toLocaleDateString() ??
							t("unknown", "Unknown")}
					</span>
					<span>
						{t("updated", "Updated")}:{" "}
						{parseDateValue(profile.updated)?.toLocaleDateString() ??
							t("unknown", "Unknown")}
					</span>
				</div>
				{onProfileDelete && (
					<DeleteProfileCard
						profileName={profile.hub_profile.name}
						onDelete={onProfileDelete}
						canDelete={canDeleteProfile && !changingImage}
						scope={deleteScope}
					/>
				)}
			</div>
		</main>
	);
}

function SettingsSection({
	title,
	description,
	children,
}: { title: string; description: string; children: ReactNode }) {
	return (
		<section className="grid gap-5 border-b py-7 md:grid-cols-[210px_minmax(0,1fr)] md:gap-10">
			<div className="space-y-2">
				<h2 className="text-base font-semibold">{title}</h2>
				<p className="text-sm leading-6 text-muted-foreground">{description}</p>
			</div>
			<div className="min-w-0 space-y-5">{children}</div>
		</section>
	);
}

function TagsInput({
	id,
	label,
	tags,
	onChange,
}: {
	id: string;
	label: string;
	tags: string[];
	onChange: (tags: string[]) => void;
}) {
	const { t } = useTranslation("settings");
	const [value, setValue] = useState("");
	const add = () => {
		const tag = value.trim();
		if (tag && !tags.includes(tag)) onChange([...tags, tag]);
		setValue("");
	};
	return (
		<div className="min-w-0 space-y-2">
			<Label htmlFor={id}>{label}</Label>
			<div className="flex gap-2">
				<Input
					id={id}
					value={value}
					onChange={(event) => setValue(event.target.value)}
					onKeyDown={(event) => {
						if (event.key === "Enter") {
							event.preventDefault();
							add();
						}
					}}
					placeholder={t("addEntry", "Add an entry")}
				/>
				<Button
					variant="outline"
					onClick={add}
					disabled={!value.trim()}
					aria-label={t("addToField", "Add to {{label}}", { label })}
				>
					{t("add", "Add")}
				</Button>
			</div>
			<div className="flex flex-wrap gap-2">
				{tags.map((tag) => (
					<Badge
						key={tag}
						variant="secondary"
						className="max-w-full gap-1 whitespace-normal break-all"
					>
						{tag}
						<button
							type="button"
							className="shrink-0 rounded p-1.5 hover:text-destructive focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
							aria-label={t("removeTag", "Remove {{tag}}", { tag })}
							onClick={() => onChange(tags.filter((item) => item !== tag))}
						>
							<X className="h-3 w-3" />
						</button>
					</Badge>
				))}
			</div>
		</div>
	);
}

function DeleteProfileCard({
	profileName,
	onDelete,
	canDelete,
	scope,
}: {
	profileName: string;
	onDelete: () => Promise<void>;
	canDelete: boolean;
	scope: "cloud" | "local";
}) {
	const { t } = useTranslation("settings");
	const [open, setOpen] = useState(false);
	const [confirmName, setConfirmName] = useState("");
	const [deleting, setDeleting] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const busy = useRef(false);
	const isConfirmed = !!profileName.trim() && confirmName === profileName;
	const action =
		scope === "local"
			? t("removeFromDevice", "Remove from this device")
			: t("deleteProfile", "Delete profile");
	const explanation =
		scope === "local"
			? t(
					"localProfileRemoval",
					"This removes the local profile. Cloud copies remain and may return when you sign in. Sign in first to delete it from synced devices.",
				)
			: t(
					"cloudProfileRemoval",
					"This deletes the profile from this account and its synced devices. Your apps remain in your library.",
				);
	const remove = async () => {
		if (!isConfirmed || busy.current) return;
		busy.current = true;
		setDeleting(true);
		setError(null);
		try {
			await onDelete();
			setOpen(false);
			setConfirmName("");
		} catch (error) {
			setError(
				apiErrorMessage(
					error,
					error instanceof Error
						? error.message
						: "Could not remove the profile. Try again.",
				),
			);
		} finally {
			busy.current = false;
			setDeleting(false);
		}
	};
	return (
		<section className="mt-2 flex flex-wrap items-center justify-between gap-4 rounded-lg border border-destructive/25 p-5">
			<div className="max-w-xl space-y-1">
				<h2 className="text-sm font-semibold">{action}</h2>
				<p className="text-sm leading-6 text-muted-foreground">
					{canDelete
						? explanation
						: t(
								"onlyProfileRemoval",
								"Create another profile before removing this one.",
							)}
				</p>
			</div>
			<AlertDialog
				open={open}
				onOpenChange={(value) => {
					if (!busy.current) {
						setOpen(value);
						setConfirmName("");
						setError(null);
					}
				}}
			>
				<AlertDialogTrigger asChild>
					<Button
						variant="destructive"
						className="bg-[color-mix(in_oklch,var(--destructive)_85%,black)] hover:bg-[color-mix(in_oklch,var(--destructive)_75%,black)]"
						disabled={!canDelete}
					>
						<Trash2 className="h-4 w-4" />
						{action}
					</Button>
				</AlertDialogTrigger>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>{action}</AlertDialogTitle>
						<AlertDialogDescription>{explanation}</AlertDialogDescription>
					</AlertDialogHeader>
					<div className="space-y-2">
						<Label htmlFor="confirm-profile-removal">
							{t("typeProfileNameToConfirm", "Type {{name}} to confirm", {
								name: profileName,
							})}
						</Label>
						<Input
							id="confirm-profile-removal"
							value={confirmName}
							disabled={deleting}
							autoComplete="off"
							onChange={(event) => setConfirmName(event.target.value)}
							onKeyDown={(event) => {
								if (event.key === "Enter") {
									event.preventDefault();
									void remove();
								}
							}}
						/>
					</div>
					{error && (
						<p role="alert" className="text-sm text-destructive">
							{error}
						</p>
					)}
					<AlertDialogFooter>
						<AlertDialogCancel disabled={deleting}>
							{t("cancel", "Cancel")}
						</AlertDialogCancel>
						<Button
							variant="destructive"
							className="bg-[color-mix(in_oklch,var(--destructive)_85%,black)] hover:bg-[color-mix(in_oklch,var(--destructive)_75%,black)]"
							disabled={!isConfirmed || deleting}
							onClick={remove}
						>
							{deleting && <Loader2 className="h-4 w-4 animate-spin" />}
							{deleting ? t("removing", "Removing…") : action}
						</Button>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</section>
	);
}
