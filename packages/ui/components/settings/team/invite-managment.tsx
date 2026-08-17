"use client";

import { useTranslation } from "@flow-like/locales";
import { useDebounce } from "@uidotdev/usehooks";
import {
	ClockIcon,
	CopyIcon,
	ExternalLinkIcon,
	Link,
	LinkIcon,
	Mail,
	MailIcon,
	MoreVerticalIcon,
	PlusIcon,
	RefreshCw,
	SearchIcon,
	Settings,
	Trash2Icon,
	User,
	UserCheckIcon,
	UserPlus2Icon,
	UserPlusIcon,
	UserX,
	Users,
	UsersIcon,
} from "lucide-react";
import { type ReactNode, useCallback, useState } from "react";
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
	DropdownMenuTrigger,
	EmptyState,
	Input,
	Label,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Separator,
	Textarea,
	useBackend,
	useHub,
	useInvalidateInfiniteInvoke,
	useInvoke,
} from "../../../";
import { apiErrorMessage } from "../../../lib/api-error";
import {
	userAvatarUrl,
	userDisplayName,
	userInitials,
	userSecondaryLabel,
} from "../../../lib/user-display";
import {
	SectionHeading,
	TEAM_ACTION_GRADIENT,
	TEAM_ROW_META,
	TEAM_ROW_TITLE,
	TeamCallout,
	TeamHint,
	TeamRowActions,
	TeamRowIcon,
	TeamSection,
	teamRowClass,
} from "./team-shared";

