"use client";

import { useTranslation } from "@flow-like/locales";
import { HardDrive } from "lucide-react";
import {
	deviceOfModelBit,
	isDeviceModelBit,
} from "../../../../../lib/bit/local-model-filter";
import type { IBit } from "../../../../../lib/schema/bit/bit";
import { cn } from "../../../../../lib/utils";
import type { DevicesT } from "../../primitives/area-context";
import {
	type DeviceKeyState,
	type HeldDevices,
	useDeviceKeyState,
} from "./device-key-state";

function badgeCopy(t: DevicesT, state: DeviceKeyState | undefined) {
	switch (state) {
		case "unlocked":
			return {
				text: t("devices:models.use.badgeUnlocked", "Device · unlocked"),
				title: t(
					"devices:models.use.badgeUnlockedTitle",
					"Runs on one of your devices, which is unlocked in this app: desktop runs call it without asking for its password. Cloud runs skip it.",
				),
			};
		case "locked":
			return {
				text: t("devices:models.use.badgeLocked", "Device · locked"),
				title: t(
					"devices:models.use.badgeLockedTitle",
					"Runs on one of your devices, which is locked in this app: a desktop run asks for its password first. Cloud runs skip it.",
				),
			};
		default:
			return {
				text: t("devices:models.use.badge", "Device"),
				title: t(
					"devices:models.use.badgeTitle",
					"Runs on one of your devices. Desktop runs reach it through an encrypted tunnel; cloud runs skip it.",
				),
			};
	}
}

/**
 * Marks a Bit whose model runs on one of the user's devices (plan §3.8),
 * wherever models are picked. With `deviceId`, the desktop app also says
 * whether a run reaches the device without asking for its password.
 */
export function DeviceModelBadge({
	deviceId,
	held,
	className,
}: Readonly<{
	deviceId?: string;
	/** The desktop app's held keys; tests replace it. */
	held?: HeldDevices;
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	const state = useDeviceKeyState(deviceId, held);
	const copy = badgeCopy(t, state);
	return (
		<span
			data-device-model=""
			data-device-state={state}
			title={copy.title}
			className={cn(
				"inline-flex shrink-0 items-center gap-0.5 rounded border border-border px-1 text-[10px]/4 font-medium text-muted-foreground",
				state === "unlocked" && "border-primary/40 text-primary",
				className,
			)}
		>
			<HardDrive aria-hidden className="size-2.5" />
			{copy.text}
		</span>
	);
}

/** The badge of a device Bit, with its device's state; nothing for any other Bit. */
export function DeviceBitBadge({ bit }: Readonly<{ bit: IBit }>) {
	return isDeviceModelBit(bit) ? (
		<DeviceModelBadge deviceId={deviceOfModelBit(bit)} />
	) : null;
}
