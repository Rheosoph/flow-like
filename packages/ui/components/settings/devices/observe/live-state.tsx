"use client";

import { useTranslation } from "@flow-like/locales";
import { KeyRound, LockOpen, Radio, RefreshCw, SquareX } from "lucide-react";
import { type ReactNode, useState } from "react";
import { DvButton } from "../primitives/dv-button";
import { GateNotice } from "../primitives/gate-notice";
import { StateView } from "../primitives/state-view";
import { ACCOUNT_SCOPE } from "../routing/devices-route";
import { useRouteLink } from "../routing/use-devices-route";
import { useLiveSession, useOverlay } from "../workspace";
import type { ObserveTarget } from "./use-observe-target";

export type LiveWhat = "activity" | "logs" | "metrics" | "history";

/** Why a live read can't run now; `open` when a session is up. */
export type LivePhase =
	| "revoked"
	| "nokeys"
	| "locked"
	| "idle"
	| "connecting"
	| "reconnecting"
	| "unreachable"
	| "failed"
	| "open";

export function livePhase(target: ObserveTarget): LivePhase {
	if (target.revoked) return "revoked";
	if (!target.hasKeys) return "nokeys";
	if (target.locked) return "locked";
	const kind = target.live.kind;
	if (kind === "live" || kind === "renewing") return "open";
	return kind;
}

/**
 * Streams subscribe only while a session is wanted already (open, or being
 * opened or retried): a tab never opens a live connection on its own, and data
 * read before a dropped connection stays on screen while it reconnects.
 */
export function liveWanted(target: ObserveTarget): boolean {
	const phase = livePhase(target);
	return (
		phase === "open" ||
		phase === "connecting" ||
		phase === "reconnecting" ||
		phase === "unreachable"
	);
}

/** "Connect live": opens a live session with the unlocked keys and keeps it while they stay unlocked. */
export function ConnectButton({
	deviceId,
	size = "sm",
}: Readonly<{ deviceId: string; size?: "sm" | "xs" }>) {
	const { t } = useTranslation("devices");
	const session = useLiveSession(deviceId);
	const [busy, setBusy] = useState(false);
	return (
		<DvButton
			size={size}
			icon={Radio}
			busy={busy}
			onClick={() => {
				setBusy(true);
				void session
					.connect()
					.catch(() => undefined)
					.finally(() => setBusy(false));
			}}
		>
			{t("observe.live.connect", "Connect live")}
		</DvButton>
	);
}

function RetryButton({ deviceId }: Readonly<{ deviceId: string }>) {
	const { t } = useTranslation("devices");
	const session = useLiveSession(deviceId);
	const [busy, setBusy] = useState(false);
	return (
		<DvButton
			size="sm"
			icon={RefreshCw}
			busy={busy}
			onClick={() => {
				setBusy(true);
				void session
					.retry()
					.catch(() => undefined)
					.finally(() => setBusy(false));
			}}
		>
			{t("observe.live.retry", "Try again")}
		</DvButton>
	);
}

/**
 * The state shown instead of live data (R6: never "empty" for locked, no keys,
 * not connected or failed). `null` when a session is open and the caller renders
 * its data or its own "nothing yet".
 */
