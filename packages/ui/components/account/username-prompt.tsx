"use client";

import { useTranslation } from "@flow-like/locales";
import { useQueryClient } from "@tanstack/react-query";
import { Loader2 } from "lucide-react";
import { useContext, useState } from "react";
import { AuthContext } from "react-oidc-context";
import { useInvoke } from "../../hooks/use-invoke";
import { apiErrorMessage } from "../../lib/api-error";
import { handleSlug, isEmailLike, userHandle } from "../../lib/user-display";
import { useBackend } from "../../state/backend-state";
import type {
	IUserInfo,
	IUserUpdate,
} from "../../state/backend-state/user-state";
import { Button } from "../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import { invalidateAccountIdentity } from "./account-model";

/** Accounts whose owner chose "Not now" since the app started. */
const postponed = new Set<string>();

/**
 * Asks a signed-in person without a username to choose one. The hub derives a
 * username from the name or address at sign-in, so this only appears when
 * neither gave one, as for a private relay address without a name.
 */
export function UsernamePrompt() {
	const signedIn = useContext(AuthContext)?.isAuthenticated ?? false;
	const backend = useBackend();
	const info = useInvoke(
		backend.userState.getInfo,
		backend.userState,
		[],
		signedIn,
	);
	const [, setPostponedNow] = useState(0);
	const account = info.data;
	if (
		!signedIn ||
		!account?.id ||
		userHandle(account) ||
		postponed.has(account.id)
	)
		return null;
	return (
		<UsernameDialog
			key={account.id}
			account={account}
			onPostpone={() => {
				postponed.add(account.id);
				setPostponedNow((count) => count + 1);
			}}
		/>
	);
}

function UsernameDialog({
	account,
	onPostpone,
}: Readonly<{ account: IUserInfo; onPostpone: () => void }>) {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const client = useQueryClient();
	const needsName = !account.name?.trim() || isEmailLike(account.name);
	const [name, setName] = useState("");
	const [username, setUsername] = useState(
		needsName ? "" : (handleSlug(account.name) ?? ""),
	);
	const [usernameEdited, setUsernameEdited] = useState(false);
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState<string>();

	async function save(event: React.FormEvent) {
		event.preventDefault();
		if (saving) return;
		const chosenName = name.replace(/\s+/g, " ").trim();
		const chosen = username.trim();
		if (needsName && !chosenName) {
			setError(
				t("usernamePromptNameRequired", "Enter the name people know you by."),
			);
			return;
		}
		if (!chosen) {
			setError(t("usernamePromptRequired", "Enter a username."));
			return;
		}
		setSaving(true);
		setError(undefined);
		try {
			const update: IUserUpdate = { preferred_username: chosen };
			if (needsName) update.name = chosenName;
			await backend.userState.updateUser(update);
			await invalidateAccountIdentity(client);
		} catch (cause) {
			setError(
				apiErrorMessage(
					cause,
					t(
						"usernamePromptFailed",
						"Your username could not be saved. Try again.",
					),
				),
			);
		} finally {
			setSaving(false);
		}
	}

	return (
		<Dialog
			open
			onOpenChange={(open) => {
				if (!open && !saving) onPostpone();
			}}
		>
			<DialogContent className="sm:max-w-md">
				<form onSubmit={save} className="space-y-5">
					<DialogHeader>
						<DialogTitle>
							{t("usernamePromptTitle", "Choose your username")}
						</DialogTitle>
						<DialogDescription>
							{t(
								"usernamePromptDescription",
								"People find and mention you by your username. Your email address stays hidden.",
							)}
						</DialogDescription>
					</DialogHeader>
					{needsName && (
						<div className="space-y-2">
							<Label htmlFor="username-prompt-name">
								{t("displayName", "Display name")}
							</Label>
							<Input
								id="username-prompt-name"
								autoComplete="name"
								maxLength={96}
								disabled={saving}
								value={name}
								onChange={(event) => {
									setName(event.target.value);
									if (!usernameEdited)
										setUsername(handleSlug(event.target.value) ?? "");
								}}
							/>
						</div>
					)}
					<div className="space-y-2">
						<Label htmlFor="username-prompt-handle">
							{t("username2", "Username")}
						</Label>
						<Input
							id="username-prompt-handle"
							autoComplete="username"
							maxLength={30}
							disabled={saving}
							aria-describedby="username-prompt-help"
							value={username}
							onChange={(event) => {
								setUsername(event.target.value);
								setUsernameEdited(true);
							}}
						/>
						<p
							id="username-prompt-help"
							className="text-xs text-muted-foreground"
						>
							{t(
								"usernamePromptRules",
								"3 to 30 letters, digits, dots, hyphens or underscores. You can change it later in your account settings.",
							)}
						</p>
					</div>
					{error && (
						<p role="alert" className="text-sm text-destructive">
							{error}
						</p>
					)}
					<DialogFooter>
						<Button
							type="button"
							variant="ghost"
							disabled={saving}
							onClick={onPostpone}
						>
							{t("notNow", "Not now")}
						</Button>
						<Button type="submit" disabled={saving}>
							{saving && <Loader2 className="size-4 animate-spin" />}
							{t("save", "Save")}
						</Button>
					</DialogFooter>
				</form>
			</DialogContent>
		</Dialog>
	);
}
