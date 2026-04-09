"use client";

import { AnimatePresence, motion } from "framer-motion";
import type { LucideIcon } from "lucide-react";
import {
	Bell,
	BellRing,
	Check,
	CheckCheck,
	Clock3,
	ExternalLink,
	LoaderCircle,
	MailOpen,
	RefreshCcw,
	Trash2,
	UserPlus,
	Workflow,
	X,
} from "lucide-react";
import { useRouter } from "next/navigation";
import { type ReactNode, useCallback, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import {
	useInfiniteInvoke,
	useInvalidateInvoke,
	useInvoke,
} from "../../hooks/use-invoke";
import { formatRelativeTime } from "../../lib/date";
import { cn } from "../../lib/utils";
import { useBackend } from "../../state/backend-state";
import type { IInvite, INotification } from "../../state/backend-state/types";
import {
	Badge,
	Button,
	Card,
	CardContent,
	CardHeader,
	CardTitle,
	Skeleton,
	Tabs,
	TabsContent,
	TabsList,
	TabsTrigger,
} from "../ui";

type NotificationsTab = "all" | "invitations" | "notifications";

export function NotificationsPageScreen() {
	const backend = useBackend();
	const auth = useAuth();
	const invalidate = useInvalidateInvoke();
	const [activeTab, setActiveTab] = useState<NotificationsTab>("all");
	const authQueryDeps = [auth?.user?.profile?.sub, auth?.isAuthenticated];

	const invitationsQuery = useInfiniteInvoke(
		backend.teamState.getInvites,
		backend.teamState,
		[],
		50,
		Boolean(auth?.isAuthenticated),
		authQueryDeps,
		0,
	);
	const invitations: IInvite[] = invitationsQuery.data
		? invitationsQuery.data.pages.flat()
		: [];

	const notificationsQuery = useInfiniteInvoke(
		backend.userState.listNotifications,
		backend.userState,
		[false],
		50,
		true,
		authQueryDeps,
		0,
	);
	const notifications: INotification[] = notificationsQuery.data
		? notificationsQuery.data.pages.flat()
		: [];

	const isInvitationsBootLoading =
		Boolean(auth?.isAuthenticated) &&
		!invitationsQuery.data &&
		invitationsQuery.isLoading;
	const isNotificationsBootLoading =
		!notificationsQuery.data && notificationsQuery.isLoading;
	const isSummaryLoading =
		invitations.length + notifications.length === 0 &&
		(auth.isLoading || isInvitationsBootLoading || isNotificationsBootLoading);
	const isRefreshing =
		!isSummaryLoading &&
		(notificationsQuery.isFetching || invitationsQuery.isFetching);
	const isFetchingMore =
		notificationsQuery.isFetchingNextPage || invitationsQuery.isFetchingNextPage;

	const totalCount = invitations.length + notifications.length;
	const unreadCount = notifications.filter((notification) => !notification.read).length;
	const subtitle = isSummaryLoading
		? "Pulling together workflow activity and team invites..."
		: totalCount > 0
			? `${invitations.length} invitation${invitations.length !== 1 ? "s" : ""}, ${notifications.length} workflow notification${notifications.length !== 1 ? "s" : ""}`
			: "You are caught up. New workflow activity and team invites will land here.";

	const handleRefresh = useCallback(async () => {
		await Promise.allSettled([
			notificationsQuery.refetch(),
			auth?.isAuthenticated
				? invitationsQuery.refetch()
				: Promise.resolve(undefined),
		]);
	}, [auth?.isAuthenticated, invitationsQuery, notificationsQuery]);

	const syncOverview = useCallback(async () => {
		await invalidate(backend.userState.getNotifications, []);
	}, [backend.userState, invalidate]);

	const handleInviteAction = useCallback(
		async (id: string, action: "accept" | "decline") => {
			try {
				if (action === "accept") {
					await backend.teamState.acceptInvite(id);
				} else {
					await backend.teamState.rejectInvite(id);
				}
				await Promise.all([invitationsQuery.refetch(), syncOverview()]);
			} catch (error) {
				console.error(`Failed to ${action} invite:`, error);
				toast.error(`Failed to ${action} invite. Please try again later.`);
			}
		},
		[backend, invitationsQuery, syncOverview],
	);

	const handleMarkAsRead = useCallback(
		async (id: string) => {
			try {
				await backend.userState.markNotificationRead(id);
				await Promise.all([notificationsQuery.refetch(), syncOverview()]);
			} catch (error) {
				console.error("Failed to mark notification as read:", error);
				toast.error("Failed to mark notification as read");
			}
		},
		[backend, notificationsQuery, syncOverview],
	);

	const handleDeleteNotification = useCallback(
		async (id: string) => {
			try {
				await backend.userState.deleteNotification(id);
				await Promise.all([notificationsQuery.refetch(), syncOverview()]);
				toast.success("Notification deleted");
			} catch (error) {
				console.error("Failed to delete notification:", error);
				toast.error("Failed to delete notification");
			}
		},
		[backend, notificationsQuery, syncOverview],
	);

	const handleMarkAllAsRead = useCallback(async () => {
		try {
			const count = await backend.userState.markAllNotificationsRead();
			await Promise.all([notificationsQuery.refetch(), syncOverview()]);
			toast.success(
				`Marked ${count} notification${count !== 1 ? "s" : ""} as read`,
			);
		} catch (error) {
			console.error("Failed to mark all as read:", error);
			toast.error("Failed to mark all as read");
		}
	}, [backend, notificationsQuery, syncOverview]);

	const showAllSkeleton =
		totalCount === 0 && (isNotificationsBootLoading || isInvitationsBootLoading);
	const showInvitationsSkeleton =
		invitations.length === 0 && isInvitationsBootLoading;
	const showNotificationsSkeleton =
		notifications.length === 0 && isNotificationsBootLoading;

	return (
		<main className="relative flex min-h-0 flex-1 overflow-hidden">
			<div className="absolute inset-0 bg-[radial-gradient(ellipse_at_top,hsl(var(--primary)/0.08),transparent_52%)]" />
			<div className="absolute inset-0 bg-[radial-gradient(ellipse_at_bottom_right,hsl(var(--secondary)/0.08),transparent_46%)]" />
			<div className="absolute inset-0 bg-grid-pattern opacity-[0.04]" />

			<div className="relative mx-auto flex min-h-0 w-full max-w-6xl flex-1 flex-col gap-6 px-4 py-5 sm:px-6 sm:py-6 lg:px-8 lg:py-8">
				<motion.section
					initial={{ opacity: 0, y: -18 }}
					animate={{ opacity: 1, y: 0 }}
					transition={{ duration: 0.35 }}
					className="relative overflow-hidden rounded-4xl border border-border/60 bg-background/80 p-6 shadow-sm backdrop-blur-xl sm:p-8"
				>
					<div className="absolute inset-0 bg-[radial-gradient(ellipse_at_top_left,hsl(var(--primary)/0.12),transparent_50%)]" />
					<div className="absolute inset-0 bg-[radial-gradient(ellipse_at_bottom_right,hsl(var(--accent)/0.2),transparent_45%)]" />

					<div className="relative flex flex-col gap-6 lg:flex-row lg:items-end lg:justify-between">
						<div className="flex items-start gap-4 sm:gap-5">
							<div className="relative flex size-15 shrink-0 items-center justify-center rounded-2xl border border-primary/20 bg-background/85 shadow-sm sm:size-17">
								<BellRing className="size-7 text-primary sm:size-8" />
								{unreadCount > 0 && (
									<span className="absolute -top-1 -right-1 flex min-w-5 items-center justify-center rounded-full bg-primary px-1.5 py-0.5 text-[10px] font-semibold text-primary-foreground shadow-sm">
										{unreadCount > 9 ? "9+" : unreadCount}
									</span>
								)}
							</div>

							<div className="space-y-3">
								<div className="flex flex-wrap items-center gap-2">
									<Badge
										variant="outline"
										className="border-primary/20 bg-primary/5 text-primary"
									>
										Inbox
									</Badge>
									{isRefreshing && (
										<Badge
											variant="outline"
											className="gap-1.5 border-border/60 bg-background/80 text-muted-foreground"
										>
											<LoaderCircle className="size-3.5 animate-spin" />
											Syncing
										</Badge>
									)}
								</div>

								<div className="space-y-2">
									<h1 className="text-3xl font-semibold tracking-tight text-foreground sm:text-4xl">
										Notifications
									</h1>
									<p className="max-w-2xl text-sm leading-6 text-muted-foreground sm:text-base">
										{subtitle}
									</p>
								</div>
							</div>
						</div>

						<div className="flex flex-wrap items-center gap-3">
							<Button
								variant="outline"
								size="sm"
								onClick={() => void handleRefresh()}
								disabled={isRefreshing}
								className="gap-2 bg-background/80"
							>
								{isRefreshing ? (
									<LoaderCircle className="size-4 animate-spin" />
								) : (
									<RefreshCcw className="size-4" />
								)}
								Refresh
							</Button>

							{unreadCount > 0 && (
								<Button
									size="sm"
									onClick={() => void handleMarkAllAsRead()}
									className="gap-2 shadow-sm"
								>
									<CheckCheck className="size-4" />
									Mark all read
								</Button>
							)}
						</div>
					</div>

					<div className="relative mt-6 grid gap-3 md:grid-cols-3">
						<SummaryTile
							label="Unread"
							value={unreadCount}
							description="Needs attention right now"
							icon={BellRing}
							loading={isSummaryLoading}
							accentClass="from-primary/16 via-primary/6 to-transparent"
							iconClassName="text-primary"
						/>
						<SummaryTile
							label="Invitations"
							value={invitations.length}
							description="Pending workspace access"
							icon={UserPlus}
							loading={isSummaryLoading}
							accentClass="from-amber-500/18 via-amber-500/5 to-transparent"
							iconClassName="text-amber-600"
						/>
						<SummaryTile
							label="Workflow Alerts"
							value={notifications.length}
							description="Runs, updates, and system activity"
							icon={Workflow}
							loading={isSummaryLoading}
							accentClass="from-sky-500/18 via-sky-500/5 to-transparent"
							iconClassName="text-sky-600"
						/>
					</div>
				</motion.section>

				<Tabs
					value={activeTab}
					onValueChange={(value) => setActiveTab(value as NotificationsTab)}
					className="flex min-h-0 flex-1 flex-col"
				>
					<div className="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
						<TabsList className="h-auto w-full flex-wrap justify-start gap-2 rounded-2xl bg-muted/60 p-1 md:w-fit">
							<TabsTrigger value="all" className="gap-2 rounded-xl px-4 py-2">
								<Bell className="size-4" />
								All Activity
								{totalCount > 0 && (
									<Badge variant="secondary">{totalCount}</Badge>
								)}
							</TabsTrigger>
							<TabsTrigger
								value="invitations"
								className="gap-2 rounded-xl px-4 py-2"
							>
								<UserPlus className="size-4" />
								Invitations
								{invitations.length > 0 && (
									<Badge variant="secondary">{invitations.length}</Badge>
								)}
							</TabsTrigger>
							<TabsTrigger
								value="notifications"
								className="gap-2 rounded-xl px-4 py-2"
							>
								<Workflow className="size-4" />
								Workflows
								{notifications.length > 0 && (
									<Badge variant="secondary">{notifications.length}</Badge>
								)}
							</TabsTrigger>
						</TabsList>

						<div className="flex min-h-9 items-center gap-2 text-sm text-muted-foreground">
							{isFetchingMore && (
								<Badge
									variant="outline"
									className="gap-1.5 border-border/60 bg-background/80"
								>
									<LoaderCircle className="size-3.5 animate-spin" />
									Loading more
								</Badge>
							)}
							<span className="hidden md:inline">
								{activeTab === "all"
									? "Everything in one stream"
									: activeTab === "invitations"
										? "Team access requests and invites"
										: "Workflow and system updates"}
							</span>
						</div>
					</div>

					<TabsContent value="all" className="mt-4 min-h-0 flex-1 overflow-auto pr-1">
						<NotificationsPanel>
							<AnimatePresence mode="popLayout">
								{showAllSkeleton ? (
									<NotificationsListSkeleton variant="all" />
								) : totalCount === 0 ? (
									<NotificationsEmptyState
										title="No activity just yet"
										description="When teammates invite you or workflows finish, fail, or need input, they will appear here."
										icon={MailOpen}
									/>
								) : (
									<>
										{invitations.map((invite, index) => (
											<InvitationCard
												key={invite.id}
												invite={invite}
												index={index}
												onAction={handleInviteAction}
											/>
										))}
										{notifications.map((notification, index) => (
											<NotificationCard
												key={notification.id}
												notification={notification}
												index={invitations.length + index}
												onMarkRead={handleMarkAsRead}
												onDelete={handleDeleteNotification}
											/>
										))}
										{(invitationsQuery.hasNextPage || notificationsQuery.hasNextPage) && (
											<LoadMoreButton
												onClick={() => {
													if (invitationsQuery.hasNextPage) {
														void invitationsQuery.fetchNextPage();
													}
													if (notificationsQuery.hasNextPage) {
														void notificationsQuery.fetchNextPage();
													}
												}}
												loading={isFetchingMore}
											/>
										)}
									</>
								)}
							</AnimatePresence>
						</NotificationsPanel>
					</TabsContent>

					<TabsContent
						value="invitations"
						className="mt-4 min-h-0 flex-1 overflow-auto pr-1"
					>
						<NotificationsPanel>
							<AnimatePresence mode="popLayout">
								{showInvitationsSkeleton ? (
									<NotificationsListSkeleton variant="invitations" />
								) : invitations.length === 0 ? (
									<NotificationsEmptyState
										title="No pending invitations"
										description="When someone invites you into a workspace or project, it will show up here first."
										icon={UserPlus}
									/>
								) : (
									<>
										{invitations.map((invite, index) => (
											<InvitationCard
												key={invite.id}
												invite={invite}
												index={index}
												onAction={handleInviteAction}
											/>
										))}
										{invitationsQuery.hasNextPage && (
											<LoadMoreButton
												onClick={() => void invitationsQuery.fetchNextPage()}
												loading={invitationsQuery.isFetchingNextPage}
											/>
										)}
									</>
								)}
							</AnimatePresence>
						</NotificationsPanel>
					</TabsContent>

					<TabsContent
						value="notifications"
						className="mt-4 min-h-0 flex-1 overflow-auto pr-1"
					>
						<NotificationsPanel>
							<AnimatePresence mode="popLayout">
								{showNotificationsSkeleton ? (
									<NotificationsListSkeleton variant="notifications" />
								) : notifications.length === 0 ? (
									<NotificationsEmptyState
										title="No workflow updates"
										description="Workflow runs, system notices, and other runtime updates will appear here as they happen."
										icon={Workflow}
									/>
								) : (
									<>
										{notifications.map((notification, index) => (
											<NotificationCard
												key={notification.id}
												notification={notification}
												index={index}
												onMarkRead={handleMarkAsRead}
												onDelete={handleDeleteNotification}
											/>
										))}
										{notificationsQuery.hasNextPage && (
											<LoadMoreButton
												onClick={() => void notificationsQuery.fetchNextPage()}
												loading={notificationsQuery.isFetchingNextPage}
											/>
										)}
									</>
								)}
							</AnimatePresence>
						</NotificationsPanel>
					</TabsContent>
				</Tabs>
			</div>
		</main>
	);
}

function SummaryTile({
	label,
	value,
	description,
	icon: Icon,
	loading,
	accentClass,
	iconClassName,
}: {
	label: string;
	value: number;
	description: string;
	icon: LucideIcon;
	loading: boolean;
	accentClass: string;
	iconClassName: string;
}) {
	return (
		<motion.div whileHover={{ y: -2 }} transition={{ duration: 0.18 }}>
			<Card className="relative overflow-hidden border-border/60 bg-background/85 shadow-sm backdrop-blur-sm">
				<div className={cn("absolute inset-0 bg-linear-to-br", accentClass)} />
				<CardContent className="relative flex items-start justify-between gap-4 p-5">
					<div className="space-y-2">
						<p className="text-xs font-medium uppercase tracking-[0.18em] text-muted-foreground">
							{label}
						</p>
						{loading ? (
							<>
								<Skeleton className="h-8 w-14" />
								<Skeleton className="h-3 w-32" />
							</>
						) : (
							<>
								<div className="text-3xl font-semibold tracking-tight text-foreground">
									{value}
								</div>
								<p className="text-sm text-muted-foreground">{description}</p>
							</>
						)}
					</div>

					<div className="rounded-2xl border border-border/60 bg-background/80 p-3 shadow-sm">
						<Icon className={cn("size-5", iconClassName)} />
					</div>
				</CardContent>
			</Card>
		</motion.div>
	);
}

function NotificationsPanel({ children }: { children: ReactNode }) {
	return (
		<div className="min-h-96 rounded-[1.75rem] border border-border/60 bg-background/78 p-4 shadow-sm backdrop-blur-sm sm:p-5">
			{children}
		</div>
	);
}

function NotificationsEmptyState({
	title,
	description,
	icon: Icon,
}: {
	title: string;
	description: string;
	icon: LucideIcon;
}) {
	return (
		<motion.div
			initial={{ opacity: 0, scale: 0.98 }}
			animate={{ opacity: 1, scale: 1 }}
			exit={{ opacity: 0, scale: 0.98 }}
			transition={{ duration: 0.2 }}
			className="flex min-h-88 items-center justify-center"
		>
			<div className="relative max-w-xl overflow-hidden rounded-[1.75rem] border border-dashed border-border/70 bg-muted/30 px-8 py-12 text-center">
				<div className="absolute inset-0 bg-[radial-gradient(circle_at_top,hsl(var(--primary)/0.08),transparent_52%)]" />
				<div className="relative flex flex-col items-center gap-4">
					<div className="rounded-2xl border border-border/60 bg-background/80 p-4 shadow-sm">
						<Icon className="size-8 text-primary" />
					</div>
					<div className="space-y-2">
						<h3 className="text-xl font-semibold tracking-tight text-foreground">
							{title}
						</h3>
						<p className="text-sm leading-6 text-muted-foreground">
							{description}
						</p>
					</div>
				</div>
			</div>
		</motion.div>
	);
}

function NotificationsListSkeleton({
	variant,
}: {
	variant: NotificationsTab;
}) {
	const rows = variant === "all" ? 3 : 2;

	return (
		<div className="space-y-4">
			{Array.from({ length: rows }).map((_, index) => (
				<Card
					key={`${variant}-skeleton-${index.toString()}`}
					className="border-border/50 bg-background/80 shadow-sm"
				>
					<CardHeader className="pb-4">
						<div className="flex items-start gap-3">
							<Skeleton className="size-11 rounded-2xl" />
							<div className="flex-1 space-y-2.5">
								<Skeleton className="h-5 w-44" />
								<div className="flex flex-wrap gap-2">
									<Skeleton className="h-5 w-20 rounded-full" />
									<Skeleton className="h-5 w-28 rounded-full" />
								</div>
							</div>
						</div>
					</CardHeader>
					<CardContent className="space-y-4 pt-0">
						<Skeleton className="h-4 w-full" />
						<Skeleton className="h-4 w-5/6" />
						<div className="flex flex-wrap gap-2">
							<Skeleton className="h-9 w-28 rounded-lg" />
							<Skeleton className="h-9 w-24 rounded-lg" />
						</div>
					</CardContent>
				</Card>
			))}
		</div>
	);
}

function LoadMoreButton({
	onClick,
	loading,
}: {
	onClick: () => void;
	loading: boolean;
}) {
	return (
		<motion.div
			initial={{ opacity: 0, y: 12 }}
			animate={{ opacity: 1, y: 0 }}
			exit={{ opacity: 0, y: 12 }}
			transition={{ duration: 0.2 }}
			className="flex justify-center pt-2"
		>
			<Button
				variant="outline"
				onClick={onClick}
				disabled={loading}
				className="w-full max-w-md gap-2 bg-background/85"
			>
				{loading && <LoaderCircle className="size-4 animate-spin" />}
				{loading ? "Loading..." : "Load more"}
			</Button>
		</motion.div>
	);
}

type InvitationCardProps = {
	invite: IInvite;
	index: number;
	onAction: (id: string, action: "accept" | "decline") => void;
};

function InvitationCard({
	invite,
	index,
	onAction,
}: Readonly<InvitationCardProps>) {
	const backend = useBackend();
	const userLookup = useInvoke(
		backend.userState.lookupUser,
		backend.userState,
		[invite.by_member_id],
	);

	const inviterLabel =
		userLookup.data?.name ??
		userLookup.data?.username ??
		userLookup.data?.email ??
		null;

	return (
		<motion.div
			layout
			initial={{ opacity: 0, y: 16, scale: 0.98 }}
			animate={{ opacity: 1, y: 0, scale: 1 }}
			exit={{ opacity: 0, y: -10, scale: 0.98 }}
			transition={{ duration: 0.22, delay: index * 0.04 }}
		>
			<Card className="overflow-hidden border-border/60 bg-background/85 shadow-sm transition-all duration-200 hover:-translate-y-0.5 hover:border-primary/25 hover:shadow-md">
				<div className="h-1 bg-linear-to-r from-amber-500/90 via-primary/80 to-primary/20" />
				<CardHeader className="pb-4">
					<div className="flex items-start justify-between gap-4">
						<div className="flex min-w-0 items-start gap-3">
							<div className="rounded-2xl border border-amber-500/20 bg-amber-500/10 p-3 shadow-sm">
								<UserPlus className="size-5 text-amber-600" />
							</div>
							<div className="min-w-0 space-y-2">
								<div className="flex flex-wrap items-center gap-2">
									<Badge variant="outline" className="border-amber-500/20 bg-amber-500/5 text-amber-700">
										Invitation
									</Badge>
									<Badge variant="secondary" className="gap-1.5">
										<Clock3 className="size-3" />
										{formatRelativeTime(invite.created_at)}
									</Badge>
								</div>

								<CardTitle className="truncate text-lg font-semibold tracking-tight text-foreground sm:text-xl">
									{invite.name ?? "New invitation"}
								</CardTitle>

								<div className="flex min-h-5 items-center gap-2 text-sm text-muted-foreground">
									<span>Invited by</span>
									{inviterLabel ? (
										<Badge variant="secondary" className="font-medium">
											{inviterLabel}
										</Badge>
									) : (
										<Skeleton className="h-5 w-24 rounded-full" />
									)}
								</div>
							</div>
						</div>
					</div>
				</CardHeader>

				<CardContent className="space-y-4 pt-0">
					{invite.message && (
						<p className="text-sm leading-6 text-muted-foreground sm:text-[15px]">
							{invite.message}
						</p>
					)}

					<div className="flex flex-wrap gap-2">
						<Button
							onClick={() => onAction(invite.id, "accept")}
							size="sm"
							className="gap-2"
						>
							<Check className="size-4" />
							Accept
						</Button>
						<Button
							onClick={() => onAction(invite.id, "decline")}
							variant="outline"
							size="sm"
							className="gap-2"
						>
							<X className="size-4" />
							Decline
						</Button>
					</div>
				</CardContent>
			</Card>
		</motion.div>
	);
}

type NotificationCardProps = {
	notification: INotification;
	index: number;
	onMarkRead: (id: string) => void;
	onDelete: (id: string) => void;
};

function NotificationCard({
	notification,
	index,
	onMarkRead,
	onDelete,
}: Readonly<NotificationCardProps>) {
	const router = useRouter();

	const handleLinkClick = useCallback(() => {
		if (!notification.link) {
			return;
		}

		if (!notification.read) {
			void onMarkRead(notification.id);
		}

		if (notification.link.startsWith("http")) {
			window.open(notification.link, "_blank", "noopener,noreferrer");
			return;
		}

		router.push(notification.link);
	}, [notification, onMarkRead, router]);

	return (
		<motion.div
			layout
			initial={{ opacity: 0, y: 16, scale: 0.98 }}
			animate={{ opacity: 1, y: 0, scale: 1 }}
			exit={{ opacity: 0, y: -10, scale: 0.98 }}
			transition={{ duration: 0.22, delay: index * 0.035 }}
		>
			<Card
				className={cn(
					"overflow-hidden border-border/60 bg-background/85 shadow-sm transition-all duration-200 hover:-translate-y-0.5 hover:border-primary/25 hover:shadow-md",
					!notification.read && "border-primary/25 bg-primary/3",
				)}
			>
				<div
					className={cn(
						"h-1 bg-linear-to-r",
						notification.read
							? "from-border via-border/50 to-transparent"
							: "from-primary via-sky-500/80 to-sky-500/20",
					)}
				/>
				<CardHeader className="pb-4">
					<div className="flex items-start justify-between gap-4">
						<div className="flex min-w-0 items-start gap-3">
							<div
								className={cn(
									"rounded-2xl border p-3 shadow-sm",
									notification.read
										? "border-border/60 bg-muted/70"
										: "border-primary/20 bg-primary/10",
								)}
							>
								{notification.icon ? (
									<span className="text-lg leading-none">{notification.icon}</span>
								) : (
									<Workflow
										className={cn(
											"size-5",
											notification.read
												? "text-muted-foreground"
												: "text-primary",
										)}
									/>
								)}
							</div>

							<div className="min-w-0 space-y-2">
								<div className="flex flex-wrap items-center gap-2">
									<Badge
										variant={
											notification.notification_type === "WORKFLOW"
												? "default"
												: "secondary"
										}
									>
										{notification.notification_type === "WORKFLOW"
											? "Workflow"
											: "System"}
									</Badge>
									<Badge variant="secondary" className="gap-1.5">
										<Clock3 className="size-3" />
										{formatRelativeTime(notification.created_at)}
									</Badge>
									{!notification.read && (
										<Badge
											variant="outline"
											className="border-primary/20 bg-primary/5 text-primary"
										>
											New
										</Badge>
									)}
								</div>

								<CardTitle
									className={cn(
										"truncate text-lg font-semibold tracking-tight sm:text-xl",
										notification.read
											? "text-foreground/80"
											: "text-foreground",
									)}
								>
									{notification.title}
								</CardTitle>
							</div>
						</div>
					</div>
				</CardHeader>

				<CardContent className="space-y-4 pt-0">
					{notification.description && (
						<p className="text-sm leading-6 text-muted-foreground sm:text-[15px]">
							{notification.description}
						</p>
					)}

					<div className="flex flex-wrap gap-2">
						{notification.link && (
							<Button
								onClick={handleLinkClick}
								variant="outline"
								size="sm"
								className="gap-2"
							>
								<ExternalLink className="size-4" />
								View details
							</Button>
						)}

						{!notification.read && (
							<Button
								onClick={() => onMarkRead(notification.id)}
								variant="secondary"
								size="sm"
								className="gap-2"
							>
								<Check className="size-4" />
								Mark as read
							</Button>
						)}

						<Button
							onClick={() => onDelete(notification.id)}
							variant="outline"
							size="sm"
							className="gap-2 text-destructive hover:text-destructive"
						>
							<Trash2 className="size-4" />
							Delete
						</Button>
					</div>
				</CardContent>
			</Card>
		</motion.div>
	);
}