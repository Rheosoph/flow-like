"use client";

import {
	AlertTriangle,
	Calendar,
	Camera,
	Cpu,
	GitBranch,
	Loader2,
	Save,
	Settings,
	Trash2,
	Upload,
	User,
	X,
	Zap,
} from "lucide-react";
import { useCallback, useRef, useState } from "react";
import type { ISettingsProfile } from "../../../types";
import { IConnectionMode, IThemes } from "../../../types";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../../ui/card";
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

export interface ProfileSettingsPageProps {
	profile: ISettingsProfile;
	isCustomTheme: boolean;
	hasChanges: boolean;
	themeTranslation: Record<IThemes, unknown>;
	onProfileUpdate: (updates: Partial<ISettingsProfile>) => void;
	onProfileImageChange?: () => Promise<void>;
	onProfileDelete?: () => Promise<void>;
	canDeleteProfile?: boolean;
}

export function ProfileSettingsPage({
	profile,
	isCustomTheme,
	hasChanges,
	themeTranslation,
	onProfileUpdate,
	onProfileImageChange,
	onProfileDelete,
	canDeleteProfile = false,
}: ProfileSettingsPageProps) {
	const [themeSelectValue, setThemeSelectValue] = useState<string>(
		profile.hub_profile.theme?.id ?? IThemes.FLOW_LIKE,
	);
	const [customCss, setCustomCss] = useState("");
	const [customThemeName, setCustomThemeName] = useState("Custom Theme");
	const [importError, setImportError] = useState<string | null>(null);

	const parseTweakcnTheme = useCallback(
		(
			input: string,
			id = "Custom Theme",
		): {
			id: string;
			light: Record<string, string>;
			dark: Record<string, string>;
		} => {
			const toCamel = (name: string) =>
				name
					.replace(/^-+/, "")
					.replace(/-([a-z0-9])/gi, (_, c) => c.toUpperCase());

			const extractBlock = (source: string, selector: string) => {
				const re = new RegExp(`${selector}\\s*\\{([\\s\\S]*?)\\}`, "m");
				const m = source.match(re);
				return m?.[1] ?? "";
			};

			const parseVars = (block: string) => {
				const out: Record<string, string> = {};
				const re = /--([a-z0-9-]+)\s*:\s*([^;]+);/gi;
				let m: RegExpExecArray | null;
				while ((m = re.exec(block))) {
					const key = toCamel(m[1]);
					const val = m[2].trim();
					out[key] = val;
				}
				return out;
			};

			const root = extractBlock(input, ":root");
			const dark = extractBlock(input, "\\.dark");
			const lightVars = parseVars(root);
			const darkVars = parseVars(dark);
			return { id, light: lightVars, dark: darkVars };
		},
		[],
	);

	const handleThemeChange = useCallback(
		(value: string) => {
			setThemeSelectValue(value);
			setImportError(null);
			if (Object.values(IThemes).includes(value as IThemes)) {
				onProfileUpdate({
					hub_profile: {
						...profile.hub_profile,
						theme:
							themeTranslation[value as IThemes] ??
							themeTranslation[IThemes.FLOW_LIKE],
					},
				});
			}
		},
		[profile, themeTranslation, onProfileUpdate],
	);

	const handleImportTheme = useCallback(() => {
		try {
			const parsed = parseTweakcnTheme(
				customCss,
				customThemeName || "Custom Theme",
			);
			if (!parsed.light?.background && !parsed.dark?.background) {
				throw new Error("No valid variables found.");
			}
			onProfileUpdate({
				hub_profile: {
					...profile.hub_profile,
					theme: parsed as unknown as typeof profile.hub_profile.theme,
				},
			});
			setThemeSelectValue("CUSTOM");
			setImportError(null);
		} catch (err: unknown) {
			setImportError((err as Error)?.message ?? "Failed to import theme.");
		}
	}, [customCss, customThemeName, profile, onProfileUpdate, parseTweakcnTheme]);

	return (
		<main className="bg-gradient-to-br from-background via-background to-muted/20 p-4 sm:p-6 flex-1 min-h-0 overflow-y-auto pb-10">
			<div className="mx-auto max-w-6xl space-y-6">
				{/* Header */}
				<div className="flex items-start sm:items-center justify-between gap-3 flex-wrap">
					<div className="space-y-1 min-w-0">
						<h1 className="text-3xl sm:text-4xl font-bold tracking-tight flex items-center gap-3 break-words">
							<User className="h-8 w-8 text-primary shrink-0" />
							{profile.hub_profile.name || "Profile Settings"}
							{isCustomTheme && (
								<Badge variant="secondary" className="ml-2">
									Custom theme
								</Badge>
							)}
						</h1>
						<p className="text-muted-foreground">
							Manage your profile settings and preferences
						</p>
					</div>
					{hasChanges && (
						<div className="flex items-center gap-2 text-sm text-muted-foreground shrink-0">
							<Save className="h-4 w-4" />
							Saving changes...
						</div>
					)}
				</div>

				<div className="grid gap-6 md:grid-cols-2 lg:grid-cols-3">
					{/* Profile Information */}
					<Card className="md:col-span-2">
						<CardHeader>
							<CardTitle className="flex items-center gap-2">
								<User className="h-5 w-5" />
								Profile Information
							</CardTitle>
							<CardDescription>
								Basic information about your profile
							</CardDescription>
						</CardHeader>
						<CardContent className="space-y-6">
							<div className="flex flex-col md:flex-row gap-4 md:gap-6">
								<div className="flex-shrink-0">
									<button
										className="relative group"
										onClick={onProfileImageChange}
										disabled={!onProfileImageChange}
									>
										<img
											title={profile.hub_profile.icon ?? ""}
											className="rounded-lg border-2 border-border hover:border-primary transition-colors w-28 sm:w-40 md:w-56 h-auto aspect-square object-cover"
											width={224}
											height={224}
											src={
												profile.hub_profile.icon ??
												"/placeholder-thumbnail.webp"
											}
											alt="Profile thumbnail"
										/>
										{onProfileImageChange && (
											<div className="absolute inset-0 bg-black/50 opacity-0 group-hover:opacity-100 transition-opacity rounded-lg flex items-center justify-center">
												<Camera className="h-8 w-8 text-white" />
											</div>
										)}
									</button>
								</div>
								<div className="flex-1 space-y-4">
									<div className="space-y-2">
										<Label htmlFor="name">Profile Name</Label>
										<Input
											id="name"
											placeholder="Enter profile name"
											value={profile.hub_profile.name}
											onChange={(e) =>
												onProfileUpdate({
													hub_profile: {
														...profile.hub_profile,
														name: e.target.value,
													},
												})
											}
										/>
									</div>
									<div className="space-y-2">
										<Label htmlFor="description">Description</Label>
										<Textarea
											id="description"
											placeholder="Describe your profile..."
											value={profile.hub_profile.description ?? ""}
											onChange={(e) =>
												onProfileUpdate({
													hub_profile: {
														...profile.hub_profile,
														description: e.target.value,
													},
												})
											}
											rows={3}
										/>
									</div>
									<div className="space-y-2">
										<Label htmlFor="hub">Current Hub</Label>
										<Input
											disabled
											id="hub"
											placeholder="Hub name or ID"
											value={profile.hub_profile.hub ?? ""}
										/>
									</div>
								</div>
							</div>

							{/* Tags Section */}
							<TagsInput
								label="Tags"
								placeholder="Add tag and press Enter"
								tags={profile.hub_profile.tags ?? []}
								onTagsChange={(tags) =>
									onProfileUpdate({
										hub_profile: { ...profile.hub_profile, tags },
									})
								}
							/>

							{/* Interests Section */}
							<TagsInput
								label="Interests"
								placeholder="Add interest and press Enter"
								tags={profile.hub_profile.interests ?? []}
								variant="outline"
								onTagsChange={(interests) =>
									onProfileUpdate({
										hub_profile: { ...profile.hub_profile, interests },
									})
								}
							/>
						</CardContent>
					</Card>

					{/* Profile Stats */}
					<Card>
						<CardHeader>
							<CardTitle className="flex items-center gap-2">
								<Calendar className="h-5 w-5" />
								Profile Stats
							</CardTitle>
						</CardHeader>
						<CardContent className="space-y-4">
							<div className="space-y-2">
								<ProfileStat
									label="Created"
									value={new Date(profile.created).toLocaleDateString()}
								/>
								<ProfileStat
									label="Updated"
									value={new Date(profile.updated).toLocaleDateString()}
								/>
								<ProfileStat
									label="Apps"
									value={profile.hub_profile.apps?.length ?? 0}
									bold
								/>
								<ProfileStat
									label="Hubs"
									value={profile.hub_profile.hubs?.length ?? 0}
									bold
								/>
								<ProfileStat
									label="Tags"
									value={profile.hub_profile.tags?.length ?? 0}
									bold
								/>
								<ProfileStat
									label="Interests"
									value={profile.hub_profile.interests?.length ?? 0}
									bold
								/>
							</div>
						</CardContent>
					</Card>

					{/* Execution Settings */}
					<Card>
						<CardHeader>
							<CardTitle className="flex items-center gap-2">
								<Cpu className="h-5 w-5" />
								Execution Settings
							</CardTitle>
							<CardDescription>
								Configure performance and execution options
							</CardDescription>
						</CardHeader>
						<CardContent className="space-y-4">
							<div className="space-y-2">
								<Label htmlFor="context_size">Max Context Size</Label>
								<Input
									id="context_size"
									placeholder="8192"
									value={profile.execution_settings?.max_context_size || ""}
									type="number"
									onChange={(e) =>
										onProfileUpdate({
											execution_settings: {
												...profile.execution_settings,
												max_context_size: Number.parseInt(e.target.value) || 0,
											},
										})
									}
								/>
							</div>
							<div className="flex items-center justify-between">
								<div className="space-y-0.5">
									<Label htmlFor="gpu" className="flex items-center gap-2">
										<Zap className="h-4 w-4" />
										GPU Mode
									</Label>
									<p className="text-sm text-muted-foreground">
										Enable GPU acceleration
									</p>
								</div>
								<Switch
									id="gpu"
									checked={profile.execution_settings?.gpu_mode ?? true}
									onCheckedChange={(checked) =>
										onProfileUpdate({
											execution_settings: {
												...profile.execution_settings,
												gpu_mode: checked,
											},
										})
									}
								/>
							</div>
						</CardContent>
					</Card>

					{/* Theme Settings */}
					<Card>
						<CardHeader>
							<CardTitle className="flex items-center gap-2">
								<Settings className="h-5 w-5" />
								Theme Settings
								{isCustomTheme && <Badge className="ml-2">Custom</Badge>}
							</CardTitle>
							<CardDescription>
								Customize your visual experience
							</CardDescription>
						</CardHeader>
						<CardContent>
							<div className="space-y-3">
								<Label htmlFor="theme">Theme</Label>
								<Select
									value={themeSelectValue}
									onValueChange={handleThemeChange}
								>
									<SelectTrigger>
										<SelectValue placeholder="Select theme" />
									</SelectTrigger>
									<SelectContent className="max-h-60">
										{Object.values(IThemes).map((theme) => (
											<SelectItem key={theme} value={theme}>
												{theme}
											</SelectItem>
										))}
										<SelectItem value="CUSTOM">Custom (import)</SelectItem>
									</SelectContent>
								</Select>

								{themeSelectValue === "CUSTOM" && (
									<div className="mt-3 space-y-3">
										<div className="grid gap-2">
											<Label htmlFor="customThemeName">Theme Name</Label>
											<Input
												id="customThemeName"
												placeholder="Custom Theme"
												value={customThemeName}
												onChange={(e) => setCustomThemeName(e.target.value)}
											/>
										</div>

										<div className="grid gap-2">
											<Label htmlFor="customTheme">Paste tweakcn export</Label>
											<Textarea
												id="customTheme"
												placeholder="Paste the CSS export from tweakcn here"
												rows={10}
												value={customCss}
												onChange={(e) => setCustomCss(e.target.value)}
											/>
										</div>

										<div className="flex items-center gap-2">
											<Button
												variant="default"
												onClick={handleImportTheme}
												className="flex items-center gap-2"
											>
												<Upload className="h-4 w-4" />
												Import & Apply
											</Button>
											{importError && (
												<span className="text-sm text-destructive">
													{importError}
												</span>
											)}
										</div>
										<p className="text-xs text-muted-foreground">
											Paste the full CSS including :root and .dark blocks from
											tweakcn.com.
										</p>
									</div>
								)}

								<p className="text-xs text-muted-foreground">
									Credits to{" "}
									<a
										href="https://tweakcn.com/"
										target="_blank"
										className="underline font-bold"
										rel="noreferrer"
									>
										tweakcn.com
									</a>
								</p>
							</div>
						</CardContent>
					</Card>

					{/* Flow Settings */}
					<Card>
						<CardHeader>
							<CardTitle className="flex items-center gap-2">
								<GitBranch className="h-5 w-5" />
								Flow Settings
							</CardTitle>
							<CardDescription>
								Configure flow visualization preferences
							</CardDescription>
						</CardHeader>
						<CardContent>
							<div className="space-y-2">
								<Label htmlFor="connection_mode">Connection Mode</Label>
								<Select
									value={
										profile.hub_profile.settings?.connection_mode ??
										IConnectionMode.Default
									}
									onValueChange={(value: IConnectionMode) =>
										onProfileUpdate({
											hub_profile: {
												...profile.hub_profile,
												settings: {
													...profile.hub_profile.settings,
													connection_mode: value,
												},
											},
										})
									}
								>
									<SelectTrigger>
										<SelectValue placeholder="Select connection mode" />
									</SelectTrigger>
									<SelectContent>
										<SelectItem value={IConnectionMode.Default}>
											Default
										</SelectItem>
										<SelectItem value={IConnectionMode.Straight}>
											Straight
										</SelectItem>
										<SelectItem value={IConnectionMode.Step}>Step</SelectItem>
										<SelectItem value={IConnectionMode.Smoothstep}>
											Smooth Step
										</SelectItem>
										<SelectItem value={IConnectionMode.Simplebezier}>
											Simple Bezier
										</SelectItem>
									</SelectContent>
								</Select>
							</div>
						</CardContent>
					</Card>
				</div>

				{onProfileDelete && (
					<DeleteProfileCard
						profileName={profile.hub_profile.name}
						onDelete={onProfileDelete}
						canDelete={canDeleteProfile}
					/>
				)}
			</div>
		</main>
	);
}

