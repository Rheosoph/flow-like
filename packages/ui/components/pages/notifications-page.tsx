"use client";

import { AnimatePresence, motion } from "framer-motion";
import type { LucideIcon } from "lucide-react";
import {
	Bell,
	BellRing,
	Check,
	CheckCheck,
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
			<div className="absolute inset-0 bg-[radial-gradient(ellipse_at_top,hsl(var(--primary)/0.06),transparent_52%)]" />
			<div className="absolute inset-0 bg-grid-pattern opacity-[0.03]" />

			<div className="relative mx-auto flex min-h-0 w-full max-w-5xl flex-1 flex-col gap-4 px-4 py-4 sm:px-6 sm:py-5 lg:px-8">
				<motion.section
					initial={{ opacity: 0, y: -12 }}
					animate={{ opacity: 1, y: 0 }}
					transition={{ duration: 0.3 }}
					className="relative overflow-hidden rounded-2xl border border-border/60 bg-background/80 px-5 py-4 shadow-sm backdrop-blur-xl sm:px-6"
				>
					<div className="absolute inset-0 bg-[radial-gradient(ellipse_at_top_left,hsl(var(--primary)/0.08),transparent_50%)]" />

					<div className="relative flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
						<div className="flex items-center gap-3">
							<div className="relative flex size-10 shrink-0 items-center justify-center rounded-xl border border-primary/20 bg-background/85 shadow-sm">
								<BellRing className="size-5 text-primary" />
								{unreadCount > 0 && (
									<span className="absolute -top-1 -right-1 flex min-w-4 items-center justify-center rounded-full bg-primary px-1 py-px text-[9px] font-semibold text-primary-foreground shadow-sm">
										{unreadCount > 9 ? "9+" : unreadCount}
									</span>
								)}
							</div>

							<div>
								<div className="flex items-center gap-2">
									<h1 className="text-xl font-semibold tracking-tight text-foreground">
										Notifications
									</h1>
									{isRefreshing && (
										<LoaderCircle className="size-3.5 animate-spin text-muted-foreground" />
									)}
								</div>
								<p className="text-xs text-muted-foreground">
									{subtitle}
								</p>
							</div>
						</div>

						<div className="flex items-center gap-2">
							<Button
								variant="ghost"
								size="sm"
								onClick={() => void handleRefresh()}
								disabled={isRefreshing}
								className="gap-1.5 text-muted-foreground"
							>
								<RefreshCcw className="size-3.5" />
								Refresh
							</Button>

							{unreadCount > 0 && (
								<Button
									variant="outline"
									size="sm"
									onClick={() => void handleMarkAllAsRead()}
									className="gap-1.5"
								>
									<CheckCheck className="size-3.5" />
									Mark all read
								</Button>
							)}
						</div>
					</div>

					<div className="relative mt-3 grid gap-2 sm:grid-cols-3">
						<SummaryTile
							label="Unread"
							value={unreadCount}
							icon={BellRing}
							loading={isSummaryLoading}
							iconClassName="text-primary"
						/>
						<SummaryTile
							label="Invitations"
							value={invitations.length}
							icon={UserPlus}
							loading={isSummaryLoading}
							iconClassName="text-amber-600"
						/>
						<SummaryTile
							label="Workflows"
							value={notifications.length}
							icon={Workflow}
							loading={isSummaryLoading}
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

					<TabsContent value="all" className="mt-3 min-h-0 flex-1 overflow-auto pr-1">
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
						className="mt-3 min-h-0 flex-1 overflow-auto pr-1"
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
						className="mt-3 min-h-0 flex-1 overflow-auto pr-1"
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
	icon: Icon,
	loading,
	iconClassName,
}: {
	label: string;
	value: number;
	icon: LucideIcon;
	loading: boolean;
	iconClassName: string;
}) {
	return (
		<div className="flex items-center gap-3 rounded-xl border border-border/50 bg-background/70 px-3.5 py-2.5">
			<div className="rounded-lg border border-border/50 bg-background/80 p-1.5">
				<Icon className={cn("size-3.5", iconClassName)} />
			</div>
			{loading ? (
				<Skeleton className="h-4 w-16" />
			) : (
				<div className="flex items-baseline gap-1.5">
					<span className="text-lg font-semibold tabular-nums text-foreground">{value}</span>
					<span className="text-xs text-muted-foreground">{label}</span>
				</div>
			)}
		</div>
	);
}

