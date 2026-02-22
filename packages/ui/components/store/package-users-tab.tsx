"use client";

import { formatDistanceToNow } from "date-fns";
import {
	MoreHorizontal,
	Shield,
	Trash2,
	UserPlus,
} from "lucide-react";
import { useCallback, useState } from "react";
import type {
	InviteUserRequest,
	PackageUser,
	UpdateUserPermissionRequest,
} from "../../lib/schema/wasm";
import {
	PackagePermissionBits,
	isMaintainer,
	isOwner,
	permissionLabel,
} from "../../lib/permission/wasm-package-permission";
import {
	Badge,
	Button,
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
	if (user.avatar) {
		return (
			<img
				src={user.avatar}
				alt={user.username ?? user.userId}
				className="h-8 w-8 rounded-full object-cover"
			/>
		);
	}

	return (
		<div className="flex h-8 w-8 items-center justify-center rounded-full bg-muted text-xs font-medium uppercase">
			{(user.username ?? user.userId).charAt(0)}
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
	const manageable = canManageUser(callerPermission, user.permission);
	if (!manageable) return <span className="text-xs text-muted-foreground">—</span>;

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
					{callerIsOwner && <SelectItem value="owner">Owner</SelectItem>}
					<SelectItem value="maintainer">Maintainer</SelectItem>
					<SelectItem value="user">User</SelectItem>
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
					<TooltipContent>Remove user</TooltipContent>
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
	const [inviteOpen, setInviteOpen] = useState(false);
	const canInvite = isMaintainer(currentUserPermission);

	if (isLoading) return <LoadingSkeleton />;

	return (
		<div className="space-y-4">
			<div className="flex items-center justify-between">
				<div className="flex items-center gap-2">
					<Shield className="h-5 w-5 text-muted-foreground" />
					<h3 className="text-sm font-medium">
						{users.length} {users.length === 1 ? "user" : "users"}
					</h3>
				</div>
				{canInvite && (
					<Button
						size="sm"
						variant="outline"
						onClick={() => setInviteOpen(true)}
					>
						<UserPlus className="mr-2 h-4 w-4" />
						Invite User
					</Button>
				)}
			</div>

			{users.length === 0 ? (
				<p className="py-8 text-center text-sm text-muted-foreground">
					No users found for this package.
				</p>
			) : (
				<Table>
					<TableHeader>
						<TableRow>
							<TableHead>User</TableHead>
							<TableHead>Role</TableHead>
							<TableHead>Added</TableHead>
							<TableHead className="text-right">Actions</TableHead>
						</TableRow>
					</TableHeader>
					<TableBody>
						{users.map((user) => (
							<TableRow key={user.id}>
								<TableCell>
									<div className="flex items-center gap-3">
										<UserAvatar user={user} />
										<div className="min-w-0">
											<p className="truncate text-sm font-medium">
												{user.name ?? user.username ?? user.userId}
											</p>
											{user.username && user.name && (
												<p className="truncate text-xs text-muted-foreground">
													@{user.username}
												</p>
											)}
										</div>
									</div>
								</TableCell>
								<TableCell>
									<Badge variant={roleBadgeVariant(user.permission)}>
										{permissionLabel(user.permission)}
									</Badge>
								</TableCell>
								<TableCell className="text-sm text-muted-foreground">
									{formatDistanceToNow(new Date(user.grantedAt), {
										addSuffix: true,
									})}
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
						))}
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
