"use client";

import { useTranslation } from "@flow-like/locales";
import { BellIcon, SettingsIcon, UserRoundXIcon } from "lucide-react";
import { memo, useMemo } from "react";
import { useInvoke } from "../../../hooks/use-invoke";
import { userDisplayName, userInitials } from "../../../lib/user-display";
import { cn } from "../../../lib/utils";
import {
	useBackend,
	useBackendReady,
	useSignedIn,
} from "../../../state/backend-state";
import { Avatar, AvatarFallback, AvatarImage } from "../../ui/avatar";
import { Popover, PopoverContent, PopoverTrigger } from "../../ui/popover";

/**
 * Who you are signed in as, pinned to the bottom of the board rail.
 *
 * The board owns the window, so the global sidebar — and with it the avatar
 * that told you at a glance whether the session was live — is not mounted here.
 * A board that silently ran signed-out looked identical to one that did not.
 */
export const BoardAccountItem = memo(function BoardAccountItem({
	onOpenSettings,
	onOpenNotifications,
}: Readonly<{
	onOpenSettings: () => void;
	onOpenNotifications: () => void;
}>) {
	const { t } = useTranslation("flow");
	const backend = useBackend();
	const signedIn = useSignedIn();
	const backendReady = useBackendReady();
	const info = useInvoke(
		backend.userState.getInfo,
		backend.userState,
		[],
		signedIn,
	);
	// Local counts are served offline too, so this stays enabled while signed
	// out — but not against the prerender placeholder backend, whose states throw.
	const notifications = useInvoke(
		backend.userState.getNotifications,
		backend.userState,
		[],
		backendReady,
	);
	const unread =
		(notifications.data?.unread_count ?? 0) +
		(notifications.data?.invites_count ?? 0);

	const displayName = useMemo(
		() => userDisplayName(info.data, t("offline", "Offline")),
		[info.data, t],
	);
	const initials = useMemo(() => userInitials(displayName, "?"), [displayName]);

	return (
		<Popover>
			<PopoverTrigger asChild>
				<button
					type="button"
					aria-label={displayName}
					title={displayName}
					className="relative flex h-10 w-11 shrink-0 items-center justify-center"
				>
					<Avatar className="size-6 rounded-md">
						<AvatarImage src={info.data?.avatar} alt={displayName} />
						<AvatarFallback className="rounded-md text-[9px]">
							{initials}
						</AvatarFallback>
					</Avatar>
					{unread > 0 ? (
						<span className="absolute right-1 top-1 min-w-3.5 rounded-full bg-primary px-1 text-[9px] font-semibold leading-3.5 tabular-nums text-primary-foreground">
							{unread > 99 ? "99+" : unread}
						</span>
					) : (
						<span
							aria-hidden="true"
							className={cn(
								"absolute bottom-1.5 right-2 size-2 rounded-full border border-background",
								signedIn ? "bg-emerald-500" : "bg-muted-foreground",
							)}
						/>
					)}
				</button>
			</PopoverTrigger>
			<PopoverContent
				side="right"
				align="end"
				sideOffset={8}
				className="w-60 p-2"
			>
				<div className="flex items-center gap-2 px-1 py-1.5">
					<Avatar className="size-8 rounded-md">
						<AvatarImage src={info.data?.avatar} alt={displayName} />
						<AvatarFallback className="rounded-md text-xs">
							{initials}
						</AvatarFallback>
					</Avatar>
					<div className="min-w-0">
						<p className="truncate text-xs font-medium">{displayName}</p>
						<p className="truncate text-[11px] text-muted-foreground">
							{info.data?.email ?? t("signedOut", "Signed out")}
						</p>
					</div>
				</div>
				{!signedIn && (
					<p className="flex items-center gap-1.5 rounded-sm bg-muted/50 px-2 py-1.5 text-[11px] text-muted-foreground">
						<UserRoundXIcon className="size-3 shrink-0" />
						{t(
							"signedOutBoardHint",
							"Running locally — cloud features are unavailable.",
						)}
					</p>
				)}
				<button
					type="button"
					onClick={onOpenNotifications}
					className="mt-1 flex w-full items-center gap-2 rounded-sm px-2 py-1.5 text-left text-xs hover:bg-accent hover:text-accent-foreground"
				>
					<BellIcon className="size-3.5 shrink-0 text-muted-foreground" />
					<span className="flex-1">{t("notifications", "Notifications")}</span>
					{unread > 0 && (
						<span className="rounded-full bg-primary px-1.5 text-[10px] font-semibold tabular-nums text-primary-foreground">
							{unread > 99 ? "99+" : unread}
						</span>
					)}
				</button>
				<button
					type="button"
					onClick={onOpenSettings}
					className="flex w-full items-center gap-2 rounded-sm px-2 py-1.5 text-left text-xs hover:bg-accent hover:text-accent-foreground"
				>
					<SettingsIcon className="size-3.5 shrink-0 text-muted-foreground" />
					{t("settings", "Settings")}
				</button>
			</PopoverContent>
		</Popover>
	);
});