function NotificationsPanel({ children }: { children: ReactNode }) {
	return (
		<div className="flex flex-col gap-2 py-1">
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
			className="flex min-h-48 items-center justify-center"
		>
			<div className="flex flex-col items-center gap-3 text-center">
				<div className="rounded-xl border border-border/60 bg-muted/40 p-3">
					<Icon className="size-6 text-muted-foreground" />
				</div>
				<div className="space-y-1">
					<h3 className="text-sm font-medium text-foreground">
						{title}
					</h3>
					<p className="text-xs text-muted-foreground">
						{description}
					</p>
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
	const rows = variant === "all" ? 4 : 3;

	return (
		<div className="flex flex-col gap-2">
			{Array.from({ length: rows }).map((_, index) => (
				<div
					key={`${variant}-skeleton-${index.toString()}`}
					className="flex items-start gap-3 rounded-xl border border-border/50 bg-background/85 px-4 py-3"
				>
					<Skeleton className="size-8 shrink-0 rounded-lg" />
					<div className="flex-1 space-y-2">
						<Skeleton className="h-4 w-48" />
						<Skeleton className="h-3 w-24" />
						<Skeleton className="h-3 w-64" />
					</div>
				</div>
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
			<div className="rounded-xl border border-border/50 bg-background/85 px-4 py-3 transition-colors hover:border-primary/20 hover:bg-background/95">
				<div className="flex items-start gap-3">
					<div className="mt-0.5 rounded-lg border border-amber-500/20 bg-amber-500/8 p-2">
						<UserPlus className="size-4 text-amber-600" />
					</div>

					<div className="min-w-0 flex-1">
						<div className="flex items-center justify-between gap-2">
							<p className="truncate text-sm font-medium text-foreground">
								{invite.name ?? "New invitation"}
							</p>
							<span className="shrink-0 text-xs text-muted-foreground">
								{formatRelativeTime(invite.created_at)}
							</span>
						</div>

						<div className="mt-0.5 flex items-center gap-1.5 text-xs text-muted-foreground">
							<Badge variant="outline" className="border-amber-500/20 bg-amber-500/5 px-1.5 py-0 text-[10px] text-amber-700">
								Invitation
							</Badge>
							<span>from</span>
							{inviterLabel ? (
								<span className="font-medium text-foreground/80">{inviterLabel}</span>
							) : (
								<Skeleton className="h-3 w-16" />
							)}
						</div>

						{invite.message && (
							<p className="mt-1 line-clamp-1 text-xs text-muted-foreground">
								{invite.message}
							</p>
						)}

						<div className="mt-2 flex gap-2">
							<Button
								onClick={() => onAction(invite.id, "accept")}
								size="sm"
								className="h-7 gap-1.5 px-3 text-xs"
							>
								<Check className="size-3" />
								Accept
							</Button>
							<Button
								onClick={() => onAction(invite.id, "decline")}
								variant="ghost"
								size="sm"
								className="h-7 gap-1.5 px-3 text-xs"
							>
								<X className="size-3" />
								Decline
							</Button>
						</div>
					</div>
				</div>
			</div>
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
			<div
				className={cn(
					"group rounded-xl border px-4 py-3 transition-colors",
					notification.read
						? "border-border/50 bg-background/85 hover:border-border/70 hover:bg-background/95"
						: "border-primary/20 bg-primary/3 hover:border-primary/30 hover:bg-primary/5",
				)}
			>
				<div className="flex items-start gap-3">
					<div
						className={cn(
							"mt-0.5 rounded-lg border p-2",
							notification.read
								? "border-border/50 bg-muted/50"
								: "border-primary/20 bg-primary/8",
						)}
					>
						{notification.icon ? (
							<span className="text-sm leading-none">{notification.icon}</span>
						) : (
							<Workflow
								className={cn(
									"size-4",
									notification.read ? "text-muted-foreground" : "text-primary",
								)}
							/>
						)}
					</div>

					<div className="min-w-0 flex-1">
						<div className="flex items-center justify-between gap-2">
							<p className={cn(
								"truncate text-sm font-medium",
								notification.read ? "text-foreground/80" : "text-foreground",
							)}>
								{notification.title}
							</p>
							<span className="shrink-0 text-xs text-muted-foreground">
								{formatRelativeTime(notification.created_at)}
							</span>
						</div>

						<div className="mt-0.5 flex items-center gap-1.5">
							<Badge
								variant={notification.notification_type === "WORKFLOW" ? "default" : "secondary"}
								className="px-1.5 py-0 text-[10px]"
							>
								{notification.notification_type === "WORKFLOW" ? "Workflow" : "System"}
							</Badge>
							{!notification.read && (
								<span className="size-1.5 rounded-full bg-primary" />
							)}
						</div>

						{notification.description && (
							<p className="mt-1 line-clamp-1 text-xs text-muted-foreground">
								{notification.description}
							</p>
						)}

						<div className="mt-2 flex gap-2">
							{notification.link && (
								<Button
									onClick={handleLinkClick}
									variant="outline"
									size="sm"
									className="h-7 gap-1.5 px-3 text-xs"
								>
									<ExternalLink className="size-3" />
									View
								</Button>
							)}

							{!notification.read && (
								<Button
									onClick={() => onMarkRead(notification.id)}
									variant="ghost"
									size="sm"
									className="h-7 gap-1.5 px-3 text-xs"
								>
									<Check className="size-3" />
									Read
								</Button>
							)}

							<Button
								onClick={() => onDelete(notification.id)}
								variant="ghost"
								size="sm"
								className="h-7 gap-1.5 px-3 text-xs text-destructive hover:text-destructive"
							>
								<Trash2 className="size-3" />
								Delete
							</Button>
						</div>
					</div>
				</div>
			</div>
		</motion.div>
	);
}