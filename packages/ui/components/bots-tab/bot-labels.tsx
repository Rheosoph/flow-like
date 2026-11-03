"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CloudIcon,
	CpuIcon,
	HardDriveIcon,
	LockIcon,
	MonitorIcon,
} from "lucide-react";
import { useCallback, useMemo } from "react";
import { cn } from "../../lib/utils";
import type {
	BotCapability,
	BotLocation,
	BotPresenceState,
	BotSystemKey,
} from "../../state/backend-state/bot-state";

export const BOT_LOCATION_ICONS = {
	server: CloudIcon,
	desktop: MonitorIcon,
	device: CpuIcon,
} as const satisfies Record<BotLocation, typeof CloudIcon>;

export function useBotLocationText() {
	const { t } = useTranslation("bots");
	const texts = useMemo<
		Record<BotLocation, { label: string; description: string }>
	>(
		() => ({
			server: {
				label: t("locations.server.label", "Cloud"),
				description: t(
					"locations.server.description",
					"Runs on Flow-Like's servers, around the clock",
				),
			},
			desktop: {
				label: t("locations.desktop.label", "Desktop app"),
				description: t(
					"locations.desktop.description",
					"Runs in the desktop app while it is open",
				),
			},
			device: {
				label: t("locations.device.label", "Device"),
				description: t(
					"locations.device.description",
					"Runs on one of your managed devices",
				),
			},
		}),
		[t],
	);
	return useCallback(
		(location: BotLocation) => texts[location] ?? texts.server,
		[texts],
	);
}

export interface BotStatus {
	label: string;
	/** A built-in bot, there whenever you chat; the others say where they run. */
	ready: boolean;
}

export function useBotStatusText() {
	const { t } = useTranslation("bots");
	return useCallback(
		(system: BotSystemKey | null, location?: BotLocation): BotStatus => {
			if (system || !location)
				return { label: t("crew.status.ready", "Ready"), ready: true };
			if (location === "desktop")
				return {
					label: t("crew.status.desktop", "Runs in the desktop app"),
					ready: false,
				};
			if (location === "device")
				return {
					label: t("crew.status.device", "Runs on a managed device"),
					ready: false,
				};
			return {
				label: t("crew.status.server", "Runs in the cloud"),
				ready: false,
			};
		},
		[t],
	);
}

export function useBotPresenceText() {
	const { t } = useTranslation("bots");
	return useCallback(
		(state: BotPresenceState): string =>
			({
				online: t("presence.online", "Online"),
				offline: t("presence.offline", "Offline"),
				unknown: t("presence.unknown", "Not seen yet"),
			})[state],
		[t],
	);
}

export function useBotCapabilityLabel() {
	const { t } = useTranslation("bots");
	const labels = useMemo<Record<BotCapability, string>>(
		() => ({
			internet: t("capabilities.internet.label", "Internet"),
			profile_apps: t(
				"capabilities.profile_apps.label",
				"All apps in a profile",
			),
			selected_apps: t("capabilities.selected_apps.label", "Selected apps"),
			code_execution: t("capabilities.code_execution.label", "Run code"),
			computer_use: t("capabilities.computer_use.label", "Computer use"),
			browser_use: t("capabilities.browser_use.label", "Browser"),
			email: t("capabilities.email.label", "Email"),
			push_notifications: t(
				"capabilities.push_notifications.label",
				"Notify you",
			),
			scheduling: t("capabilities.scheduling.label", "Scheduling"),
			memory: t("capabilities.memory.label", "Memory"),
			documents: t("capabilities.documents.label", "Documents"),
			data_connections: t(
				"capabilities.data_connections.label",
				"Data connections",
			),
			knowledge: t("capabilities.knowledge.label", "Knowledge"),
			connectors: t("capabilities.connectors.label", "Connectors"),
			channels: t("capabilities.channels.label", "Channels"),
			skills: t("capabilities.skills.label", "Skills"),
			image_generation: t(
				"capabilities.image_generation.label",
				"Image generation",
			),
			delegation: t("capabilities.delegation.label", "Delegation"),
			storage: t("capabilities.storage.label", "Files"),
			data_studio: t("capabilities.data_studio.label", "Data Studio"),
			command_line: t("capabilities.command_line.label", "Command line"),
			encryption: t("capabilities.encryption.label", "End-to-end encryption"),
			webhook: t("capabilities.webhook.label", "Webhook"),
			flowpilot: t("capabilities.flowpilot.label", "FlowPilot"),
		}),
		[t],
	);
	return useCallback(
		(capability: BotCapability) => labels[capability] ?? capability,
		[labels],
	);
}

const PILL =
	"inline-flex h-5.5 items-center gap-1 rounded-full px-2 text-[11px] font-medium whitespace-nowrap";

export function BotLocationBadge({
	location,
	className,
}: Readonly<{ location: BotLocation; className?: string }>) {
	const locationText = useBotLocationText();
	const Icon = BOT_LOCATION_ICONS[location] ?? CloudIcon;
	const { label, description } = locationText(location);
	return (
		<span
			className={cn(PILL, "bg-muted text-muted-foreground", className)}
			title={description}
		>
			<Icon className="size-3" aria-hidden />
			{label}
		</span>
	);
}

/** A bot that lives only on this computer until it moves to an account. */
export function LocalBadge({ className }: Readonly<{ className?: string }>) {
	const { t } = useTranslation("bots");
	return (
		<span
			className={cn(PILL, "bg-muted text-muted-foreground", className)}
			title={t(
				"local.badgeHint",
				"Lives only on this computer until you move it to an account.",
			)}
		>
			<HardDriveIcon className="size-3" aria-hidden />
			{t("local.badge", "On this device")}
		</span>
	);
}

/** The quiet marker of a chat that lives only on this computer. */
export function LocalConversationMark({
	className,
}: Readonly<{ className?: string }>) {
	const { t } = useTranslation("bots");
	const label = t(
		"local.conversationHint",
		"This chat lives only on this computer.",
	);
	return (
		<span
			role="img"
			aria-label={label}
			title={label}
			className={cn("inline-flex shrink-0 text-muted-foreground", className)}
		>
			<HardDriveIcon className="size-3.5" aria-hidden />
		</span>
	);
}

export function BuiltInBadge({ className }: Readonly<{ className?: string }>) {
	const { t } = useTranslation("bots");
	return (
		<span
			className={cn(
				PILL,
				"border border-border text-muted-foreground",
				className,
			)}
		>
			{t("crew.builtIn", "Built in")}
		</span>
	);
}

export function EncryptedBadge({
	className,
}: Readonly<{ className?: string }>) {
	const { t } = useTranslation("bots");
	const label = t("crew.encrypted", "End-to-end encrypted");
	return (
		<span
			role="img"
			aria-label={label}
			title={label}
			className={cn(
				"inline-flex size-6 items-center justify-center rounded-full bg-muted text-muted-foreground",
				className,
			)}
		>
			<LockIcon className="size-3" aria-hidden />
		</span>
	);
}
