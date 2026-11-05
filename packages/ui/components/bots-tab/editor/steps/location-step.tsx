"use client";

import { useTranslation } from "@flow-like/locales";
import { useId } from "react";
import { useAccountDevices } from "../../../../hooks/use-account-devices";
import {
	type BotHostState,
	botHostState,
} from "../../../../lib/device-bot-host";
import type { DeviceRow } from "../../../../lib/devices";
import { cn } from "../../../../lib/utils";
import {
	BOT_LOCATIONS,
	type BotCapability,
	type BotLocation,
} from "../../../../state/backend-state/bot-state";
import { Label } from "../../../ui/label";
import { RadioGroup, RadioGroupItem } from "../../../ui/radio-group";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../../ui/select";
import { Skeleton } from "../../../ui/skeleton";
import { useBotHostStatus } from "../../device-host-status";
import {
	type BotCopy,
	useBotCapabilityCopy,
	useBotLocationCopy,
} from "../bot-editor-copy";
import { BotNotice, BotStepPanel } from "../bot-editor-parts";
import { BOT_LOCATION_ICONS } from "../bot-preview-panel";

const THIS_COMPUTER: readonly BotLocation[] = ["desktop"];

export function LocationStep({
	location,
	localOnly = false,
	deviceId,
	removed,
	restored,
	onLocationChange,
	onDeviceChange,
}: Readonly<{
	location: BotLocation;
	/** The bot lives only on this computer, so this computer is the one host it can have. */
	localOnly?: boolean;
	deviceId: string | null;
	/** Switched off by the last move to the cloud. */
	removed: readonly BotCapability[];
	/** Switched back on by the last move away from the cloud. */
	restored: readonly BotCapability[];
	onLocationChange: (location: BotLocation) => void;
	onDeviceChange: (deviceId: string) => void;
}>) {
	const { t } = useTranslation("bots");
	const capabilityCopy = useBotCapabilityCopy();
	const ids = useId();
	const labels = (capabilities: readonly BotCapability[]) =>
		capabilities
			.map((capability) => capabilityCopy(capability).label)
			.join(", ");
	const thisComputer: BotCopy = {
		label: t("local.location.label", "This computer"),
		description: t(
			"local.location.description",
			"Runs in the desktop app on this computer while the app is open",
		),
	};
	return (
		<BotStepPanel
			title={t("editor.location.title")}
			subtitle={t("editor.location.subtitle")}
		>
			<RadioGroup
				value={location}
				onValueChange={(next) => onLocationChange(next as BotLocation)}
				aria-label={t("editor.steps.location")}
				className={cn("grid gap-3", localOnly ? "max-w-sm" : "@xl:grid-cols-3")}
			>
				{(localOnly ? THIS_COMPUTER : BOT_LOCATIONS).map((option) => (
					<LocationCard
						key={option}
						id={`${ids}-${option}`}
						location={option}
						checked={option === location}
						copy={localOnly ? thisComputer : undefined}
					/>
				))}
			</RadioGroup>
			{localOnly && (
				<BotNotice>
					{t(
						"local.location.note",
						"Bots on the server or a device need an account.",
					)}
				</BotNotice>
			)}
			{removed.length > 0 && (
				<BotNotice tone="warning">
					{t("editor.location.turnedOff", { capabilities: labels(removed) })}
				</BotNotice>
			)}
			{restored.length > 0 && (
				<BotNotice live>
					{t("editor.location.turnedBackOn", {
						capabilities: labels(restored),
					})}
				</BotNotice>
			)}
			{location === "device" && (
				<div className="flex flex-col gap-1.5">
					<Label htmlFor={`${ids}-device`}>{t("editor.location.device")}</Label>
					<BotDevicePicker
						id={`${ids}-device`}
						value={deviceId}
						onChange={onDeviceChange}
					/>
				</div>
			)}
		</BotStepPanel>
	);
}

function LocationCard({
	id,
	location,
	checked,
	copy: override,
}: Readonly<{
	id: string;
	location: BotLocation;
	checked: boolean;
	copy?: BotCopy;
}>) {
	const locationCopy = useBotLocationCopy();
	const copy = override ?? locationCopy(location);
	const Icon = BOT_LOCATION_ICONS[location];
	return (
		<label
			htmlFor={id}
			className={cn(
				"flex cursor-pointer flex-col gap-3 rounded-xl border border-border bg-card p-4 transition-colors hover:border-primary/50 has-focus-visible:ring-2 has-focus-visible:ring-ring",
				checked && "border-primary bg-primary/5 ring-1 ring-primary/30",
			)}
		>
			<span className="flex items-center justify-between gap-2">
				<span
					className={cn(
						"grid size-9 place-items-center rounded-lg bg-muted text-muted-foreground",
						checked && "bg-primary/10 text-primary",
					)}
				>
					<Icon className="size-4.5" aria-hidden />
				</span>
				<RadioGroupItem id={id} value={location} />
			</span>
			<span className="flex flex-col gap-1">
				<span className="text-sm font-semibold">{copy.label}</span>
				<span className="text-xs leading-relaxed text-muted-foreground">
					{copy.description}
				</span>
			</span>
		</label>
	);
}

function hostDot(state: BotHostState) {
	return state === "ready" ? "text-good-solid" : "text-muted-foreground";
}

/** One device in the picker: its name and whether its agent hosts bots. */
export function BotDeviceOption({ device }: Readonly<{ device: DeviceRow }>) {
	const hostStatus = useBotHostStatus();
	const status = hostStatus(device.bot_host_seen_at);
	return (
		<span className="flex min-w-0 flex-col">
			<span className="truncate">{device.display_name || device.name}</span>
			<span
				data-host-state={status.state}
				className={cn("truncate text-xs", hostDot(status.state))}
			>
				{status.label}
			</span>
		</span>
	);
}

/** The signed-in user's own active devices; a bot can only live on a device its owner holds. */
function BotDevicePicker({
	id,
	value,
	onChange,
}: Readonly<{
	id: string;
	value: string | null;
	onChange: (deviceId: string) => void;
}>) {
	const { t } = useTranslation("bots");
	const { devices, isLoading, isError } = useAccountDevices();
	if (isLoading) return <Skeleton className="h-9 w-full" />;
	if (isError)
		return (
			<p role="alert" className="text-xs text-destructive">
				{t("editor.location.deviceFailed")}
			</p>
		);
	const owned = (devices ?? []).filter(
		(device) =>
			device.status === "active" &&
			(device.relationship ?? "owner") === "owner",
	);
	if (!owned.length)
		return (
			<p className="text-xs text-muted-foreground">
				{t("editor.location.deviceEmpty")}
			</p>
		);
	const chosen = owned.find((device) => device.device_id === value);
	return (
		<>
			<Select value={value ?? ""} onValueChange={onChange}>
				<SelectTrigger id={id} className="w-full">
					<SelectValue placeholder={t("editor.location.devicePlaceholder")}>
						{chosen ? chosen.display_name || chosen.name : null}
					</SelectValue>
				</SelectTrigger>
				<SelectContent>
					{owned.map((device) => (
						<SelectItem key={device.device_id} value={device.device_id}>
							<BotDeviceOption device={device} />
						</SelectItem>
					))}
				</SelectContent>
			</Select>
			{chosen && botHostState(chosen.bot_host_seen_at) !== "ready" && (
				<BotNotice>
					{t(
						"editor.location.hostNotice",
						"This device isn't hosting bots right now. The bot will answer once the agent runs there.",
					)}
				</BotNotice>
			)}
		</>
	);
}
