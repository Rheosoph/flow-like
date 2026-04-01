"use client";
import {
	Badge,
	Button,
	Card,
	CardContent,
	CardHeader,
	CardTitle,
	type IInvite,
	type INotification,
	Separator,
	Tabs,
	TabsContent,
	TabsList,
	TabsTrigger,
	formatRelativeTime,
	useBackend,
	useInfiniteInvoke,
	useInvoke,
} from "@tm9657/flow-like-ui";
import { AnimatePresence, motion } from "framer-motion";
import {
	Bell,
	BellIcon,
	Check,
	CheckCheck,
	Clock,
	ExternalLink,
	MailOpen,
	Sparkles,
	Trash2,
	UserPlus,
	Workflow,
	X,
} from "lucide-react";
import { useRouter } from "next/navigation";
import { useCallback, useState } from "react";
import { toast } from "sonner";

export default function NotificationsPage() {
	const backend = useBackend();
	const [activeTab, setActiveTab] = useState<
		"all" | "invitations" | "notifications"
	>("all");

	const {
		data: invitationPages,
		fetchNextPage: fetchNextInvitations,
		hasNextPage: hasMoreInvitations,
		refetch: refetchInvitations,
	} = useInfiniteInvoke(
		backend.teamState.getInvites,
		backend.teamState,
		[],
		50,
		true,
		[],
		0,
	);
	const invitations: IInvite[] = invitationPages
		? invitationPages.pages.flat()
		: [];

	const {
		data: notificationPages,
		fetchNextPage: fetchNextNotifications,
		hasNextPage: hasMoreNotifications,
		refetch: refetchNotifications,
	} = useInfiniteInvoke(
		backend.userState.listNotifications,
		backend.userState,
		[false],
		50,
		true,
		[],
		0, // staleTime: 0 to always refetch on mount
	);
	const notifications: INotification[] = notificationPages
		? notificationPages.pages.flat()
		: [];

	const handleInviteAction = useCallback(
		async (id: string, action: "accept" | "decline") => {
			try {
				if (action === "accept") {
					await backend.teamState.acceptInvite(id);
				} else {
					await backend.teamState.rejectInvite(id);
				}
				await refetchInvitations();
			} catch (error) {
				console.error(`Failed to ${action} invite:`, error);
				toast.error(`Failed to ${action} invite. Please try again later.`);
			}
		},
		[backend, refetchInvitations],
	);

	const handleMarkAsRead = useCallback(
		async (id: string) => {
			try {
				await backend.userState.markNotificationRead(id);
				await refetchNotifications();
			} catch (error) {
				console.error("Failed to mark notification as read:", error);
			}
		},
		[backend, refetchNotifications],
	);

	const handleDeleteNotification = useCallback(
		async (id: string) => {
			try {
				await backend.userState.deleteNotification(id);
				await refetchNotifications();
				toast.success("Notification deleted");
			} catch (error) {
				console.error("Failed to delete notification:", error);
				toast.error("Failed to delete notification");
			}
		},
		[backend, refetchNotifications],
	);

	const handleMarkAllAsRead = useCallback(async () => {
		try {
			const count = await backend.userState.markAllNotificationsRead();
			await refetchNotifications();
			toast.success(
				`Marked ${count} notification${count !== 1 ? "s" : ""} as read`,
			);
		} catch (error) {
			console.error("Failed to mark all as read:", error);
			toast.error("Failed to mark all as read");
		}
	}, [backend, refetchNotifications]);

	const totalCount = invitations.length + notifications.length;
	const unreadCount = notifications.filter((n) => !n.read).length;

	return (
		<main className="flex max-w-screen-xl w-full overflow-hidden flex-col p-6 gap-8 mx-auto flex-1 min-h-0">
			{/* Header Section */}
			<motion.div
				initial={{ opacity: 0, y: -20 }}
				animate={{ opacity: 1, y: 0 }}
				transition={{ duration: 0.5 }}
				className="flex items-center justify-between"
			>
				<div className="flex items-center gap-4">
					<div className="relative">
						<motion.div
							animate={{ rotate: [0, 15, -15, 0] }}
							transition={{
								duration: 2,
								repeat: Number.POSITIVE_INFINITY,
								repeatDelay: 3,
							}}
						>
							<BellIcon className="w-10 h-10 text-primary" />
						</motion.div>
						{unreadCount > 0 && (
							<span className="absolute -top-1 -right-1 w-5 h-5 bg-destructive text-destructive-foreground text-xs font-bold rounded-full flex items-center justify-center">
								{unreadCount > 9 ? "9+" : unreadCount}
							</span>
						)}
					</div>
					<div>
						<h1 className="text-4xl font-bold text-foreground relative">
							Notifications
						</h1>
						<p className="text-muted-foreground mt-1">
							{totalCount > 0
								? `${invitations.length} invitation${invitations.length !== 1 ? "s" : ""}, ${notifications.length} notification${notifications.length !== 1 ? "s" : ""}`
								: "All caught up! No new notifications"}
						</p>
					</div>
				</div>

				<div className="flex items-center gap-3">
					{unreadCount > 0 && (
						<motion.div
							initial={{ opacity: 0, x: 20 }}
							animate={{ opacity: 1, x: 0 }}
							transition={{ delay: 0.3 }}
						>
							<Button variant="outline" size="sm" onClick={handleMarkAllAsRead}>
								<CheckCheck className="w-4 h-4 mr-2" />
								Mark all read
							</Button>
						</motion.div>
					)}
					{totalCount > 0 && (
						<motion.div
							initial={{ opacity: 0, x: 20 }}
							animate={{ opacity: 1, x: 0 }}
							transition={{ delay: 0.3 }}
							className="flex items-center gap-2 px-4 py-2 bg-primary/10 border border-primary/20 rounded-full"
						>
							<Sparkles className="w-4 h-4 text-primary" />
							<span className="text-sm font-medium text-primary">
								{totalCount} item{totalCount !== 1 ? "s" : ""}
							</span>
						</motion.div>
					)}
				</div>
			</motion.div>

			<Separator className="bg-border" />

			{/* Tabs */}
			<Tabs
				value={activeTab}
				onValueChange={(v) => setActiveTab(v as typeof activeTab)}
				className="flex-1 flex flex-col min-h-0"
			>
				<TabsList className="w-fit">
					<TabsTrigger value="all" className="gap-2">
						<Bell className="w-4 h-4" />
						All
						{totalCount > 0 && <Badge variant="secondary">{totalCount}</Badge>}
					</TabsTrigger>
					<TabsTrigger value="invitations" className="gap-2">
						<UserPlus className="w-4 h-4" />
						Invitations
						{invitations.length > 0 && (
							<Badge variant="secondary">{invitations.length}</Badge>
						)}
					</TabsTrigger>
					<TabsTrigger value="notifications" className="gap-2">
						<Workflow className="w-4 h-4" />
						Workflows
						{notifications.length > 0 && (
							<Badge variant="secondary">{notifications.length}</Badge>
						)}
					</TabsTrigger>
				</TabsList>

				<TabsContent
					value="all"
					className="flex-1 min-h-0 overflow-auto space-y-4 pr-2 py-2 mt-4"
				>
					<AnimatePresence mode="popLayout">
						{totalCount === 0 ? (
							<EmptyState />
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
								{(hasMoreInvitations || hasMoreNotifications) && (
									<LoadMoreButton
										onClick={() => {
											if (hasMoreInvitations) fetchNextInvitations();
											if (hasMoreNotifications) fetchNextNotifications();
										}}
									/>
								)}
							</>
						)}
					</AnimatePresence>
				</TabsContent>

				<TabsContent
					value="invitations"
					className="flex-1 min-h-0 overflow-auto space-y-4 pr-2 py-2 mt-4"
				>
					<AnimatePresence mode="popLayout">
						{invitations.length === 0 ? (
							<EmptyState message="No pending invitations" />
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
								{hasMoreInvitations && (
									<LoadMoreButton onClick={() => fetchNextInvitations()} />
								)}
							</>
						)}
					</AnimatePresence>
				</TabsContent>

				<TabsContent
					value="notifications"
					className="flex-1 min-h-0 overflow-auto space-y-4 pr-2 py-2 mt-4"
				>
					<AnimatePresence mode="popLayout">
						{notifications.length === 0 ? (
							<EmptyState message="No workflow notifications" />
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
								{hasMoreNotifications && (
									<LoadMoreButton onClick={() => fetchNextNotifications()} />
								)}
							</>
						)}
					</AnimatePresence>
				</TabsContent>
			</Tabs>
		</main>
	);
}

