"use client";

import { useTranslation } from "@flow-like/locales";
import { User } from "lucide-react";
import { useCallback, useState } from "react";
import { useProjectUserSearch } from "../../hooks/use-project-user-search";
import { PackagePermissionBits } from "../../lib/permission/wasm-package-permission";
import type { InviteUserRequest } from "../../lib/schema/wasm";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../ui/select";
import { Separator } from "../ui/separator";
import { UserInviteSearchResults } from "../ui/user-invite-search-results";

interface PackageInviteDialogProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	/** Resolves `true` once the invitation exists; the dialog then closes. */
	onInvite: (request: InviteUserRequest) => Promise<boolean>;
	memberIds: ReadonlySet<string>;
}

export function PackageInviteDialog({
	open,
	onOpenChange,
	onInvite,
	memberIds,
}: PackageInviteDialogProps) {
	const { t } = useTranslation("store");
	const [query, setQuery] = useState("");
	const [permissionLevel, setPermissionLevel] = useState<"maintainer" | "user">(
		"user",
	);
	const userSearch = useProjectUserSearch(undefined, query, open);

	const handleOpenChange = useCallback(
		(next: boolean) => {
			if (!next) {
				setQuery("");
				setPermissionLevel("user");
			}
			onOpenChange(next);
		},
		[onOpenChange],
	);

	const handleInvite = useCallback(
		async (inviteeId: string) => {
			const permission =
				permissionLevel === "maintainer"
					? PackagePermissionBits.Maintainer
					: PackagePermissionBits.User;
			if (await onInvite({ inviteeId, permission })) handleOpenChange(false);
		},
		[permissionLevel, onInvite, handleOpenChange],
	);

	return (
		<Dialog open={open} onOpenChange={handleOpenChange}>
			<DialogContent className="max-h-[90dvh] overflow-y-auto sm:max-w-md">
				<DialogHeader>
					<DialogTitle>{t("inviteUser", "Invite User")}</DialogTitle>
					<DialogDescription>
						{t(
							"searchForPeopleAndInviteThemToThisPackage",
							"Search for people and invite them to this package.",
						)}
					</DialogDescription>
				</DialogHeader>

				<div className="space-y-4 py-4">
					<div className="space-y-2">
						<Label htmlFor="package-invite-query">
							{t("settings:nameHandleOrEmail", "Name, handle or email")}
						</Label>
						<div className="relative">
							<Input
								id="package-invite-query"
								placeholder={t(
									"settings:searchByNameHandleEmailOrUserId",
									"Search by name, handle, email or user ID...",
								)}
								value={query}
								onChange={(e) => setQuery(e.target.value)}
								className="pl-10"
								maxLength={200}
								autoComplete="off"
							/>
							<User className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
						</div>
					</div>

					<div className="space-y-2">
						<Label htmlFor="invite-permission">
							{t("permissionLevel", "Permission Level")}
						</Label>
						<Select
							value={permissionLevel}
							onValueChange={(v) =>
								setPermissionLevel(v as "maintainer" | "user")
							}
						>
							<SelectTrigger id="invite-permission">
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="user">{t("user", "User")}</SelectItem>
								<SelectItem value="maintainer">
									{t("maintainer", "Maintainer")}
								</SelectItem>
							</SelectContent>
						</Select>
					</div>

					<div className="space-y-3">
						<Separator />
						<UserInviteSearchResults
							search={userSearch}
							query={query}
							excludeIds={memberIds}
							onInvite={(user) => handleInvite(user.id)}
						/>
					</div>
				</div>
			</DialogContent>
		</Dialog>
	);
}
