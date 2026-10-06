"use client";

import { useTranslation } from "@flow-like/locales";
import { Boxes, LockOpen, RefreshCw } from "lucide-react";
import { useState } from "react";
import type {
	FixAction,
	GateFailure,
	GateResult,
} from "../../../../lib/device-management/model/types";
import type { DeviceFailure } from "../../../../lib/device-management/workspace/errors";
import { errorCopy } from "../copy/error-copy";
import { fixLabel, gateCopy } from "../copy/gate-copy";
import { gateView } from "../device/use-device-page";
import { ConnectButton, LiveDataState, livePhase } from "../observe/live-state";
import type { ObserveTarget } from "../observe/use-observe-target";
import { useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { GateNotice } from "../primitives/gate-notice";
import { monoNames } from "../primitives/obj-name";
import { LOCKED_DATA_CLASS, StateView } from "../primitives/state-view";
import { cx } from "../primitives/tone";
import { useDevicesRoute } from "../routing/use-devices-route";
import { stampOf, useRunAttentionTarget } from "../shell/attention-popover";
import { useOverlay } from "../workspace";
import { headlineCopy } from "./models-copy";
import { summaryHeadline } from "./models-view";
import { useModelsSnapshot } from "./use-models";

/*
 * What the Models tab shows instead of its data (R6: never "empty" for
 * locked, not connected, an older agent, no access or a failed read).
 */

function FixButton({ gate }: Readonly<{ gate: GateFailure }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { navigate } = useDevicesRoute();
	const run = useRunAttentionTarget({ onNavigate: navigate });
	const fix = gateView(t, time, gate)?.fix;
	if (!fix) return null;
	return (
		<DvButton size="sm" onClick={() => run(fix.action)}>
			{fix.label}
		</DvButton>
	);
}

function LockedModels({ target }: Readonly<{ target: ObserveTarget }>) {
	const { t } = useTranslation("devices");
	const overlay = useOverlay();
	return (
		<StateView
			kind="locked"
			title={t("devices:models.states.lockedTitle", "Locked")}
			text={t(
				"devices:models.states.locked",
				"Models and their usage are read from {{device}} with your keys. Unlock to read them; they are cleared again when you lock.",
				{ device: target.name },
			)}
			actions={
				<DvButton
					size="sm"
					icon={LockOpen}
					onClick={() =>
						overlay.openUnlock(target.deviceId, {
							connectLive: true,
							forModels: true,
						})
					}
				>
					{t("devices:models.states.unlock", "Unlock…")}
				</DvButton>
			}
		/>
	);
}

function NeedsLive({ target }: Readonly<{ target: ObserveTarget }>) {
	const { t } = useTranslation("devices");
	const device = target.name;
	const offline =
		target.presence?.kind === "offline" || target.presence?.kind === "never";
	return (
		<StateView
			kind="notloaded"
			title={t(
				"devices:models.states.needsLiveTitle",
				"Needs a live connection",
			)}
			text={
				offline
					? t(
							"devices:models.states.offline",
							"{{device}} is offline. Its models are read live from the device.",
							{ device },
						)
					: t(
							"devices:models.states.needsLive",
							"Models, downloads and runtimes are read from {{device}} over a live connection.",
							{ device },
						)
			}
			actions={
				offline || livePhase(target) === "unreachable" ? null : (
					<ConnectButton deviceId={target.deviceId} />
				)
			}
		/>
	);
}

/** Locked, no keys, or no live session: the connection's state in the Models tab's words. */
export function ModelsLiveState({
	target,
}: Readonly<{ target: ObserveTarget }>) {
	const phase = livePhase(target);
	if (phase === "locked") return <LockedModels target={target} />;
	if (phase === "idle" || phase === "unreachable")
		return <NeedsLive target={target} />;
	return <LiveDataState target={target} what="metrics" />;
}

/** The agent doesn't host models: an older agent, or one that answered "unsupported". */
export function OlderAgentNotice({
	target,
}: Readonly<{ target: ObserveTarget }>) {
	const { t } = useTranslation("devices");
	const { navigate } = useDevicesRoute();
	const run = useRunAttentionTarget({ onNavigate: navigate });
	const fix: FixAction = { kind: "update_agent", deviceId: target.deviceId };
	return (
		<GateNotice
			kind="unsupported"
			title={t(
				"devices:models.states.olderAgent",
				"The agent on {{device}} can't host models yet.",
				{ device: target.name },
			)}
			text={t(
				"devices:models.states.olderAgentText",
				"Update the device agent to serve models from it. Its services keep running as they are.",
			)}
			actions={
				<DvButton size="sm" onClick={() => run(fix)}>
					{fixLabel(t, fix)}
				</DvButton>
			}
		/>
	);
}

/** Any other gate: no access, a blocked identity, the hub. Its own sentence and fix. */
function GateState({ gate }: Readonly<{ gate: GateFailure }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const copy = gateCopy(t, gate, { at: time.at, locale: time.locale });
	return (
		<GateNotice
			kind={gate.kind}
			title={copy.title}
			text={copy.text}
			have={copy.have}
			actions={<FixButton gate={gate} />}
		/>
	);
}

/** G7 because the keys are simply locked; held elsewhere or unlocking keep their own sentence. */
const plainlyLocked = (gate: GateFailure) =>
	gate.gate === "G7" &&
	(gate.copy.code.startsWith("locked_") ||
		gate.copy.code === "unlock_to_check_permissions");

/** Why the Models tab can't read; `null` when the gate passes. */
export function ModelsGateState({
	target,
	gate,
}: Readonly<{ target: ObserveTarget; gate: GateResult }>) {
	const { t } = useTranslation("devices");
	if (gate.ok) return null;
	if (gate.copy.code === "agent_features_unknown")
		return (
			<StateView
				kind="loading"
				title={t(
					"devices:models.states.checking",
					"Checking what {{device}} supports…",
					{ device: target.name },
				)}
			/>
		);
	if (plainlyLocked(gate)) return <LockedModels target={target} />;
	if (gate.gate === "G8") return <ModelsLiveState target={target} />;
	if (gate.gate === "G9") return <OlderAgentNotice target={target} />;
	return <GateState gate={gate} />;
}

/**
 * What the device last reported about its models while nothing can be read
 * live: the counters of its encrypted metrics snapshot, or what the keys kept
 * when they locked. Nothing without one (no model host, no model permission).
 */
export function ModelsSnapshotBlock({
	target,
}: Readonly<{ target: ObserveTarget }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const snapshot = useModelsSnapshot(target.deviceId);
	if (!snapshot) return null;
	const { summary, freshness } = snapshot;
	const copy = headlineCopy(t, summaryHeadline(summary), {
		device: target.name,
		locale: time.locale,
	});
	const names = [target.name];
	return (
		<Block
			id="models-snapshot"
			icon={Boxes}
			title={t("devices:models.states.snapshotTitle", "Last reported")}
			stamp={<FreshnessStamp {...stampOf(freshness)} />}
		>
			<p
				className={cx(
					"flex flex-col gap-1 text-ui",
					freshness.age === "locked" && LOCKED_DATA_CLASS,
				)}
			>
				<span className="font-medium">{monoNames(copy.lead, names)}</span>
				{summary.models && copy.rest ? (
					<span className="text-muted-foreground">
						{monoNames(copy.rest, names)}
					</span>
				) : null}
			</p>
		</Block>
	);
}

/** The first read failed: the reason in one sentence and a retry. */
export function ModelsReadFailed({
	target,
	failure,
	onRetry,
}: Readonly<{
	target: ObserveTarget;
	failure: DeviceFailure;
	onRetry: () => Promise<void>;
}>) {
	const { t } = useTranslation("devices");
	const [busy, setBusy] = useState(false);
	const reason = failure.rejection?.error;
	return (
		<StateView
			kind="error"
			title={t(
				"devices:models.states.failedTitle",
				"{{device}} didn't return its models",
				{ device: target.name },
			)}
			text={
				<>
					{errorCopy(t, failure.code, { device: target.name })}
					{reason ? ` “${reason}”` : null}
				</>
			}
			actions={
				<DvButton
					size="sm"
					icon={RefreshCw}
					busy={busy}
					onClick={() => {
						setBusy(true);
						void onRetry().finally(() => setBusy(false));
					}}
				>
					{t("devices:models.states.retry", "Try again")}
				</DvButton>
			}
		/>
	);
}