export function LiveDataState({
	target,
	what,
}: Readonly<{ target: ObserveTarget; what: LiveWhat }>): ReactNode {
	const { t } = useTranslation("devices");
	const overlay = useOverlay();
	const link = useRouteLink();
	const phase = livePhase(target);
	const device = target.name;
	const offline =
		target.presence?.kind === "offline" || target.presence?.kind === "never";

	if (phase === "open") return null;
	if (phase === "revoked")
		return (
			<StateView
				kind="notloaded"
				icon={SquareX}
				title={t("observe.live.revokedTitle", "Not available")}
				text={t("observe.live.revokedText", "Revoked devices aren't read.")}
			/>
		);
	if (phase === "nokeys")
		return (
			<GateNotice
				kind="nokeys"
				title={t(
					"observe.live.noKeys",
					"This computer has no keys for {{device}}.",
					{ device },
				)}
				text={t(
					"observe.live.noKeysText",
					"Activity, logs and metrics are encrypted for keys on a computer; the hub can't read them.",
				)}
				actions={
					<DvButton asChild size="sm" icon={KeyRound}>
						<a
							{...link(
								{ screen: "keys", focusDeviceId: target.deviceId },
								{ scope: ACCOUNT_SCOPE },
							)}
						>
							{t("observe.live.restoreKeys", "Restore keys…")}
						</a>
					</DvButton>
				}
			/>
		);
	if (phase === "locked") {
		const texts: Record<LiveWhat, string> = {
			activity: t(
				"observe.live.lockedActivity",
				"Activity is cleared when you lock. Unlock to read it again.",
			),
			logs: t(
				"observe.live.lockedLogs",
				"Logs are cleared when you lock. Unlock to read them again.",
			),
			metrics: t(
				"observe.live.lockedMetrics",
				"Live resource values need the keys. Unlock to read them.",
			),
			history: t(
				"observe.live.lockedHistory",
				"History settings are read from the device. Unlock to read them.",
			),
		};
		return (
			<StateView
				kind="locked"
				title={t("observe.live.lockedTitle", "Locked")}
				text={texts[what]}
				actions={
					<DvButton
						size="sm"
						icon={LockOpen}
						onClick={() =>
							overlay.openUnlock(target.deviceId, { connectLive: true })
						}
					>
						{t("observe.live.unlock", "Unlock…")}
					</DvButton>
				}
			/>
		);
	}
	if (phase === "connecting")
		return (
			<StateView
				kind="loading"
				title={t("observe.live.connecting", "Connecting to {{device}}…", {
					device,
				})}
			/>
		);
	if (phase === "failed")
		return (
			<StateView
				kind="error"
				title={t(
					"observe.live.failedTitle",
					"The live connection to {{device}} failed",
					{ device },
				)}
				text={t(
					"observe.live.failedText",
					"Nothing was read. Your keys are still unlocked.",
				)}
				actions={<RetryButton deviceId={target.deviceId} />}
			/>
		);
	if (phase === "reconnecting")
		return (
			<StateView
				kind="notloaded"
				title={t("observe.live.reconnecting", "Reconnecting…")}
				text={t(
					"observe.live.reconnectingText",
					"The live connection dropped. Your keys are still unlocked; reading continues once {{device}} answers.",
					{ device },
				)}
			/>
		);

	const offlineTexts: Record<LiveWhat, string> = {
		activity: t(
			"observe.live.offlineActivity",
			"{{device}} is offline. Activity is read live from the device.",
			{ device },
		),
		logs: t(
			"observe.live.offlineLogs",
			"{{device}} is offline. Logs are read live from the device; retained history may still have them.",
			{ device },
		),
		metrics: t(
			"observe.live.offlineMetrics",
			"{{device}} is offline. Live values are read from the device.",
			{ device },
		),
		history: t(
			"observe.live.offlineHistory",
			"{{device}} is offline. History settings are read live from the device.",
			{ device },
		),
	};
	const connectTexts: Record<LiveWhat, string> = {
		activity: t(
			"observe.live.connectActivity",
			"Activity comes from {{device}} over a live connection.",
			{ device },
		),
		logs: t(
			"observe.live.connectLogs",
			"Connect live to {{device}} to follow its logs.",
			{ device },
		),
		metrics: t(
			"observe.live.connectMetrics",
			"Live values are read from {{device}} over a live connection.",
			{ device },
		),
		history: t(
			"observe.live.connectHistory",
			"History settings are read from {{device}} over a live connection.",
			{ device },
		),
	};
	return (
		<StateView
			kind="notloaded"
			title={t("observe.live.needsLive", "Needs a live connection")}
			text={offline ? offlineTexts[what] : connectTexts[what]}
			actions={
				offline || phase === "unreachable" ? null : (
					<ConnectButton deviceId={target.deviceId} />
				)
			}
		/>
	);
}
