"use client";

import { useTranslation } from "@flow-like/locales";
import { KeyRound, Lock, LockOpen } from "lucide-react";
import { useMemo } from "react";
import { toast } from "sonner";
import { deviceName } from "../../../../lib/device-management/model/device-view";
import type {
	AttentionInput,
	DevicesScope,
} from "../../../../lib/device-management/model/types";
import type {
	KeySessionSnapshot,
	LiveState,
} from "../../../../lib/device-management/workspace/types";
import { DvButton } from "../primitives/dv-button";
import { KeyChip, type KeyChipState } from "../primitives/status-chip";
import { useAttentionState, useKeyChip, useOverlayStore } from "../workspace";
import type { ChromeNavigate } from "./rail-row";

export interface KeysPopoverProps {
	scope: DevicesScope;
	onNavigate: ChromeNavigate;
	/** Called after a link or action was chosen so the popover closes. */
	onClose?: () => void;
}

/** A device with keys on this computer. */
export interface KeySessionRow {
	deviceId: string;
	name: string;
	state: KeyChipState;
	transport?: "direct" | "relayed";
	/** Unix seconds of the next connection renewal (direct only). */
	renewsAt?: number;
}

const OPEN_STATES = new Set<KeyChipState>(["live", "reconnecting", "unlocked"]);
const NO_ACTION = new Set<KeyChipState>(["unlocking", "blocked", "stale"]);

/** Keys are open on this computer: the row offers Lock. */
export const isKeySessionOpen = (state: KeyChipState) => OPEN_STATES.has(state);

export interface KeysPopoverViewProps {
	sessions: readonly KeySessionRow[];
	/** Minutes of no use after which an unlocked device locks. */
	idleMinutes?: number;
	onLock?: (deviceId: string) => void;
	onUnlock?: (deviceId: string) => void;
	onUnlockSeveral?: () => void;
	onLockAll?: () => void;
	onOpenKeys?: () => void;
}

interface SessionRowProps
	extends Pick<KeysPopoverViewProps, "onLock" | "onUnlock"> {
	session: KeySessionRow;
}

function SessionAction({
	session,
	onLock,
	onUnlock,
}: Readonly<SessionRowProps>) {
	const { t } = useTranslation("devices");
	if (NO_ACTION.has(session.state)) return null;
	if (isKeySessionOpen(session.state))
		return (
			<DvButton
				size="xs"
				icon={Lock}
				aria-label={t("chrome.keys.lockNamed", "Lock {{device}}", {
					device: session.name,
				})}
				onClick={() => onLock?.(session.deviceId)}
			>
				{t("chrome.keys.lock", "Lock")}
			</DvButton>
		);
	return (
		<DvButton
			size="xs"
			icon={LockOpen}
			aria-label={t("chrome.keys.unlockNamed", "Unlock {{device}}…", {
				device: session.name,
			})}
			onClick={() => onUnlock?.(session.deviceId)}
		>
			{session.state === "held_elsewhere"
				? t("chrome.keys.useHere", "Use here…")
				: t("chrome.keys.unlock", "Unlock…")}
		</DvButton>
	);
}

function SessionRow(props: Readonly<SessionRowProps>) {
	const { session } = props;
	return (
		<li
			data-key-session={session.deviceId}
			className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 border-t border-hairline py-1.5 first:border-t-0"
		>
			<span
				title={session.name}
				className="min-w-[10ch] flex-1 truncate font-mono text-[12.5px]/[18px] font-medium"
			>
				{session.name}
			</span>
			<KeyChip
				state={session.state}
				transport={session.transport}
				renewsAt={session.renewsAt}
			/>
			<SessionAction {...props} />
		</li>
	);
}

