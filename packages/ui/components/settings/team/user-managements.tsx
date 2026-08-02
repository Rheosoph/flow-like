"use client";

import {
	CrownIcon,
	FilterIcon,
	MoreVerticalIcon,
	SettingsIcon,
	ShieldIcon,
	Trash2Icon,
	UserXIcon,
	UsersIcon,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { toast } from "sonner";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
	AlertDialogTrigger,
	Avatar,
	AvatarFallback,
	AvatarImage,
	Button,
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	DialogTrigger,
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
	EmptyState,
	type IBackendRole,
	type IMember,
	Label,
	RolePermissions,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Skeleton,
	useBackend,
	useInfiniteInvoke,
	useInvalidateInvoke,
	useInvoke,
} from "../../../";
import {
	SectionHeading,
	StatusChip,
	TEAM_ROW_HANDLE,
	TEAM_ROW_META,
	TEAM_ROW_TITLE,
	TeamHint,
	TeamRowActions,
	TeamSearchInput,
	TeamSection,
	TeamToolbar,
	teamRowClass,
} from "./team-shared";

export function UserManagement({ appId }: Readonly<{ appId: string }>) {
	const backend = useBackend();
	const {
		data: team,
		hasNextPage,
		fetchNextPage,
		isFetchingNextPage,
		isLoading: isLoadingTeam,
	} = useInfiniteInvoke(backend.teamState.getTeam, backend.teamState, [appId]);
	const roles = useInvoke(backend.roleState.getRoles, backend.roleState, [
		appId,
	]);

	const [searchQuery, setSearchQuery] = useState("");
	const [roleFilter, setRoleFilter] = useState<string>("all");
	const [hiddenIds, setHiddenIds] = useState<ReadonlySet<string>>(new Set());

	const members = useMemo(() => team?.pages.flat() ?? [], [team]);
	const roleList = roles.data?.[1];

	const filteredTeam = useMemo(() => {
		if (roleFilter === "all") return members;
		return members.filter((member) => member.role_id === roleFilter);
	}, [members, roleFilter]);

	const reportMatch = useCallback((memberId: string, matches: boolean) => {
		setHiddenIds((previous) => {
			if (matches === !previous.has(memberId)) return previous;
			const next = new Set(previous);
			if (matches) next.delete(memberId);
			else next.add(memberId);
			return next;
		});
	}, []);

	const searchTerm = searchQuery.trim();

	const visibleCount = useMemo(
		() =>
			searchTerm.length === 0
				? filteredTeam.length
				: filteredTeam.filter((member) => !hiddenIds.has(member.id)).length,
		[filteredTeam, hiddenIds, searchTerm],
	);

	const isFiltering = searchTerm.length > 0 || roleFilter !== "all";
	const isInitialLoading = isLoadingTeam || roleList === undefined;

	return (
		<TeamSection>
			<SectionHeading
				icon={UsersIcon}
				title="People with access"
				count={members.length}
				description="Everyone who can open this app. Roles decide what they can change."
			/>

			<TeamToolbar>
				<TeamSearchInput
					value={searchQuery}
					onChange={setSearchQuery}
					placeholder="Search by name or handle…"
				/>
				<Select value={roleFilter} onValueChange={setRoleFilter}>
					<SelectTrigger className="h-9 w-40">
						<FilterIcon className="size-4 text-muted-foreground" />
						<SelectValue placeholder="Filter by role" />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="all">All roles</SelectItem>
						{roleList?.map((role) => (
							<SelectItem key={role.id} value={role.id}>
								{role.name}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			</TeamToolbar>

			<div className="flex flex-col gap-2">
				{isInitialLoading ? (
					<>
						<MemberRowSkeleton />
						<MemberRowSkeleton />
						<MemberRowSkeleton />
					</>
				) : (
					<>
						{filteredTeam.map((member) => (
							<Member
								key={member.id}
								member={member}
								roles={roleList ?? []}
								searchQuery={searchQuery}
								onMatchChange={reportMatch}
							/>
						))}

						{visibleCount === 0 && (
							<EmptyState
								className="max-w-full"
								title="No members found"
								description={
									isFiltering
										? "Try adjusting your search or filter criteria"
										: "No team members have been added yet"
								}
								icons={[UserXIcon]}
							/>
						)}
					</>
				)}

				{hasNextPage && (
					<Button
						variant="outline"
						className="w-full"
						onClick={() => fetchNextPage()}
						disabled={isFetchingNextPage}
					>
						{isFetchingNextPage ? "Loading..." : "Load More Members"}
					</Button>
				)}
			</div>

			{members.length > 0 && (
				<TeamHint>
					{`Showing ${visibleCount} of ${members.length} loaded ${
						members.length === 1 ? "member" : "members"
					}${hasNextPage ? " · more can be loaded" : ""}`}
				</TeamHint>
			)}
		</TeamSection>
	);
}

function MemberRowSkeleton() {
	return (
		<div className={teamRowClass()}>
			<Skeleton className="size-9 shrink-0 rounded-full" />
			<div className="min-w-0 flex-1 space-y-2">
				<Skeleton className="h-4 w-45" />
				<Skeleton className="h-3 w-30" />
			</div>
		</div>
	);
}

function Member({
	member,
	roles,
	searchQuery,
	onMatchChange,
}: Readonly<{
	member: IMember;
	roles: IBackendRole[];
	searchQuery: string;
	onMatchChange: (memberId: string, matches: boolean) => void;
}>) {
	const invalidate = useInvalidateInvoke();
	const userRole = roles.find((role) => role.id === member.role_id);
	const permission = new RolePermissions(userRole?.permissions ?? 0);
	const isOwner = permission.contains(RolePermissions.Owner);
	const backend = useBackend();
	const user = useInvoke(backend.userState.lookupUser, backend.userState, [
		member.user_id,
	]);
	const userData = user.data;

	const [isChangeRoleOpen, setIsChangeRoleOpen] = useState(false);
	const [selectedRoleId, setSelectedRoleId] = useState(member.role_id);

	const matches = useMemo(() => {
		const query = searchQuery.trim().toLowerCase();
		if (query.length === 0) return true;
		if (!userData) return true;
		return [
			userData?.name,
			userData?.preferred_username,
			userData?.username,
			userData?.email,
			userRole?.name,
		]
			.filter((value): value is string => Boolean(value))
			.some((value) => value.toLowerCase().includes(query));
	}, [searchQuery, userData, userRole]);

	useEffect(() => {
		onMatchChange(member.id, matches);
	}, [member.id, matches, onMatchChange]);

	const handleChangeRole = useCallback(
		async (roleId: string) => {
			if (!userRole) return;
			if (roleId === member.role_id) return;
			await backend.roleState.assignRole(
				userRole.app_id,
				roleId,
				member.user_id,
			);
			invalidate(backend.teamState.getTeam, [userRole.app_id]);
			setIsChangeRoleOpen(false);
		},
		[member.role_id, member.user_id, backend, userRole, invalidate],
	);

	const handleRemoveMember = useCallback(async () => {
		if (!userRole) return;
		await backend.teamState.removeUser(userRole.app_id, member.user_id);
		invalidate(backend.teamState.getTeam, [userRole.app_id]);
		toast.success(
			`${userData?.username ?? "User"} has been removed from the team.`,
		);
	}, [member.user_id, backend, userRole, userData, invalidate]);

	if (!matches) return null;

	if (!userData) return <MemberRowSkeleton />;

	const evaluatedName =
		userData.name ??
		userData.preferred_username ??
		userData.username ??
		userData.email ??
		"Unknown User";
	const handle =
		userData.preferred_username ?? userData.username ?? userData.email;
	const roleName = userRole?.name ?? "No Role Assigned";

	return (
		<div className={teamRowClass()}>
			<Avatar className="size-9 shrink-0">
				<AvatarImage src={userData.avatar_url} alt={evaluatedName} />
				<AvatarFallback className="text-[11px] font-semibold text-foreground">
					{evaluatedName
						.split(" ")
						.map((n) => n[0])
						.join("")}
				</AvatarFallback>
			</Avatar>

			<div className="min-w-0 flex-1">
				<div className={TEAM_ROW_TITLE}>
					<a
						href={`/profile?sub=${userData.id}`}
						className="truncate hover:underline"
					>
						{evaluatedName}
					</a>
					{handle && <span className={TEAM_ROW_HANDLE}>@{handle}</span>}
				</div>
				<div className={TEAM_ROW_META}>
					{isOwner ? (
						<StatusChip tone="owner" icon={CrownIcon}>
							{roleName}
						</StatusChip>
					) : (
						<StatusChip icon={ShieldIcon}>{roleName}</StatusChip>
					)}
				</div>
			</div>

			{!isOwner && (
				<TeamRowActions>
					<DropdownMenu>
						<DropdownMenuTrigger asChild>
							<Button variant="ghost" size="icon" className="size-8">
								<MoreVerticalIcon className="size-4" />
							</Button>
						</DropdownMenuTrigger>
						<DropdownMenuContent align="end">
							<Dialog
								open={isChangeRoleOpen}
								onOpenChange={setIsChangeRoleOpen}
							>
								<DialogTrigger asChild>
									<DropdownMenuItem onSelect={(e) => e.preventDefault()}>
										<SettingsIcon className="size-4" />
										Change Role
									</DropdownMenuItem>
								</DialogTrigger>
								<DialogContent>
									<DialogHeader>
										<DialogTitle>Change Role</DialogTitle>
										<DialogDescription>
											Select a new role for {evaluatedName}
										</DialogDescription>
									</DialogHeader>
									<div className="space-y-4 py-4">
										<div className="space-y-2">
											<Label htmlFor="role">Role</Label>
											<Select
												value={selectedRoleId}
												onValueChange={setSelectedRoleId}
											>
												<SelectTrigger>
													<SelectValue />
												</SelectTrigger>
												<SelectContent>
													{roles.map((role) => (
														<SelectItem key={role.id} value={role.id}>
															<div className="flex items-center gap-2">
																{role.name}
															</div>
														</SelectItem>
													))}
												</SelectContent>
											</Select>
										</div>
									</div>
									<DialogFooter>
										<Button
											variant="outline"
											onClick={() => setIsChangeRoleOpen(false)}
										>
											Cancel
										</Button>
										<Button
											onClick={async () => {
												await handleChangeRole(selectedRoleId);
											}}
										>
											Save Changes
										</Button>
									</DialogFooter>
								</DialogContent>
							</Dialog>
							<DropdownMenuSeparator />
							<AlertDialog>
								<AlertDialogTrigger asChild>
									<DropdownMenuItem
										variant="destructive"
										onSelect={(e) => e.preventDefault()}
									>
										<Trash2Icon className="size-4" />
										Remove
									</DropdownMenuItem>
								</AlertDialogTrigger>
								<AlertDialogContent>
									<AlertDialogHeader>
										<AlertDialogTitle>Remove Team Member</AlertDialogTitle>
										<AlertDialogDescription>
											Are you sure you want to remove {evaluatedName} from the
											team? This action cannot be undone.
										</AlertDialogDescription>
									</AlertDialogHeader>
									<AlertDialogFooter>
										<AlertDialogCancel>Cancel</AlertDialogCancel>
										<AlertDialogAction
											className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
											onClick={handleRemoveMember}
										>
											Remove
										</AlertDialogAction>
									</AlertDialogFooter>
								</AlertDialogContent>
							</AlertDialog>
						</DropdownMenuContent>
					</DropdownMenu>
				</TeamRowActions>
			)}
		</div>
	);
}
