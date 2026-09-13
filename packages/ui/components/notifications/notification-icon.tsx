"use client";

import { useQuery } from "@tanstack/react-query";
import { DynamicIcon, type IconName } from "lucide-react/dynamic";
import { useState } from "react";
import { useAuth } from "react-oidc-context";
import { getApiOrigin } from "../../lib/api-url";
import { notificationIconCandidates } from "../../lib/notification-icon";
import { cn } from "../../lib/utils";
import { useBackend, useBackendReady } from "../../state/backend-state";

function useNotificationAppIcon(appId?: string) {
	const backend = useBackend();
	const ready = useBackendReady();
	const auth = useAuth();
	const profileId = backend.profile?.id;
	const enabled = Boolean(
		appId &&
			ready &&
			profileId &&
			!auth.isLoading &&
			(!auth.isAuthenticated || auth.user?.profile.sub),
	);
	const icons = useQuery({
		queryKey: [
			"notification-app-icons",
			getApiOrigin(backend.profile),
			profileId,
			(auth.isAuthenticated ? auth.user?.profile.sub : undefined) ?? "local",
		],
		enabled,
		staleTime: 60_000,
		queryFn: async () => {
			const [profile, apps] = await Promise.all([
				backend.userState.getProfile(),
				backend.appState.getApps(),
			]);
			if (profile.id !== profileId) return {} as Record<string, string>;
			const visible = new Set(profile.apps?.map((app) => app.app_id));
			return Object.fromEntries(
				apps.flatMap(([app, meta]) =>
					visible.has(app.id) && typeof meta?.icon === "string" && meta.icon
						? [[app.id, meta.icon]]
						: [],
				),
			) as Record<string, string>;
		},
	});
	return enabled && appId ? icons.data?.[appId] : undefined;
}

export function NotificationIcon({
	icon,
	appId,
	read = false,
	className,
}: {
	icon?: string | null;
	appId?: string;
	read?: boolean;
	className?: string;
}) {
	const appIcon = useNotificationAppIcon(appId);
	return (
		<NotificationIconArtwork
			key={JSON.stringify([icon, appIcon])}
			icon={icon}
			appIcon={appIcon}
			read={read}
			className={className}
		/>
	);
}

/** The same fallback chain is used by inbox rows, Home widgets, and toasts. */
export function NotificationIconArtwork({
	icon,
	appIcon,
	read = false,
	className,
}: {
	icon?: string | null;
	appIcon?: string;
	read?: boolean;
	className?: string;
}) {
	const [failed, setFailed] = useState<string[]>([]);
	const source = notificationIconCandidates(icon, appIcon).find(
		(item) => !failed.includes(item.value),
	);
	const classes = cn(
		"size-6 shrink-0",
		read && "opacity-70 grayscale",
		className,
	);
	const fallback = () => (
		<NotificationIconArtwork
			appIcon={
				source?.value ===
				(typeof appIcon === "string" ? appIcon.trim() : undefined)
					? undefined
					: appIcon
			}
			read={read}
			className={className}
		/>
	);
	if (source?.kind === "image")
		return (
			<img
				key={source.value}
				src={source.value}
				alt=""
				aria-hidden="true"
				className={cn("rounded-sm object-contain", classes)}
				referrerPolicy="no-referrer"
				onError={() => setFailed((values) => [...values, source.value])}
			/>
		);
	if (source?.kind === "lucide")
		return (
			<DynamicIcon
				name={source.value as IconName}
				aria-hidden="true"
				className={cn(read ? "text-muted-foreground" : "text-primary", classes)}
				fallback={fallback}
			/>
		);
	if (source?.kind === "emoji")
		return (
			<span
				aria-hidden="true"
				className={cn(
					"flex items-center justify-center text-[1.3em] leading-none",
					classes,
				)}
			>
				{source.value}
			</span>
		);
	return (
		<span
			aria-hidden="true"
			className={cn(
				"flex items-center justify-center rounded-sm bg-orange-500/10 text-[10px] font-bold text-orange-600",
				classes,
			)}
		>
			FL
		</span>
	);
}