function EmptyState({
	message = "No notifications at the moment",
}: { message?: string }) {
	return (
		<motion.div
			initial={{ opacity: 0, scale: 0.9 }}
			animate={{ opacity: 1, scale: 1 }}
			exit={{ opacity: 0, scale: 0.9 }}
			transition={{ duration: 0.3 }}
		>
			<Card className="border-dashed border-2 border-border bg-muted/30">
				<CardContent className="py-12 text-center">
					<motion.div
						animate={{ y: [0, -10, 0] }}
						transition={{ duration: 2, repeat: Number.POSITIVE_INFINITY }}
						className="mb-4"
					>
						<MailOpen className="w-16 h-16 text-muted-foreground mx-auto" />
					</motion.div>
					<h3 className="text-xl font-semibold text-foreground mb-2">
						All clear!
					</h3>
					<p className="text-muted-foreground">{message}</p>
				</CardContent>
			</Card>
		</motion.div>
	);
}

function LoadMoreButton({ onClick }: { onClick: () => void }) {
	return (
		<motion.div
			initial={{ opacity: 0, y: 20 }}
			animate={{ opacity: 1, y: 0 }}
			exit={{ opacity: 0, y: 20 }}
			transition={{ duration: 0.3 }}
			className="flex justify-center mt-4"
		>
			<Button variant="outline" onClick={onClick} className="w-full max-w-md">
				Load More
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

	const evaluatedName =
		userLookup.data?.name ??
		userLookup.data?.username ??
		userLookup.data?.email ??
		"Unknown User";

	return (
		<motion.div
			key={invite.id}
			layout
			initial={{ opacity: 0, y: 20, scale: 0.95 }}
			animate={{ opacity: 1, y: 0, scale: 1 }}
			exit={{ opacity: 0, x: -100, scale: 0.95 }}
			transition={{
				duration: 0.3,
				delay: index * 0.05,
				layout: { duration: 0.3 },
			}}
			whileHover={{ y: -2 }}
			className="group"
		>
			<Card className="transition-all duration-300 hover:shadow-xl hover:shadow-primary/10 border-border bg-card/80 backdrop-blur-sm">
				<CardHeader className="pb-3">
					<div className="flex items-start justify-between">
						<div className="flex items-start gap-3">
							<motion.div
								whileHover={{ rotate: 15 }}
								transition={{ duration: 0.2 }}
								className="mt-1 p-2 bg-primary/10 rounded-lg group-hover:bg-primary/20 transition-colors"
							>
								<UserPlus className="w-5 h-5 text-primary" />
							</motion.div>
							<div>
								<CardTitle className="text-xl font-semibold text-foreground group-hover:text-primary transition-colors">
									{invite.name ?? "New Invitation"}
								</CardTitle>
								<div className="flex items-center gap-2 mt-2">
									<span className="text-sm text-muted-foreground">
										Invited by
									</span>
									<Badge variant="secondary" className="font-medium">
										{evaluatedName}
									</Badge>
									<div className="flex items-center gap-1 text-xs text-muted-foreground">
										<Clock className="w-3 h-3" />
										{formatRelativeTime(invite.created_at)}
									</div>
								</div>
							</div>
						</div>
						<Badge variant="outline" className="text-xs">
							Invitation
						</Badge>
					</div>
				</CardHeader>

				<CardContent className="pt-0">
					<p className="text-muted-foreground mb-6 leading-relaxed">
						{invite.message}
					</p>

					<div className="flex gap-3">
						<motion.div whileHover={{ scale: 1.05 }} whileTap={{ scale: 0.95 }}>
							<Button
								onClick={() => onAction(invite.id, "accept")}
								className="bg-green-600 hover:bg-green-700 text-white shadow-lg shadow-green-600/20 hover:shadow-green-600/30 transition-all"
								size="sm"
							>
								<Check className="w-4 h-4 mr-2" />
								Accept
							</Button>
						</motion.div>

						<motion.div whileHover={{ scale: 1.05 }} whileTap={{ scale: 0.95 }}>
							<Button
								onClick={() => onAction(invite.id, "decline")}
								variant="destructive"
								size="sm"
							>
								<X className="w-4 h-4 mr-2" />
								Decline
							</Button>
						</motion.div>
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
		if (!notification.link) return;
		if (!notification.read) onMarkRead(notification.id);
		if (notification.link.startsWith("http")) {
			window.open(notification.link, "_blank");
		} else {
			router.push(notification.link);
		}
	}, [notification, onMarkRead, router]);

	return (
		<motion.div
			key={notification.id}
			layout
			initial={{ opacity: 0, y: 20, scale: 0.95 }}
			animate={{ opacity: 1, y: 0, scale: 1 }}
			exit={{ opacity: 0, x: -100, scale: 0.95 }}
			transition={{
				duration: 0.3,
				delay: index * 0.05,
				layout: { duration: 0.3 },
			}}
			whileHover={{ y: -2 }}
			className="group"
		>
			<Card
				className={`transition-all duration-300 hover:shadow-xl hover:shadow-primary/10 border-border bg-card/80 backdrop-blur-sm ${
					!notification.read ? "border-l-4 border-l-primary" : ""
				}`}
			>
				<CardHeader className="pb-3">
					<div className="flex items-start justify-between">
						<div className="flex items-start gap-3">
							<motion.div
								whileHover={{ rotate: 15 }}
								transition={{ duration: 0.2 }}
								className={`mt-1 p-2 rounded-lg transition-colors ${
									notification.read
										? "bg-muted"
										: "bg-primary/10 group-hover:bg-primary/20"
								}`}
							>
								{notification.icon ? (
									<span className="text-lg">{notification.icon}</span>
								) : (
									<Workflow
										className={`w-5 h-5 ${notification.read ? "text-muted-foreground" : "text-primary"}`}
									/>
								)}
							</motion.div>
							<div>
								<CardTitle
									className={`text-xl font-semibold transition-colors ${
										notification.read
											? "text-muted-foreground"
											: "text-foreground group-hover:text-primary"
									}`}
								>
									{notification.title}
								</CardTitle>
								<div className="flex items-center gap-2 mt-2">
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
									<div className="flex items-center gap-1 text-xs text-muted-foreground">
										<Clock className="w-3 h-3" />
										{formatRelativeTime(notification.created_at)}
									</div>
									{!notification.read && (
										<Badge
											variant="outline"
											className="text-xs bg-primary/10 text-primary border-primary/20"
										>
											New
										</Badge>
									)}
								</div>
							</div>
						</div>
					</div>
				</CardHeader>

				<CardContent className="pt-0">
					{notification.description && (
						<p className="text-muted-foreground mb-4 leading-relaxed">
							{notification.description}
						</p>
					)}

					<div className="flex gap-3">
						{notification.link && (
							<motion.div
								whileHover={{ scale: 1.05 }}
								whileTap={{ scale: 0.95 }}
							>
								<Button
									onClick={handleLinkClick}
									variant="outline"
									size="sm"
									className="gap-2"
								>
									<ExternalLink className="w-4 h-4" />
									View Details
								</Button>
							</motion.div>
						)}

						{!notification.read && (
							<motion.div
								whileHover={{ scale: 1.05 }}
								whileTap={{ scale: 0.95 }}
							>
								<Button
									onClick={() => onMarkRead(notification.id)}
									variant="secondary"
									size="sm"
									className="gap-2"
								>
									<Check className="w-4 h-4" />
									Mark as read
								</Button>
							</motion.div>
						)}

						<motion.div whileHover={{ scale: 1.05 }} whileTap={{ scale: 0.95 }}>
							<Button
								onClick={() => onDelete(notification.id)}
								variant="destructive"
								size="sm"
								className="gap-2"
							>
								<Trash2 className="w-4 h-4" />
								Delete
							</Button>
						</motion.div>
					</div>
				</CardContent>
			</Card>
		</motion.div>
	);
}