function DeleteProfileCard({
	profileName,
	onDelete,
	canDelete,
}: {
	profileName: string;
	onDelete: () => Promise<void>;
	canDelete: boolean;
}) {
	const [confirmName, setConfirmName] = useState("");
	const [isDeleting, setIsDeleting] = useState(false);
	const [isOpen, setIsOpen] = useState(false);
	const inputRef = useRef<HTMLInputElement>(null);

	const isConfirmed = confirmName === profileName;

	const handleDelete = useCallback(async () => {
		if (!isConfirmed || isDeleting) return;
		setIsDeleting(true);
		try {
			await onDelete();
		} finally {
			setIsDeleting(false);
			setIsOpen(false);
			setConfirmName("");
		}
	}, [isConfirmed, isDeleting, onDelete]);

	return (
		<Card className="border-destructive/30">
			<CardHeader>
				<CardTitle className="flex items-center gap-2 text-destructive">
					<AlertTriangle className="h-5 w-5" />
					Danger Zone
				</CardTitle>
				<CardDescription>
					Irreversible actions that affect your profile
				</CardDescription>
			</CardHeader>
			<CardContent>
				<div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
					<div className="space-y-1 min-w-0">
						<p className="text-sm font-medium">Delete this profile</p>
						<p className="text-sm text-muted-foreground">
							{canDelete
								? "Permanently delete this profile and all its data. If synced online, it will be removed from all devices."
								: "You cannot delete your only profile. Create another profile first."}
						</p>
					</div>
					{canDelete ? (
						<>
							<Button
								variant="destructive"
								className="shrink-0"
								onClick={() => {
									setIsOpen(true);
									setConfirmName("");
									setTimeout(() => inputRef.current?.focus(), 100);
								}}
							>
								<Trash2 className="h-4 w-4 mr-2" />
								Delete Profile
							</Button>
							{isOpen && (
								<div className="fixed inset-0 z-50 flex items-center justify-center">
									<div
										className="fixed inset-0 bg-black/50"
										onClick={() => {
											setIsOpen(false);
											setConfirmName("");
										}}
										onKeyDown={() => {}}
									/>
									<div className="relative z-50 w-full max-w-md rounded-lg border bg-background p-6 shadow-lg">
										<div className="space-y-4">
											<div className="space-y-2">
												<h3 className="text-lg font-semibold">
													Delete profile
												</h3>
												<p className="text-sm text-muted-foreground">
													This will permanently delete{" "}
													<span className="font-semibold text-foreground">
														{profileName}
													</span>{" "}
													and remove it from all synced devices. This action
													cannot be undone.
												</p>
											</div>
											<div className="space-y-2">
												<Label htmlFor="confirm-delete">
													Type{" "}
													<span className="font-mono font-semibold text-destructive">
														{profileName}
													</span>{" "}
													to confirm
												</Label>
												<Input
													ref={inputRef}
													id="confirm-delete"
													placeholder={profileName}
													value={confirmName}
													onChange={(e) => setConfirmName(e.target.value)}
													onKeyDown={(e) => {
														if (e.key === "Enter" && isConfirmed)
															handleDelete();
														if (e.key === "Escape") {
															setIsOpen(false);
															setConfirmName("");
														}
													}}
												/>
											</div>
											<div className="flex justify-end gap-2">
												<Button
													variant="outline"
													onClick={() => {
														setIsOpen(false);
														setConfirmName("");
													}}
												>
													Cancel
												</Button>
												<Button
													variant="destructive"
													disabled={!isConfirmed || isDeleting}
													onClick={handleDelete}
												>
													{isDeleting ? (
														<Loader2 className="h-4 w-4 animate-spin mr-2" />
													) : (
														<Trash2 className="h-4 w-4 mr-2" />
													)}
													Delete permanently
												</Button>
											</div>
										</div>
									</div>
								</div>
							)}
						</>
					) : (
						<Button variant="destructive" className="shrink-0" disabled>
							<Trash2 className="h-4 w-4 mr-2" />
							Delete Profile
						</Button>
					)}
				</div>
			</CardContent>
		</Card>
	);
}

