"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Crown,
	LogOut,
	MoreHorizontal,
	RefreshCw,
	Send,
	UserMinus,
	UserPlus,
	Users,
	X,
} from "lucide-react";
import { useMemo, useState } from "react";
import { toast } from "sonner";
import {
	type AssignableRole,
	type PackageRole,
	assignableRoles,
	canWithdrawInvitation,
	packageRole,
	personActions,
	roleAfterTransfer,
	roleBits,
} from "../../lib/package-people";
import { PackagePermissionBits } from "../../lib/permission/wasm-package-permission";
import type {
	PackagePendingInvitation,
	PackageUser,
} from "../../lib/schema/wasm";
import {
	type UserDisplayLike,
	userAvatarUrl,
	userDisplayName,
	userInitials,
	userSecondaryLabel,
} from "../../lib/user-display";
import { cn } from "../../lib/utils";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
	Badge,
	Button,
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
	RelativeTime,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Skeleton,
} from "../ui";
import { UserAvatar } from "../ui/user-identity";
import { PackageInviteDialog } from "./package-invite-dialog";
import type { PackageAccessModel } from "./package-workspace/use-package-access";
import {
	CountBadge,
	WorkspaceSection,
} from "./package-workspace/workspace-parts";

type Confirm =
	| { kind: "transfer"; person: PackageUser; name: string }
	| { kind: "remove"; person: PackageUser; name: string }
	| { kind: "leave"; person: PackageUser; name: string };

function useRoleLabel() {
	const { t } = useTranslation("store");
	return (role: PackageRole) =>
		({
			owner: t("owner", "Owner"),
			maintainer: t("maintainer", "Maintainer"),
			user: t("user", "User"),
			buyer: t("packageRoleBuyer", "Buyer"),
		})[role];
}

function RoleBadge({ role }: Readonly<{ role: PackageRole }>) {
	const label = useRoleLabel();
	return (
		<Badge
			variant={
				role === "owner"
					? "default"
					: role === "maintainer"
						? "secondary"
						: "outline"
			}
			className="gap-1"
		>
			{role === "owner" && <Crown className="size-3" />}
			{label(role)}
		</Badge>
	);
}

function PersonIdentity({
	person,
	fallbackId,
	detail,
	isSelf,
	muted,
}: Readonly<{
	person: UserDisplayLike;
	fallbackId: string;
	detail?: React.ReactNode;
	isSelf?: boolean;
	muted?: boolean;
}>) {
	const { t } = useTranslation("store");
	const name = userDisplayName(person, fallbackId);
	const secondary = userSecondaryLabel(person);
	return (
		<div className="flex min-w-0 items-center gap-3">
			<UserAvatar
				avatarUrl={userAvatarUrl(person)}
				initials={userInitials(person)}
				label={name}
				className={cn(muted && "opacity-70")}
			/>
			<div className="min-w-0">
				<p className="flex min-w-0 items-center gap-1.5 text-sm font-medium">
					<span className="truncate" title={name}>
						{name}
					</span>
					{isSelf && (
						<span className="shrink-0 rounded-full bg-muted px-1.5 text-[11px] font-normal leading-4.5 text-muted-foreground">
							{t("packagePeopleYou", "You")}
						</span>
					)}
				</p>
				{(detail ?? secondary) && (
					<p className="truncate text-xs text-muted-foreground">
						{detail ?? secondary}
					</p>
				)}
			</div>
		</div>
	);
}