/** The key sessions popover over plain rows. */
export function KeysPopoverView(props: Readonly<KeysPopoverViewProps>) {
	const { t } = useTranslation("devices");
	const { sessions, idleMinutes = 30 } = props;
	const anyOpen = sessions.some((session) => isKeySessionOpen(session.state));
	return (
		<div data-chrome="keys-popover" className="flex min-w-0 flex-col">
			<div className="flex flex-col gap-1.5 px-4 pt-3 pb-2">
				<h3 className="text-ui font-semibold tracking-normal">
					{t("chrome.keys.head", "Key sessions on this computer")}
				</h3>
				{sessions.length ? (
					<ul className="m-0 flex list-none flex-col p-0">
						{sessions.map((session) => {
							return (
								<SessionRow
									key={session.deviceId}
									session={session}
									onLock={props.onLock}
									onUnlock={props.onUnlock}
								/>
							);
						})}
					</ul>
				) : (
					<p className="text-ui text-muted-foreground">
						{t("chrome.keys.none", "No device keys on this computer.")}
					</p>
				)}
			</div>
			<div className="flex flex-col gap-1.5 border-t border-hairline px-4 py-2.5">
				<div className="flex flex-wrap gap-2">
					<DvButton size="sm" icon={LockOpen} onClick={props.onUnlockSeveral}>
						{t("chrome.keys.unlockSeveral", "Unlock several…")}
					</DvButton>
					<DvButton
						size="sm"
						variant="ghost"
						icon={Lock}
						aria-disabled={!anyOpen || undefined}
						onClick={props.onLockAll}
					>
						{t("chrome.keys.lockAll", "Lock all")}
					</DvButton>
					<DvButton
						size="sm"
						variant="ghost"
						icon={KeyRound}
						onClick={props.onOpenKeys}
					>
						{t("chrome.keys.keysAndRecovery", "Keys & recovery")}
					</DvButton>
				</div>
				<p className="text-xs text-muted-foreground">
					{t(
						"chrome.keys.idleHint",
						"Unlocked devices lock after {{count, number}} min unused. Locking clears logs and decrypted history from this window.",
						{ count: idleMinutes },
					)}
				</p>
			</div>
		</div>
	);
}

/* Binding. */

const LIVE_OPEN = new Set<LiveState["kind"]>(["live", "renewing"]);
const LIVE_RETRYING = new Set<LiveState["kind"]>([
	"connecting",
	"reconnecting",
	"unreachable",
]);

/** The key chip of a device: its key session, with the live connection on top of open keys. */
export function keyChipOf(
	keys: Pick<KeySessionSnapshot, "state">,
	live: LiveState | undefined,
): Pick<KeySessionRow, "state" | "transport"> {
	if (keys.state !== "unlocked" || !live) return { state: keys.state };
	if (LIVE_RETRYING.has(live.kind)) return { state: "reconnecting" };
	if (!LIVE_OPEN.has(live.kind)) return { state: "unlocked" };
	const transport = "transport" in live ? live.transport : undefined;
	return {
		state: "live",
		transport: transport === "websocket" ? "relayed" : "direct",
	};
}

/** Every device with keys on this computer: open sessions first, then by name. */
export function keySessionRows(
	sessions: readonly KeySessionSnapshot[],
	input: Pick<AttentionInput, "devices" | "live">,
): KeySessionRow[] {
	const names = new Map(
		input.devices.map((row) => [row.device_id, deviceName(row)]),
	);
	return sessions
		.filter((session) => session.state !== "none")
		.map((session) => ({
			deviceId: session.deviceId,
			name: names.get(session.deviceId) ?? session.deviceId.slice(0, 8),
			...keyChipOf(session, input.live[session.deviceId]?.state),
		}))
		.sort(
			(a, b) =>
				Number(isKeySessionOpen(b.state)) - Number(isKeySessionOpen(a.state)) ||
				a.name.localeCompare(b.name),
		);
}

/** SPEC §3.2 item 7: key sessions on this computer with Lock, Unlock several… and Lock all. */
export function KeysPopover({
	onNavigate,
	onClose,
}: Readonly<KeysPopoverProps>) {
	const { t } = useTranslation("devices");
	const chip = useKeyChip();
	const { input } = useAttentionState();
	const sessions = useMemo(
		() => keySessionRows(chip.sessions, input),
		[chip.sessions, input],
	);
	const overlay = useOverlayStore.getState();
	const lock = (deviceId: string) => {
		onClose?.();
		chip.lock(deviceId);
		const name = sessions.find((row) => row.deviceId === deviceId)?.name;
		toast(
			t(
				"chrome.keys.lockedToast",
				"{{device}} locked. Logs and decrypted history were cleared from this window.",
				{ device: name ?? deviceId.slice(0, 8) },
			),
		);
	};
	const lockAll = () => {
		onClose?.();
		chip.lockAll();
		toast(
			t(
				"chrome.keys.lockedAllToast",
				"All devices are locked on this computer.",
			),
		);
	};
	return (
		<KeysPopoverView
			sessions={sessions}
			onLock={lock}
			onUnlock={(deviceId) => {
				onClose?.();
				overlay.openUnlock(deviceId);
			}}
			onUnlockSeveral={() => {
				onClose?.();
				overlay.openUnlockSeveral();
			}}
			onLockAll={lockAll}
			onOpenKeys={() => {
				onClose?.();
				onNavigate({ screen: "keys" });
			}}
		/>
	);
}