interface TagsInputProps {
	label: string;
	placeholder: string;
	tags: string[];
	variant?: "secondary" | "outline";
	onTagsChange: (tags: string[]) => void;
}

function TagsInput({
	label,
	placeholder,
	tags,
	variant = "secondary",
	onTagsChange,
}: TagsInputProps) {
	return (
		<div className="space-y-2">
			<Label htmlFor={label.toLowerCase()}>{label}</Label>
			<div className="space-y-2">
				<Input
					id={label.toLowerCase()}
					placeholder={placeholder}
					onKeyDown={(e) => {
						if (e.key === "Enter") {
							const value = e.currentTarget.value.trim();
							if (value && !tags.includes(value)) {
								onTagsChange([...tags, value]);
								e.currentTarget.value = "";
							}
						}
					}}
				/>
				<div className="flex flex-wrap gap-2">
					{tags.map((tag, index) => (
						<Badge
							key={index}
							variant={variant}
							className="flex items-center gap-1 max-w-full break-words"
						>
							{tag}
							<X
								className="h-3 w-3 cursor-pointer hover:text-destructive"
								onClick={() => onTagsChange(tags.filter((_, i) => i !== index))}
							/>
						</Badge>
					))}
				</div>
			</div>
		</div>
	);
}

interface ProfileStatProps {
	label: string;
	value: string | number;
	bold?: boolean;
}

function ProfileStat({ label, value, bold = false }: ProfileStatProps) {
	return (
		<div className="flex justify-between text-sm">
			<span className="text-muted-foreground">{label}</span>
			<span className={bold ? "font-medium" : ""}>{value}</span>
		</div>
	);
}
