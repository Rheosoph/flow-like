"use client";

import { useTranslation } from "@flow-like/locales";
import { Mail, RefreshCw } from "lucide-react";
import { useMemo, useState } from "react";
import type { ProjectUserSearch } from "../../hooks/use-project-user-search";
import { apiErrorMessage } from "../../lib/api-error";
import {
	userAvatarUrl,
	userDisplayName,
	userInitials,
	userSecondaryLabel,
} from "../../lib/user-display";
import type { IUserLookup } from "../../state/backend-state/types";
import { Avatar, AvatarFallback, AvatarImage } from "./avatar";
import { Button } from "./button";

/**
 * The result list and status lines under an invite search box, shared by every
 * surface that invites a person so a project and a package search alike.
 */
export function UserInviteSearchResults({
	search,
	query,
	excludeIds,
	onInvite,
}: Readonly<{
	search: ProjectUserSearch;
	query: string;
	/** People the target already has, e.g. current package members. */
	excludeIds?: ReadonlySet<string>;
	onInvite: (user: IUserLookup, displayName: string) => Promise<void>;
}>) {
	const { t } = useTranslation("settings");
	const [invitingId, setInvitingId] = useState<string | null>(null);
	const results = useMemo(
		() =>
			excludeIds
				? search.results.filter(({ user }) => !excludeIds.has(user.id))
				: search.results,
		[search.results, excludeIds],
	);

	const invite = async (user: IUserLookup, displayName: string) => {
		setInvitingId(user.id);
		try {
			await onInvite(user, displayName);
		} finally {
			setInvitingId(null);
		}
	};

	return (
		<>
			{results.length > 0 && (
				<div className="space-y-2">
					<h4 className="text-sm font-medium">
						{query.trim()
							? t("searchResults", "Search Results")
							: t("peopleFromYourProjects", "People from your projects")}
					</h4>
					<div className="max-h-60 space-y-2 overflow-y-auto pr-2">
						{results.map(({ user, fromProject }) => {
							const displayName = userDisplayName(user, user.id);
							const secondary = userSecondaryLabel(user);
							return (
								<div
									key={user.id}
									className="flex items-center justify-between gap-3 rounded-lg border bg-card p-3"
								>
									<div className="flex min-w-0 items-center gap-3">
										<Avatar className="h-9 w-9 shrink-0">
											<AvatarImage
												src={userAvatarUrl(user)}
												alt={displayName}
											/>
											<AvatarFallback className="bg-primary/10 text-primary">
												{userInitials(user)}
											</AvatarFallback>
										</Avatar>
										<div className="min-w-0 flex-1">
											<p className="truncate text-sm font-medium">
												{displayName}
											</p>
											{secondary && (
												<p className="truncate text-xs text-muted-foreground">
													{secondary}
												</p>
											)}
											{fromProject && (
												<p className="text-xs text-primary">
													{t("fromYourProjects", "From your projects")}
												</p>
											)}
										</div>
									</div>
									<Button
										size="sm"
										disabled={invitingId !== null}
										aria-label={t("inviteNamedUser", {
											defaultValue: "Invite {{name}}",
											name: displayName,
										})}
										onClick={() => void invite(user, displayName)}
										className="h-8 shrink-0 gap-1.5 text-xs"
									>
										{invitingId === user.id ? (
											<RefreshCw className="h-3 w-3 animate-spin" />
										) : (
											<Mail className="h-3 w-3" />
										)}
										{t("invite", "Invite")}
									</Button>
								</div>
							);
						})}
					</div>
				</div>
			)}
			<section
				aria-label={t("userSearchStatus", "User search status")}
				aria-live="polite"
				className="space-y-2 text-sm text-muted-foreground"
			>
				{search.isLoadingContacts && (
					<p className="flex items-center gap-2">
						<RefreshCw className="h-3 w-3 animate-spin" />
						{t(
							"loadingProjectContacts",
							"Loading people from your projects...",
						)}
					</p>
				)}
				{search.isSearchingDirectory && (
					<p className="flex items-center gap-2">
						<RefreshCw className="h-3 w-3 animate-spin" />
						{t("searchingDirectory", "Searching for more people...")}
					</p>
				)}
				{/* The server's own message tells a revoked role from a dropped
				    connection; "Retry" alone reads as a network blip. */}
				{search.contactsError && (
					<div className="flex items-center justify-between gap-2">
						<p>
							{apiErrorMessage(
								search.contactsError,
								t(
									"projectContactsFailed",
									"Could not load people from your projects.",
								),
							)}
						</p>
						<Button
							size="sm"
							variant="outline"
							onClick={() => search.retryContacts()}
						>
							{t("retry", "Retry")}
						</Button>
					</div>
				)}
				{search.directoryError && (
					<div className="flex items-center justify-between gap-2">
						<p>
							{apiErrorMessage(
								search.directoryError,
								t("userSearchFailed", "Could not search for users"),
							)}
						</p>
						<Button
							size="sm"
							variant="outline"
							onClick={() => search.retryDirectory()}
						>
							{t("retry", "Retry")}
						</Button>
					</div>
				)}
				{!search.canSearchDirectory && (
					<p>
						{t(
							"searchDirectoryHint",
							"Enter at least 2 characters to search for anyone by name, handle, email or user ID.",
						)}
					</p>
				)}
				{search.canSearchDirectory &&
					!search.isSearchingDirectory &&
					!search.isLoadingContacts &&
					!search.directoryError &&
					!search.contactsError &&
					results.length === 0 && (
						<p className="py-4 text-center">
							{t("noUsersFound", "No users found")}
						</p>
					)}
			</section>
		</>
	);
}
