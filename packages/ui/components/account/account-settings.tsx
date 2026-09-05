"use client";

import { useTranslation } from "@flow-like/locales";
import { useQueryClient } from "@tanstack/react-query";
import {
	ArrowUpRight,
	Check,
	CreditCard,
	ImagePlus,
	KeyRound,
	Loader2,
	Mail,
	ShieldCheck,
	UserRound,
} from "lucide-react";
import Link from "next/link";
import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { useInvoke } from "../../hooks/use-invoke";
import { userAvatarUrl, userInitials } from "../../lib/user-display";
import { useBackend } from "../../state/backend-state";
import type { IUserInfo } from "../../state/backend-state/user-state";
import { Alert, AlertDescription } from "../ui/alert";
import { Avatar, AvatarFallback, AvatarImage } from "../ui/avatar";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../ui/card";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import { Skeleton } from "../ui/skeleton";
import { Textarea } from "../ui/textarea";
import {
	accountDraft,
	accountError,
	accountHasChanges,
	invalidateAccountIdentity,
	mergeAccountDraft,
} from "./account-model";

export interface ProfileActions {
	updateEmail?: () => Promise<void>;
	handleAttributeUpdate?: (attribute: string, value: string) => Promise<void>;
	changePassword?: () => Promise<void>;
	viewBilling?: () => Promise<void>;
	previewProfile?: () => Promise<void>;
	viewSubscription?: () => Promise<void>;
	providerManaged?: boolean;
	credentialsReady?: boolean;
}