export function InviteUserDialog({
	appId,
	trigger,
}: Readonly<{ appId: string; trigger: ReactNode }>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const invalidateInfinite = useInvalidateInfiniteInvoke();
	const [message, setMessage] = useState("");
	const [invitee, setInvitee] = useState("");
	const inviteeSearch = useDebounce(invitee.trim(), 350);
	const [showInviteDialog, setShowInviteDialog] = useState(false);

	// One character matches most of the directory, so it is not worth a round trip.
	const canSearch = inviteeSearch.length >= 2;
	const userSearch = useInvoke(
		backend.userState.searchUsers,
		backend.userState,
		[inviteeSearch],
		canSearch,
	);

	return (
		<Dialog open={showInviteDialog} onOpenChange={setShowInviteDialog}>
			<DialogTrigger asChild>{trigger}</DialogTrigger>
			<DialogContent className="sm:max-w-md">
				<DialogHeader className="space-y-3">
					<div className="mx-auto flex h-12 w-12 items-center justify-center rounded-full bg-primary/10">
						<UserPlus2Icon className="h-6 w-6 text-primary" />
					</div>
					<DialogTitle className="text-center text-xl">
						{t('inviteNewMember', 'Invite New Member')}
					</DialogTitle>
					<DialogDescription className="text-center">
						{t('searchForUsersAndSendThemAnInvitationToJoinYourTeam', 'Search for users and send them an invitation to join your team')}
					</DialogDescription>
				</DialogHeader>

				<div className="space-y-4 py-4">
					<div className="space-y-2">
						<Label htmlFor="usernameOrEmail" className="text-sm font-medium">
							{t('nameHandleOrEmail', 'Name, handle or email')}
						</Label>
						<div className="relative">
							<Input
								id="usernameOrEmail"
								placeholder={t('searchByNameHandleEmailOrUserId', 'Search by name, handle, email or user ID...')}
								value={invitee}
								onChange={(e) => setInvitee(e.target.value)}
								className="pl-10"
							/>
							<User className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
						</div>
					</div>

					<div className="space-y-2">
						<Label htmlFor="inviteMessage" className="text-sm font-medium">
							{t('personalMessage', 'Personal Message')}
						</Label>
						<Textarea
							id="inviteMessage"
							placeholder={t('addAPersonalMessageToYourInvitationOptional', 'Add a personal message to your invitation (optional)')}
							value={message}
							onChange={(e) => setMessage(e.target.value)}
							className="min-h-20 resize-none"
						/>
					</div>

					{canSearch && (
						<div className="space-y-3">
							<Separator />

							{userSearch.isFetching && (
								<div className="flex items-center justify-center gap-2 py-8 text-muted-foreground">
									<RefreshCw className="h-4 w-4 animate-spin" />
									<span className="text-sm">{t('searchingForUsers', 'Searching for users...')}</span>
								</div>
							)}

							{!userSearch.isFetching &&
								userSearch.data &&
								userSearch.data.length > 0 && (
									<div className="space-y-2">
										<h4 className="text-sm font-medium text-foreground">
											{t('searchResults', 'Search Results')}
										</h4>
										<div className="max-h-48 space-y-2 overflow-y-auto pr-2">
											{userSearch.data.map((user) => {
												const displayName = userDisplayName(user, user.id);
												const secondary = userSecondaryLabel(user);
												return (
													<div
														key={user.id}
														className="group flex items-center justify-between gap-3 rounded-lg border bg-card p-3 transition-colors hover:bg-accent/50"
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
															</div>
														</div>
														<Button
															size="sm"
															onClick={async () => {
																try {
																	await backend.teamState.inviteUser(
																		appId,
																		user.id,
																		message,
																	);
																	// Surface the new invitation in the people
																	// list without waiting for a manual refresh.
																	await invalidateInfinite(
																		backend.teamState.getAppInvites,
																		[appId],
																	);
																	toast.success(
																		`Invitation sent to ${displayName}!`,
																	);
																	setShowInviteDialog(false);
																	setInvitee("");
																	setMessage("");
																} catch (error) {
																	console.error(error);
																	toast.error(
																		apiErrorMessage(
																			error,
																			"Failed to send invite. Please try again.",
																		),
																	);
																}
															}}
															className="h-8 shrink-0 gap-1.5 text-xs"
														>
															<Mail className="h-3 w-3" />
															{t('invite', 'Invite')}
														</Button>
													</div>
												);
											})}
										</div>
									</div>
								)}

							{!userSearch.isFetching &&
								(!userSearch.data || userSearch.data.length === 0) && (
									<div className="flex flex-col items-center gap-2 py-8 text-center">
										<div className="flex h-12 w-12 items-center justify-center rounded-full bg-muted">
											<UserX className="h-6 w-6 text-muted-foreground" />
										</div>
										<div className="space-y-1">
											<p className="text-sm font-medium">{t('noUsersFound', 'No users found')}</p>
											<p className="text-xs text-muted-foreground">
												{t('searchByNameHandleEmailOrUserId2', 'Search by name, handle, email or user ID')}
											</p>
										</div>
									</div>
								)}
						</div>
					)}

					{!canSearch && (
						<div className="flex flex-col items-center gap-2 py-6 text-center text-muted-foreground">
							<Users className="h-8 w-8" />
							<p className="text-sm">
								{t('keepTypingAtLeast2Characters', { defaultValue_zero: 'Start typing to search for users', defaultValue_other: 'Keep typing — at least 2 characters', count: inviteeSearch.length })}
							</p>
						</div>
					)}
				</div>
			</DialogContent>
		</Dialog>
	);
}