function MemberRow({
	person,
	callerPermission,
	isSelf,
	busy,
	onRole,
	onConfirm,
}: Readonly<{
	person: PackageUser;
	callerPermission: number;
	isSelf: boolean;
	busy: boolean;
	onRole: (person: PackageUser, role: AssignableRole) => void;
	onConfirm: (confirm: Confirm) => void;
}>) {
	const { t } = useTranslation("store");
	const roleLabel = useRoleLabel();
	const role = packageRole(person.permission);
	const name = userDisplayName(person, person.userId);
	const actions = personActions(callerPermission, person.permission, isSelf);
	const hasMenu = actions.canTransfer || actions.canRemove || actions.canLeave;
	const secondary = userSecondaryLabel(person);

	return (
		<li className="flex flex-wrap items-center gap-x-3 gap-y-2 py-2.5">
			<div className="min-w-0 flex-1 basis-56">
				<PersonIdentity
					person={person}
					fallbackId={person.userId}
					isSelf={isSelf}
					detail={
						<>
							{secondary && <span>{secondary} · </span>}
							<RelativeTime
								value={person.grantedAt}
								fallback="—"
								className="whitespace-nowrap"
							/>
						</>
					}
				/>
			</div>
			<div className="ml-auto flex shrink-0 items-center gap-1.5">
				{actions.roleOptions.length > 0 &&
				(role === "maintainer" || role === "user") ? (
					<Select
						value={role}
						onValueChange={(next) => onRole(person, next as AssignableRole)}
						disabled={busy}
					>
						<SelectTrigger
							className="h-8 w-34"
							aria-label={t("packagePersonRole", {
								defaultValue: "Role of {{name}}",
								name,
							})}
						>
							<SelectValue />
						</SelectTrigger>
						<SelectContent>
							{actions.roleOptions.map((option) => (
								<SelectItem key={option} value={option}>
									{roleLabel(option)}
								</SelectItem>
							))}
						</SelectContent>
					</Select>
				) : (
					<RoleBadge role={role} />
				)}
				{hasMenu ? (
					<DropdownMenu>
						<DropdownMenuTrigger asChild>
							<Button
								variant="ghost"
								size="icon"
								className="size-8"
								disabled={busy}
								aria-label={t("packagePersonActions", {
									defaultValue: "Actions for {{name}}",
									name,
								})}
							>
								<MoreHorizontal className="size-4" />
							</Button>
						</DropdownMenuTrigger>
						<DropdownMenuContent align="end">
							{actions.canTransfer && (
								<DropdownMenuItem
									onSelect={() => onConfirm({ kind: "transfer", person, name })}
								>
									<Crown className="size-4" />
									{t("packageMakeOwner", "Make owner…")}
								</DropdownMenuItem>
							)}
							{actions.canTransfer && actions.canRemove && (
								<DropdownMenuSeparator />
							)}
							{actions.canRemove && (
								<DropdownMenuItem
									variant="destructive"
									onSelect={() => onConfirm({ kind: "remove", person, name })}
								>
									<UserMinus className="size-4" />
									{t("packageRemovePerson", "Remove from package…")}
								</DropdownMenuItem>
							)}
							{actions.canLeave && (
								<DropdownMenuItem
									variant="destructive"
									onSelect={() => onConfirm({ kind: "leave", person, name })}
								>
									<LogOut className="size-4" />
									{t("packageLeave", "Leave package…")}
								</DropdownMenuItem>
							)}
						</DropdownMenuContent>
					</DropdownMenu>
				) : (
					<span className="size-8" aria-hidden />
				)}
			</div>
		</li>
	);
}

function InvitationRow({
	invitation,
	callerPermission,
	busy,
	onResend,
	onWithdraw,
}: Readonly<{
	invitation: PackagePendingInvitation;
	callerPermission: number;
	busy: boolean;
	onResend: (invitation: PackagePendingInvitation) => void;
	onWithdraw: (invitation: PackagePendingInvitation) => void;
}>) {
	const { t } = useTranslation("store");
	const name = userDisplayName(invitation, invitation.inviteeId);
	const manageable = canWithdrawInvitation(
		callerPermission,
		invitation.permission,
	);

	return (
		<li className="flex flex-wrap items-center gap-x-3 gap-y-2 py-2.5">
			<div className="min-w-0 flex-1 basis-56">
				<PersonIdentity
					person={invitation}
					fallbackId={invitation.inviteeId}
					muted
					detail={
						<>
							{invitation.invitedByName && (
								<span>
									{t("packageInvitedBy", {
										defaultValue: "Invited by {{name}}",
										name: invitation.invitedByName,
									})}
									{" · "}
								</span>
							)}
							{invitation.expiresAt ? (
								<span className="whitespace-nowrap">
									{t("packageInvitationExpires", "expires")}{" "}
									<RelativeTime value={invitation.expiresAt} />
								</span>
							) : (
								<RelativeTime
									value={invitation.createdAt}
									className="whitespace-nowrap"
								/>
							)}
						</>
					}
				/>
			</div>
			<div className="ml-auto flex shrink-0 items-center gap-1.5">
				<Badge
					variant="outline"
					className="border-dashed text-muted-foreground"
				>
					{t("packageInvitationPending", "Pending")}
				</Badge>
				<RoleBadge role={packageRole(invitation.permission)} />
				{manageable && (
					<>
						<Button
							variant="ghost"
							size="icon"
							className="size-8"
							disabled={busy}
							onClick={() => onResend(invitation)}
							aria-label={t("packageResendInvitation", {
								defaultValue: "Send the invitation to {{name}} again",
								name,
							})}
							title={t("packageResend", "Send again")}
						>
							<Send className="size-4" />
						</Button>
						<Button
							variant="ghost"
							size="icon"
							className="size-8 text-muted-foreground hover:text-destructive"
							disabled={busy}
							onClick={() => onWithdraw(invitation)}
							aria-label={t("packageWithdrawInvitation", {
								defaultValue: "Withdraw the invitation to {{name}}",
								name,
							})}
							title={t("packageWithdraw", "Withdraw")}
						>
							<X className="size-4" />
						</Button>
					</>
				)}
			</div>
		</li>
	);
}

