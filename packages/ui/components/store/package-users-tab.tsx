"use client";

import { useTranslation } from "@flow-like/locales";
import { Shield, Trash2, UserPlus } from "lucide-react";
import { useCallback, useState } from "react";
import {
	PackagePermissionBits,
	isMaintainer,
	isOwner,
} from "../../lib/permission/wasm-package-permission";
import type {
	InviteUserRequest,
	PackageUser,
	UpdateUserPermissionRequest,
} from "../../lib/schema/wasm";
import {
	userAvatarUrl,
	userDisplayName,
	userInitials,
	userSecondaryLabel,
} from "../../lib/user-display";
import {
	Badge,
	Button,
	RelativeTime,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Skeleton,
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "../ui";
import { PackageInviteDialog } from "./package-invite-dialog";

interface PackageUsersTabProps {
	packageId: string;
	users: PackageUser[];
	currentUserPermission: number;
	isLoading: boolean;
	onInvite: (request: InviteUserRequest) => void;
	onUpdatePermission: (
		userId: string,
		request: UpdateUserPermissionRequest,
	) => void;
	onRemoveUser: (userId: string) => void;
	isMutating: boolean;
}

function roleBadgeVariant(
	permission: number,
): "default" | "secondary" | "outline" {
	if (isOwner(permission)) return "default";
	if (isMaintainer(permission)) return "secondary";
	return "outline";
}

function UserAvatar({ user }: { user: PackageUser }) {
	const avatar = userAvatarUrl(user);
	if (avatar) {
		return (
			<img
				src={avatar}
				alt={userDisplayName(user, user.userId)}
				className="h-8 w-8 rounded-full object-cover"
			/>
		);
	}

	return (
		<div className="flex h-8 w-8 items-center justify-center rounded-full bg-muted text-xs font-medium uppercase">
			{userInitials(user)}
		</div>
	);
}

function canManageUser(
	callerPermission: number,
	targetPermission: number,
): boolean {
	if (!isMaintainer(callerPermission)) return false;
	if (isOwner(targetPermission) && !isOwner(callerPermission)) return false;
	return true;
}

function UserActions({
	user,
	callerPermission,
	onUpdatePermission,
	onRemoveUser,
	isMutating,
}: {
	user: PackageUser;
	callerPermission: number;
	onUpdatePermission: (
		userId: string,
		request: UpdateUserPermissionRequest,
	) => void;
	onRemoveUser: (userId: string) => void;
	isMutating: boolean;
}) {
	const { t } = useTranslation("store");
	const manageable = canManageUser(callerPermission, user.permission);
	if (!manageable)
		return <span className="text-xs text-muted-foreground">—</span>;

	const callerIsOwner = isOwner(callerPermission);

	const handleRoleChange = useCallback(
		(value: string) => {
			let permission: number;
			switch (value) {
				case "owner":
					permission = PackagePermissionBits.Owner;
					break;
				case "maintainer":
					permission = PackagePermissionBits.Maintainer;
					break;
				default:
					permission = PackagePermissionBits.User;
			}
			onUpdatePermission(user.userId, { permission });
		},
		[user.userId, onUpdatePermission],
	);

	const currentLevel = isOwner(user.permission)
		? "owner"
		: isMaintainer(user.permission)
			? "maintainer"
			: "user";

	return (
		<div className="flex items-center gap-2">
			<Select
				value={currentLevel}
				onValueChange={handleRoleChange}
				disabled={isMutating}
			>
				<SelectTrigger className="h-8 w-[130px]">
					<SelectValue />
				</SelectTrigger>
				<SelectContent>
					{callerIsOwner && (
						<SelectItem value="owner">{t("owner", "Owner")}</SelectItem>
					)}
					<SelectItem value="maintainer">
						{t("maintainer", "Maintainer")}
					</SelectItem>
					<SelectItem value="user">{t("user", "User")}</SelectItem>
				</SelectContent>
			</Select>

			<TooltipProvider>
				<Tooltip>
					<TooltipTrigger asChild>
						<Button
							variant="ghost"
							size="icon"
							className="h-8 w-8 text-destructive"
							onClick={() => onRemoveUser(user.userId)}
							disabled={isMutating}
						>
							<Trash2 className="h-4 w-4" />
						</Button>
					</TooltipTrigger>
					<TooltipContent>{t("removeUser", "Remove user")}</TooltipContent>
				</Tooltip>
			</TooltipProvider>
		</div>
	);
}

function LoadingSkeleton() {
	return (
		<div className="space-y-3">
			{Array.from({ length: 3 }).map((_, i) => (
				<div key={i} className="flex items-center gap-3">
					<Skeleton className="h-8 w-8 rounded-full" />
					<Skeleton className="h-4 w-32" />
					<Skeleton className="ml-auto h-4 w-20" />
				</div>
			))}
		</div>
	);
}

export function PackageUsersTab({
	packageId,
	users,
	currentUserPermission,
	isLoading,
	onInvite,
	onUpdatePermission,
	onRemoveUser,
	isMutating,
}: PackageUsersTabProps) {
	const { t } = useTranslation("store");
	const [inviteOpen, setInviteOpen] = useState(false);
	const canInvite = isMaintainer(currentUserPermission);

	if (isLoading) return <LoadingSkeleton />;

	return (
		<div className="space-y-4">
			<div className="flex items-center justify-between">
				<div className="flex items-center gap-2">
					<Shield className="h-5 w-5 text-muted-foreground" />
					<h3 className="text-sm font-medium">
						{t("userCount", {
							count: users.length,
							defaultValue_one: "{{count}} user",
							defaultValue_other: "{{count}} users",
						})}
					</h3>
				</div>
				{canInvite && (
					<Button
						size="sm"
						variant="outline"
						onClick={() => setInviteOpen(true)}
					>
						<UserPlus className="mr-2 h-4 w-4" />
						{t("inviteUser", "Invite User")}
					</Button>
				)}
			</div>

			{users.length === 0 ? (
				<p className="py-8 text-center text-sm text-muted-foreground">
					{t("noUsersFoundForThisPackage", "No users found for this package.")}
				</p>
			) : (
				<Table>
					<TableHeader>
						<TableRow>
							<TableHead>{t("user", "User")}</TableHead>
							<TableHead>{t("role", "Role")}</TableHead>
							<TableHead>{t("added", "Added")}</TableHead>
							<TableHead className="text-right">
								{t("actions", "Actions")}
							</TableHead>
						</TableRow>
					</TableHeader>
					<TableBody>
						{users.map((user) => {
							const displayName = userDisplayName(user, user.userId);
							const secondary = userSecondaryLabel(user);
							const secondaryLabel =
								secondary === `@${displayName}` ? undefined : secondary;

							return (
								<TableRow key={user.id}>
									<TableCell>
										<div className="flex items-center gap-3">
											<UserAvatar user={user} />
											<div className="min-w-0">
												<p className="truncate text-sm font-medium">
													{displayName}
												</p>
												{secondaryLabel && (
													<p className="truncate text-xs text-muted-foreground">
														{secondaryLabel}
													</p>
												)}
											</div>
										</div>
									</TableCell>
									<TableCell>
										<Badge variant={roleBadgeVariant(user.permission)}>
											{isOwner(user.permission)
												? t("owner", "Owner")
												: isMaintainer(user.permission)
													? t("maintainer", "Maintainer")
													: t("user", "User")}
										</Badge>
									</TableCell>
									<TableCell className="text-sm text-muted-foreground">
										<RelativeTime fallback="—" value={user.grantedAt} />
									</TableCell>
									<TableCell className="text-right">
										<UserActions
											user={user}
											callerPermission={currentUserPermission}
											onUpdatePermission={onUpdatePermission}
											onRemoveUser={onRemoveUser}
											isMutating={isMutating}
										/>
									</TableCell>
								</TableRow>
							);
						})}
					</TableBody>
				</Table>
			)}

			<PackageInviteDialog
				open={inviteOpen}
				onOpenChange={setInviteOpen}
				onInvite={(req) => {
					onInvite(req);
					setInviteOpen(false);
				}}
				isSubmitting={isMutating}
			/>
		</div>
	);
}