export function InviteManagement({ appId }: Readonly<{ appId: string }>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const links = useInvoke(backend.teamState.getInviteLinks, backend.teamState, [
		appId,
	]);
	const [showCreateLinkDialog, setShowCreateLinkDialog] = useState(false);
	const [newLinkName, setNewLinkName] = useState("");
	const [newLinkMaxUses, setNewLinkMaxUses] = useState<string>("");
	const [newLinkExpiry, setNewLinkExpiry] = useState<string>("168");
	const { hub } = useHub();

	const host = hub?.app ?? "app.flow-like.com";

	const webLink = useCallback(
		(token: string) => `https://${host}/join?appId=${appId}&token=${token}`,
		[host, appId],
	);

	const copyInviteLink = (token: string) => {
		navigator.clipboard.writeText(token);
		toast.success("Invite link copied to clipboard!");
	};

	const createInviteLink = useCallback(async () => {
		let maxUses: number | undefined = Number.parseInt(newLinkMaxUses);
		if (Number.isNaN(maxUses) || maxUses <= 0) {
			maxUses = -1; // Allow unlimited uses if not specified
		}

		const expiresInHours = Number.parseInt(newLinkExpiry);
		await backend.teamState.createInviteLink(
			appId,
			newLinkName,
			maxUses,
			Number.isNaN(expiresInHours) || expiresInHours <= 0
				? undefined
				: expiresInHours,
		);
		setNewLinkName("");
		setNewLinkMaxUses("");
		setNewLinkExpiry("168");
		setShowCreateLinkDialog(false);
		toast.success("New invite link created!");
		await links.refetch();
	}, [
		appId,
		newLinkName,
		newLinkMaxUses,
		newLinkExpiry,
		backend,
		links.refetch,
	]);

	const deleteInviteLink = useCallback(
		async (id: string) => {
			await backend.teamState.removeInviteLink(appId, id);
			await links.refetch();
		},
		[backend, links.refetch, appId],
	);

	return (
		<div className="space-y-8">
			<TeamSection>
				<SectionHeading
					icon={UserPlusIcon}
					title={t('inviteSomeoneDirectly', 'Invite someone directly')}
					description={t('searchAFlowlikeAccountAndSendItAnInvitationWithANote', 'Search a Flow-Like account and send it an invitation with a note.')}
					actions={
						<InviteUserDialog
							appId={appId}
							trigger={
								<Button size="sm" className={TEAM_ACTION_GRADIENT}>
									<UserPlusIcon className="size-4" />
									{t('invitePeople', 'Invite people')}
								</Button>
							}
						/>
					}
				/>
				<TeamCallout icon={SearchIcon}>
					{t('matchingAccountsShowUpAsYouTypeAUsernameOrEmailPickTheRightOneAndItGetsTheInvitationStraightAwayAddAPersonalMessageAndItTravelsWithTheInvite', "Matching accounts show up as you type a username or email — pick the right one and it gets the invitation straight away. Add a personal message and it travels with the invite.")}
				</TeamCallout>
			</TeamSection>

			<TeamSection>
				<SectionHeading
					icon={LinkIcon}
					title={t('inviteLinks', 'Invite links')}
					count={links.data?.length ?? 0}
					description={t('shareableLinksThatAddWhoeverOpensThemCapTheUsesOrLeaveThemOpen', 'Shareable links that add whoever opens them. Cap the uses, or leave them open.')}
					actions={
						<Dialog
							open={showCreateLinkDialog}
							onOpenChange={setShowCreateLinkDialog}
						>
							<DialogTrigger asChild>
								<Button variant="outline" size="sm">
									<PlusIcon className="size-4" />
									{t('newLink', 'New link')}
								</Button>
							</DialogTrigger>
							<DialogContent className="sm:max-w-md">
								<DialogHeader className="space-y-3">
									<div className="mx-auto flex h-12 w-12 items-center justify-center rounded-full bg-primary/10">
										<Link className="h-6 w-6 text-primary" />
									</div>
									<DialogTitle className="text-center text-xl">
										{t('createInviteLink', 'Create Invite Link')}
									</DialogTitle>
									<DialogDescription className="text-center">
										{t('generateAShareableLinkWithOptionalUsageLimitsForYourTeam', "Generate a shareable link with optional usage limits for your team")}
									</DialogDescription>
								</DialogHeader>

								<div className="space-y-4 py-4">
									<div className="space-y-2">
										<Label htmlFor="linkName" className="text-sm font-medium">
											{t('linkName', 'Link Name')}
										</Label>
										<div className="relative">
											<Input
												id="linkName"
												placeholder={t('egMarketingTeamBetaUsers', 'e.g., Marketing Team, Beta Users')}
												value={newLinkName}
												onChange={(e) => setNewLinkName(e.target.value)}
												className="pl-10"
											/>
											<Settings className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
										</div>
									</div>

									<div className="space-y-2">
										<Label htmlFor="maxUses" className="text-sm font-medium">
											{t('maximumUses', 'Maximum Uses')}
										</Label>
										<div className="relative">
											<Input
												id="maxUses"
												type="number"
												placeholder={t('leaveEmptyForUnlimitedUses', 'Leave empty for unlimited uses')}
												value={newLinkMaxUses}
												onChange={(e) => setNewLinkMaxUses(e.target.value)}
												className="pl-10"
											/>
											<Users className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
										</div>
										<p className="text-xs text-muted-foreground">
											{t('setALimitOnHowManyPeopleCanUseThisLinkLeaveEmptyForUnlimitedAccess', "Set a limit on how many people can use this link. Leave empty for unlimited access.")}
										</p>
									</div>

									<div className="space-y-2">
										<Label htmlFor="linkExpiry" className="text-sm font-medium">
											{t('expiresAfter', 'Expires After')}
										</Label>
										<Select
											value={newLinkExpiry}
											onValueChange={setNewLinkExpiry}
										>
											<SelectTrigger id="linkExpiry">
												<SelectValue placeholder={t('selectExpiry', 'Select expiry')} />
											</SelectTrigger>
											<SelectContent>
												<SelectItem value="24">{t('1Day', '1 day')}</SelectItem>
												<SelectItem value="168">{t('7Days', '7 days')}</SelectItem>
												<SelectItem value="720">{t('30Days', '30 days')}</SelectItem>
												<SelectItem value="never">{t('never', 'Never')}</SelectItem>
											</SelectContent>
										</Select>
										<p className="text-xs text-muted-foreground">
											{t('expiredLinksStopWorkingButStayListedUntilYouDeleteThem', "Expired links stop working but stay listed until you delete them.")}
										</p>
									</div>
								</div>

								<DialogFooter className="gap-2 sm:gap-0">
									<Button
										variant="outline"
										onClick={() => {
											setShowCreateLinkDialog(false);
											setNewLinkName("");
											setNewLinkMaxUses("");
											setNewLinkExpiry("168");
										}}
									>
										{t('cancel', 'Cancel')}
									</Button>
									<Button
										onClick={createInviteLink}
										disabled={!newLinkName.trim()}
									>
										{t('createLink', 'Create Link')}
									</Button>
								</DialogFooter>
							</DialogContent>
						</Dialog>
					}
				/>

				{(links.data?.length ?? 0) === 0 ? (
					<EmptyState
						className="max-w-full"
						title={t('noInviteLinks', 'No Invite Links')}
						description={t('createInviteLinksToShareYourProject', 'Create Invite Links to share your project')}
						icons={[UsersIcon, LinkIcon, MailIcon]}
					/>
				) : (
					<div className="flex flex-col gap-2">
						{links.data?.map((link) => (
							<div key={link.id} className={teamRowClass({ align: "start" })}>
								<TeamRowIcon icon={LinkIcon} className="mt-0.5" />
								<div className="min-w-0 flex-1">
									<div className={TEAM_ROW_TITLE}>{link.name}</div>
									<div className={TEAM_ROW_META}>
										<span className="flex items-center gap-1">
											<UserCheckIcon className="size-3.5" />{t('count_joinedJoined', '{{count_joined}} joined', { count_joined: link.count_joined })}</span>
										<span>
											{link.max_uses > 0 ? t('ofMax_uses', 'of {{max_uses}}', { max_uses: link.max_uses }) : t('noLimit', 'no limit')}
										</span>
										<span className="flex items-center gap-1">
											<ClockIcon className="size-3.5" />
											{new Date(
												Date.parse(link.created_at),
											).toLocaleDateString()}
										</span>
										<span className="max-w-[18ch] truncate font-mono">
											{link.token}
										</span>
									</div>

									{link.max_uses > 0 && (
										<div className="mt-2 flex items-center gap-2">
											<div className="h-1.5 flex-1 overflow-hidden rounded-full bg-muted">
												<div
													className="h-full rounded-full bg-linear-to-r from-primary to-tertiary transition-all"
													style={{
														width: `${Math.min((link.count_joined / link.max_uses) * 100, 100)}%`,
													}}
												/>
											</div>
											<span className="text-[11px] tabular-nums text-muted-foreground">{`${link.count_joined}/${link.max_uses}`}</span>
										</div>
									)}

									<div className="mt-2 flex items-center gap-2">
										<Input
											value={webLink(link.token)}
											readOnly
											className="h-8 font-mono text-xs"
										/>
										<TeamRowActions always>
											<Button
												onClick={() => copyInviteLink(webLink(link.token))}
												variant="outline"
												size="icon"
												className="size-8"
												aria-label={t('copyInviteLink', 'Copy invite link')}
											>
												<CopyIcon className="size-4" />
											</Button>
											<DropdownMenu>
												<DropdownMenuTrigger asChild>
													<Button
														variant="ghost"
														size="icon"
														className="size-8"
														aria-label={t('inviteLinkOptions', 'Invite link options')}
													>
														<MoreVerticalIcon className="size-4" />
													</Button>
												</DropdownMenuTrigger>
												<DropdownMenuContent align="end">
													<DropdownMenuItem
														onClick={() => copyInviteLink(webLink(link.token))}
													>
														<ExternalLinkIcon className="size-4" />
														{t('copyWebLink', 'Copy Web Link')}
													</DropdownMenuItem>
													<DropdownMenuItem
														onClick={() =>
															copyInviteLink(
																`flow-like://join?appId=${appId}&token=${link.token}`,
															)
														}
													>
														<CopyIcon className="size-4" />
														{t('copyDesktopLink', 'Copy Desktop Link')}
													</DropdownMenuItem>
													<AlertDialog>
														<AlertDialogTrigger asChild>
															<DropdownMenuItem
																variant="destructive"
																onSelect={(e) => e.preventDefault()}
															>
																<Trash2Icon className="size-4" />
																{t('delete', 'Delete')}
															</DropdownMenuItem>
														</AlertDialogTrigger>
														<AlertDialogContent>
															<AlertDialogHeader>
																<AlertDialogTitle>
																	{t('deleteInviteLink', 'Delete Invite Link')}
																</AlertDialogTitle>
																<AlertDialogDescription>
																	Are you sure you want to delete &quot;
																	{link.name}
																	&quot;? This action cannot be undone and the
																	link will no longer work.
																</AlertDialogDescription>
															</AlertDialogHeader>
															<AlertDialogFooter>
																<AlertDialogCancel>{t('cancel', 'Cancel')}</AlertDialogCancel>
																<AlertDialogAction
																	onClick={() => deleteInviteLink(link.id)}
																	className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
																>
																	{t('delete', 'Delete')}
																</AlertDialogAction>
															</AlertDialogFooter>
														</AlertDialogContent>
													</AlertDialog>
												</DropdownMenuContent>
											</DropdownMenu>
										</TeamRowActions>
									</div>
								</div>
							</div>
						))}
					</div>
				)}

				<TeamHint>
					{t('everyLinkCopiesEitherAsAWebAddressOrAsAFlowlikeLinkThatOpensStraightInTheDesktopApp', "Every link copies either as a web address or as a flow-like:// link that opens straight in the desktop app.")}
				</TeamHint>
			</TeamSection>
		</div>
	);
}