function ConfirmDialog({
	confirm,
	busy,
	onCancel,
	onConfirm,
}: Readonly<{
	confirm: Confirm | null;
	busy: boolean;
	onCancel: () => void;
	onConfirm: (confirm: Confirm) => void;
}>) {
	const { t } = useTranslation("store");
	const roleLabel = useRoleLabel();
	if (!confirm) return null;

	const copy = {
		transfer: {
			title: t("packageTransferTitle", {
				defaultValue: "Make {{name}} the owner?",
				name: confirm.name,
			}),
			description: t("packageTransferDescription", {
				defaultValue:
					"A package has one owner. {{name}} gets full control, including deleting the package and transferring it again. You stay on the package as {{role}}.",
				name: confirm.name,
				role: roleLabel(roleAfterTransfer(confirm.person.permission)),
			}),
			action: t("packageTransferAction", "Transfer ownership"),
		},
		remove: {
			title: t("packageRemoveTitle", {
				defaultValue: "Remove {{name}}?",
				name: confirm.name,
			}),
			description: t(
				"packageRemoveDescription",
				"They lose access to this package right away. Projects that use the package through them lose it too, unless someone else there has access.",
			),
			action: t("packageRemoveAction", "Remove"),
		},
		leave: {
			title: t("packageLeaveTitle", "Leave this package?"),
			description: t(
				"packageLeaveDescription",
				"You lose access right away. A maintainer or the owner has to invite you again to get it back.",
			),
			action: t("packageLeaveAction", "Leave"),
		},
	}[confirm.kind];

	return (
		<AlertDialog open onOpenChange={(open) => !open && !busy && onCancel()}>
			<AlertDialogContent>
				<AlertDialogHeader>
					<AlertDialogTitle>{copy.title}</AlertDialogTitle>
					<AlertDialogDescription>{copy.description}</AlertDialogDescription>
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogCancel disabled={busy}>
						{t("cancel", "Cancel")}
					</AlertDialogCancel>
					<AlertDialogAction
						disabled={busy}
						className={cn(
							confirm.kind !== "transfer" &&
								"bg-destructive text-destructive-foreground hover:bg-destructive/90",
						)}
						onClick={(event) => {
							event.preventDefault();
							onConfirm(confirm);
						}}
					>
						{busy && <RefreshCw className="size-4 animate-spin" />}
						{copy.action}
					</AlertDialogAction>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}

function ListSkeleton() {
	return (
		<div className="space-y-3 py-1">
			{["a", "b", "c"].map((key) => (
				<div key={key} className="flex items-center gap-3">
					<Skeleton className="size-8 rounded-full" />
					<Skeleton className="h-4 w-40" />
					<Skeleton className="ml-auto h-6 w-20" />
				</div>
			))}
		</div>
	);
}

export function LoadError({
	message,
	onRetry,
}: Readonly<{ message: string; onRetry: () => void }>) {
	const { t } = useTranslation("store");
	return (
		<div className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm">
			<span className="text-destructive">{message}</span>
			<Button size="sm" variant="outline" onClick={onRetry}>
				<RefreshCw className="size-3.5" />
				{t("retry", "Retry")}
			</Button>
		</div>
	);
}

/** Members and pending invitations of a package, with the invite, role, transfer, remove and leave actions the API allows the caller. */
export function PackageUsersTab({
	access,
	callerPermission,
	selfId,
	onLeft,
}: Readonly<{
	access: PackageAccessModel;
	callerPermission: number;
	selfId?: string;
	onLeft: () => void;
}>) {
	const { t } = useTranslation("store");
	const [inviteOpen, setInviteOpen] = useState(false);
	const [confirm, setConfirm] = useState<Confirm | null>(null);
	const { users, invitations } = access;
	const roles = assignableRoles(callerPermission);

	const excludedIds = useMemo(
		() =>
			new Set([
				...users.list.map((person) => person.userId),
				...invitations.list.map((invitation) => invitation.inviteeId),
			]),
		[users.list, invitations.list],
	);

	const confirmBusy =
		access.transfer.isPending ||
		access.remove.isPending ||
		access.leave.isPending;
	const rowBusy = confirmBusy || access.setRole.isPending;

	const runConfirm = (next: Confirm) => {
		const close = () => setConfirm(null);
		if (next.kind === "transfer") {
			access.transfer.mutate(
				{ userId: next.person.userId, permission: PackagePermissionBits.Owner },
				{ onSettled: close },
			);
		} else if (next.kind === "remove") {
			access.remove.mutate(next.person.userId, { onSettled: close });
		} else {
			access.leave.mutate(next.person.userId, {
				onSuccess: onLeft,
				onSettled: close,
			});
		}
	};

	const count = users.list.length + invitations.list.length;

	return (
		<WorkspaceSection
			icon={Users}
			title={
				<span className="flex items-center gap-2">
					{t("packagePeople", "People")}
					{!users.isLoading && <CountBadge value={count} />}
				</span>
			}
			action={
				roles.length > 0 && (
					<Button
						size="sm"
						variant="outline"
						onClick={() => setInviteOpen(true)}
					>
						<UserPlus className="size-4" />
						{t("inviteUser", "Invite User")}
					</Button>
				)
			}
		>
			{users.isLoading ? (
				<ListSkeleton />
			) : users.isError ? (
				<LoadError
					message={t(
						"packagePeopleLoadFailed",
						"Couldn't load the people on this package.",
					)}
					onRetry={() => void users.refetch()}
				/>
			) : (
				<ul className="divide-y divide-border/60">
					{users.list.map((person) => (
						<MemberRow
							key={person.id}
							person={person}
							callerPermission={callerPermission}
							isSelf={person.userId === selfId}
							busy={rowBusy}
							onRole={(target, role) =>
								access.setRole.mutate({
									userId: target.userId,
									permission: roleBits(role),
								})
							}
							onConfirm={setConfirm}
						/>
					))}
				</ul>
			)}

			{invitations.isError ? (
				<div className="mt-3">
					<LoadError
						message={t(
							"packageInvitationsLoadFailed",
							"Couldn't load pending invitations.",
						)}
						onRetry={() => void invitations.refetch()}
					/>
				</div>
			) : (
				invitations.list.length > 0 && (
					<div className="mt-3 border-t border-border/60 pt-3">
						<h3 className="flex items-center gap-2 text-xs font-medium text-muted-foreground">
							{t("packagePendingInvitations", "Waiting for an answer")}
							<CountBadge value={invitations.list.length} />
						</h3>
						<ul className="divide-y divide-border/60">
							{invitations.list.map((invitation) => (
								<InvitationRow
									key={invitation.id}
									invitation={invitation}
									callerPermission={callerPermission}
									busy={access.withdraw.isPending || access.invite.isPending}
									onResend={(target) =>
										access.invite.mutate(
											{
												inviteeId: target.inviteeId,
												permission: target.permission,
											},
											{
												onSuccess: () =>
													toast.success(
														t(
															"packageInvitationResent",
															"Invitation sent again",
														),
													),
											},
										)
									}
									onWithdraw={(target) => access.withdraw.mutate(target.id)}
								/>
							))}
						</ul>
					</div>
				)
			)}

			<ConfirmDialog
				confirm={confirm}
				busy={confirmBusy}
				onCancel={() => setConfirm(null)}
				onConfirm={runConfirm}
			/>
			<PackageInviteDialog
				open={inviteOpen}
				onOpenChange={setInviteOpen}
				roles={roles}
				memberIds={excludedIds}
				onInvite={(request) =>
					access.invite.mutateAsync(request).then(
						() => {
							toast.success(t("invitationSent", "Invitation sent"));
							return true;
						},
						() => false,
					)
				}
			/>
		</WorkspaceSection>
	);
}