export function ProfilePage({ actions = {} }: { actions?: ProfileActions }) {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const client = useQueryClient();
	const info = useInvoke(backend.userState.getInfo, backend.userState, []);
	const [draft, setDraft] = useState(accountDraft());
	const baseline = useRef({ id: "", draft: accountDraft() });
	const [pending, setPending] = useState<"profile" | "avatar" | null>(null);
	const busy = useRef(false);
	const [error, setError] = useState("");
	const photoInput = useRef<HTMLInputElement>(null);
	const [preview, setPreview] = useState<string>();

	useEffect(() => {
		if (!info.data) return;
		const next = accountDraft(info.data);
		const previous = baseline.current;
		setDraft((current) =>
			previous.id === info.data.id
				? mergeAccountDraft(current, previous.draft, next)
				: next,
		);
		baseline.current = { id: info.data.id, draft: next };
	}, [info.data]);
	useEffect(
		() => () => {
			if (preview) URL.revokeObjectURL(preview);
		},
		[preview],
	);

	const saved = accountDraft(info.data);
	const hasChanges = accountHasChanges(draft, saved);
	const canEditUsername = Boolean(actions.handleAttributeUpdate);

	async function save(event: React.FormEvent) {
		event.preventDefault();
		if (busy.current || !info.data || !hasChanges) return;
		const next = {
			...draft,
			name: draft.name.replace(/\s+/g, " ").trim(),
			username: draft.username.trim(),
		};
		if (!next.name) {
			setError(
				t(
					"accountNameRequired",
					"Enter a display name so people can recognize you.",
				),
			);
			return;
		}
		if (canEditUsername && next.username !== saved.username && !next.username) {
			setError(
				t(
					"accountUsernameRequired",
					"Enter a username, or keep your current username.",
				),
			);
			return;
		}
		busy.current = true;
		setPending("profile");
		setError("");
		let usernameSaved = false;
		let profileSaved = false;
		try {
			if (actions.handleAttributeUpdate && next.username !== saved.username) {
				await actions.handleAttributeUpdate(
					"preferred_username",
					next.username,
				);
				usernameSaved = true;
			}
			if (next.name !== saved.name || next.description !== saved.description) {
				await backend.userState.updateUser({
					name: next.name,
					description: next.description,
				});
				profileSaved = true;
			}
			const infoKey = [backend.userState.getInfo.name];
			// Both writes completed. Keep the confirmed values if refreshing fails.
			client.setQueryData<IUserInfo>(infoKey, {
				...info.data,
				name: next.name,
				description: next.description,
				preferred_username: canEditUsername
					? next.username
					: info.data.preferred_username,
			});
			await invalidateAccountIdentity(client);
			const refreshed = client.getQueryData<IUserInfo>(infoKey);
			const confirmed = refreshed ? accountDraft(refreshed) : next;
			baseline.current = { id: info.data.id, draft: confirmed };
			setDraft(confirmed);
			toast.success(t("accountProfileSaved", "Profile saved."));
		} catch (cause) {
			if (usernameSaved || profileSaved)
				await invalidateAccountIdentity(client);
			setError(
				usernameSaved
					? t(
							"accountPartialSave",
							"Your username changed, but your other profile changes could not be saved. Your draft is still here. Try saving again.",
						)
					: accountError(
							cause,
							t(
								"accountSaveFailed",
								"Your profile could not be saved. Your changes are still here. Try again.",
							),
						),
			);
		} finally {
			busy.current = false;
			setPending(null);
		}
	}

	async function upload(event: React.ChangeEvent<HTMLInputElement>) {
		const file = event.currentTarget.files?.[0];
		event.currentTarget.value = "";
		if (!file || busy.current) return;
		if (
			!/\.(webp|png|jpe?g|gif|avif)$/i.test(file.name) ||
			!file.type.startsWith("image/")
		) {
			setError(
				t(
					"accountPhotoFormat",
					"Choose a PNG, JPEG, WebP, GIF, or AVIF image.",
				),
			);
			return;
		}
		if (file.size > 10 * 1024 * 1024) {
			setError(
				t("accountPhotoTooLarge", "Choose an image smaller than 10 MB."),
			);
			return;
		}
		busy.current = true;
		setPending("avatar");
		setError("");
		try {
			await backend.userState.updateUser({}, file);
			setPreview(URL.createObjectURL(file));
			await invalidateAccountIdentity(client);
			toast.success(t("accountPhotoUpdated", "Profile photo updated."));
		} catch (cause) {
			setError(
				accountError(
					cause,
					t(
						"accountPhotoFailed",
						"Your photo could not be updated. Try again.",
					),
				),
			);
		} finally {
			busy.current = false;
			setPending(null);
		}
	}

	const header = (
		<header className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
			<div className="space-y-2">
				<p className="text-xs font-semibold uppercase tracking-[0.16em] text-muted-foreground">
					{t("account", "Account")}
				</p>
				<h1 className="text-3xl font-semibold tracking-tight">
					{t("accountSettings", "Account settings")}
				</h1>
				<p className="max-w-xl text-sm leading-6 text-muted-foreground">
					{t(
						"accountSettingsDescription",
						"Choose how you appear to others and manage how you sign in.",
					)}
				</p>
			</div>
			{actions.previewProfile && (
				<Button
					variant="outline"
					className="w-fit shrink-0"
					onClick={actions.previewProfile}
				>
					{t("viewProfile", "View profile")}
					<ArrowUpRight className="size-4" />
				</Button>
			)}
		</header>
	);

	if (!info.data)
		return (
			<main className="mx-auto w-full max-w-5xl space-y-8 px-4 py-8 sm:px-8">
				{header}
				{info.isError ? (
					<Alert variant="destructive">
						<AlertDescription className="space-y-3">
							<p>
								{t(
									"accountLoadFailed",
									"Your account settings could not be loaded.",
								)}
							</p>
							<Button variant="outline" onClick={() => info.refetch()}>
								{t("retry", "Retry")}
							</Button>
						</AlertDescription>
					</Alert>
				) : (
					<div
						aria-label={t("accountLoading", "Loading account settings")}
						aria-live="polite"
						className="space-y-4"
					>
						<Skeleton className="h-32 w-full rounded-xl" />
						<Skeleton className="h-80 w-full rounded-xl" />
					</div>
				)}
			</main>
		);

	return (
		<main className="mx-auto w-full max-w-5xl space-y-8 px-4 py-8 sm:px-8">
			{header}
			{error && (
				<Alert variant="destructive" role="alert">
					<AlertDescription>{error}</AlertDescription>
				</Alert>
			)}
			{info.isError && (
				<Alert>
					<AlertDescription className="flex flex-wrap items-center justify-between gap-3">
						<span>
							{t(
								"accountRefreshFailed",
								"Your latest account details could not be loaded. Your draft is preserved.",
							)}
						</span>
						<Button variant="outline" size="sm" onClick={() => info.refetch()}>
							{t("retry", "Retry")}
						</Button>
					</AlertDescription>
				</Alert>
			)}
			<div className="grid items-start gap-6 lg:grid-cols-[minmax(0,1fr)_18rem]">
				<div className="min-w-0 space-y-6">
					<Card className="overflow-hidden">
						<CardHeader>
							<CardTitle className="flex items-center gap-2">
								<UserRound className="size-5 text-muted-foreground" />
								{t("publicProfile", "Public profile")}
							</CardTitle>
							<CardDescription>
								{t(
									"accountPublicProfileHelp",
									"Your photo, display name, username, and bio identify you in Flow-Like.",
								)}
							</CardDescription>
						</CardHeader>
						<CardContent>
							<div className="mb-6 flex flex-col gap-4 border-b pb-6 sm:flex-row sm:items-center">
								<Avatar className="size-20 shrink-0 border">
									<AvatarImage
										src={preview ?? userAvatarUrl(info.data)}
										alt={draft.name}
									/>
									<AvatarFallback className="text-xl">
										{userInitials(draft.name)}
									</AvatarFallback>
								</Avatar>
								<div className="space-y-2">
									<Button
										type="button"
										variant="outline"
										size="sm"
										disabled={Boolean(pending)}
										onClick={() => photoInput.current?.click()}
									>
										{pending === "avatar" ? (
											<Loader2 className="size-4 animate-spin" />
										) : (
											<ImagePlus className="size-4" />
										)}
										{t("accountChangePhoto", "Change photo")}
									</Button>
									<p className="text-xs leading-5 text-muted-foreground">
										{t(
											"accountPhotoHelp",
											"PNG, JPEG, WebP, GIF, or AVIF. Up to 10 MB. Photos save immediately.",
										)}
									</p>
									<input
										ref={photoInput}
										type="file"
										accept=".png,.jpg,.jpeg,.webp,.gif,.avif"
										aria-label={t("accountUploadPhoto", "Upload profile photo")}
										onChange={upload}
										className="hidden"
									/>
								</div>
							</div>
							<form
								onSubmit={save}
								className="space-y-5"
								aria-busy={pending === "profile"}
							>
								<div className="space-y-2">
									<Label htmlFor="account-name">
										{t("displayName", "Display name")}
									</Label>
									<Input
										id="account-name"
										name="name"
										autoComplete="name"
										required
										maxLength={96}
										disabled={Boolean(pending)}
										value={draft.name}
										onChange={(event) =>
											setDraft((value) => ({
												...value,
												name: event.target.value,
											}))
										}
									/>
									<p className="text-xs text-muted-foreground">
										{t(
											"accountNameHelp",
											"Use the name you want other people to see.",
										)}
									</p>
								</div>
								<div className="space-y-2">
									<Label htmlFor="account-username">
										{t("username2", "Username")}
									</Label>
									<Input
										id="account-username"
										name="username"
										autoComplete="username"
										maxLength={128}
										readOnly={!canEditUsername}
										disabled={Boolean(pending)}
										aria-describedby="account-username-help"
										value={draft.username}
										onChange={(event) =>
											setDraft((value) => ({
												...value,
												username: event.target.value,
											}))
										}
									/>
									<p
										id="account-username-help"
										className="text-xs text-muted-foreground"
									>
										{canEditUsername
											? t(
													"accountUsernameHelp",
													"Your username appears alongside your display name.",
												)
											: actions.credentialsReady === false
												? t(
														"accountSignInLoading",
														"Loading your sign-in options...",
													)
												: t(
														"accountUsernameManaged",
														"Your sign-in provider manages this username.",
													)}
									</p>
								</div>
								<div className="space-y-2">
									<Label htmlFor="account-bio">{t("accountBio", "Bio")}</Label>
									<Textarea
										id="account-bio"
										name="bio"
										maxLength={2000}
										disabled={Boolean(pending)}
										value={draft.description}
										onChange={(event) =>
											setDraft((value) => ({
												...value,
												description: event.target.value,
											}))
										}
										placeholder={t(
											"accountBioPlaceholder",
											"Share what you work on or what interests you.",
										)}
										className="min-h-28"
									/>
									<p className="text-right text-xs tabular-nums text-muted-foreground">
										{draft.description.length}/2,000
									</p>
								</div>
								<div className="flex flex-wrap items-center justify-between gap-3 border-t pt-5">
									<p
										aria-live="polite"
										className="flex items-center gap-1.5 text-xs text-muted-foreground"
									>
										{hasChanges ? (
											t("accountUnsaved", "Unsaved changes")
										) : (
											<>
												<Check className="size-3.5" />
												{t("accountUpToDate", "Your profile is up to date")}
											</>
										)}
									</p>
									<div className="flex gap-2">
										{hasChanges && (
											<Button
												type="button"
												variant="ghost"
												disabled={Boolean(pending)}
												onClick={() => {
													setDraft(saved);
													setError("");
												}}
											>
												{t("discard", "Discard")}
											</Button>
										)}
										<Button
											type="submit"
											disabled={Boolean(pending) || !hasChanges}
										>
											{pending === "profile" && (
												<Loader2 className="size-4 animate-spin" />
											)}
											{t("saveChanges", "Save changes")}
										</Button>
									</div>
								</div>
							</form>
						</CardContent>
					</Card>
					<Card>
						<CardHeader>
							<CardTitle className="flex items-center gap-2">
								<ShieldCheck className="size-5 text-muted-foreground" />
								{t("accountSignInSecurity", "Sign-in and security")}
							</CardTitle>
							<CardDescription>
								{actions.providerManaged
									? t(
											"accountProviderManaged",
											"Your sign-in provider manages your email and password. Update them in your provider's account settings.",
										)
									: t(
											"accountSecurityHelp",
											"Keep your sign-in details current. Changes here save separately from your public profile.",
										)}
							</CardDescription>
						</CardHeader>
						<CardContent className="divide-y">
							<div className="flex flex-col gap-3 pb-4 sm:flex-row sm:items-center sm:justify-between">
								<div className="min-w-0 space-y-1">
									<p className="flex items-center gap-2 text-sm font-medium">
										<Mail className="size-4 text-muted-foreground" />
										{t("email", "Email")}
									</p>
									<p className="break-all text-sm text-muted-foreground">
										{info.data.email ||
											t("accountNoEmail", "No email address available")}
									</p>
								</div>
								{actions.updateEmail && (
									<Button
										variant="outline"
										size="sm"
										className="w-fit shrink-0"
										disabled={Boolean(pending)}
										onClick={actions.updateEmail}
									>
										{t("accountChangeEmail", "Change email")}
									</Button>
								)}
							</div>
							<div className="flex flex-col gap-3 pt-4 sm:flex-row sm:items-center sm:justify-between">
								<div className="space-y-1">
									<p className="flex items-center gap-2 text-sm font-medium">
										<KeyRound className="size-4 text-muted-foreground" />
										{t("password", "Password")}
									</p>
									<p className="text-xs text-muted-foreground">
										{actions.providerManaged
											? t(
													"accountPasswordProvider",
													"Managed by your sign-in provider",
												)
											: t(
													"accountPasswordPrivate",
													"Your password is never displayed.",
												)}
									</p>
								</div>
								{actions.changePassword && (
									<Button
										variant="outline"
										size="sm"
										className="w-fit shrink-0"
										disabled={Boolean(pending)}
										onClick={actions.changePassword}
									>
										{t("changePassword", "Change password")}
									</Button>
								)}
							</div>
						</CardContent>
					</Card>
				</div>
				<aside className="space-y-6">
					<Card className="bg-muted/20">
						<CardHeader>
							<CardTitle className="text-base">
								{t("accountProfileNote", "Workspace profiles")}
							</CardTitle>
							<CardDescription className="leading-6">
								{t(
									"accountProfileNoteHelp",
									"Choose models and appearance for each workspace profile in Profile settings.",
								)}
							</CardDescription>
						</CardHeader>
						<CardContent>
							<Button variant="outline" size="sm" asChild>
								<Link href="/settings/profiles">
									{t("accountOpenProfiles", "Profile settings")}
									<ArrowUpRight className="size-4" />
								</Link>
							</Button>
						</CardContent>
					</Card>
					{(actions.viewSubscription || actions.viewBilling) && (
						<Card>
							<CardHeader>
								<CardTitle className="flex items-center gap-2 text-base">
									<CreditCard className="size-4 text-muted-foreground" />
									{t("accountPlanBilling", "Plan and billing")}
								</CardTitle>
							</CardHeader>
							<CardContent className="space-y-4">
								<div className="flex items-center justify-between gap-2">
									<span className="text-sm text-muted-foreground">
										{t("currentPlan", "Current plan")}
									</span>
									<Badge variant="secondary">{info.data.tier ?? "FREE"}</Badge>
								</div>
								{actions.viewSubscription && (
									<Button
										variant="outline"
										className="w-full"
										onClick={actions.viewSubscription}
									>
										{t("viewPlans", "View plans")}
									</Button>
								)}
								{actions.viewBilling && (
									<Button
										variant="ghost"
										className="w-full"
										onClick={actions.viewBilling}
									>
										{t("manageBilling", "Manage billing")}
										<ArrowUpRight className="size-4" />
									</Button>
								)}
							</CardContent>
						</Card>
					)}
				</aside>
			</div>
		</main>
	);
}
