"use client";

import { useTranslation } from "@flow-like/locales";
import { useQueryClient } from "@tanstack/react-query";
import { Laptop, RotateCw, Square } from "lucide-react";
import { useCallback, useMemo, useSyncExternalStore } from "react";
import {
	type ModelPush,
	type PushEnvironment,
	type PushableAsset,
	desktopPushEnvironment,
	modelPushOf,
	pushFile,
	pushModelAsset,
	pushableAsset,
} from "../../../../lib/device-management/model-push";
import { deviceLabel } from "../../../../lib/device-management/model/device-view";
import type { ModelJob } from "../../../../lib/device-management/models";
import type { DeviceWorkspace } from "../../../../lib/device-management/workspace/types";
import { useBackend } from "../../../../state/backend-state";
import { gateView } from "../device/use-device-page";
import { bytesText } from "../observe/observe-data";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { DvButton } from "../primitives/dv-button";
import { GatedAction } from "../primitives/gate-notice";
import { InlineResult } from "../primitives/inline-result";
import { ProgressBar } from "../primitives/meter";
import { useAttentionState, useDeviceWorkspace, useGate } from "../workspace";
import { etaText, rateText } from "./models-copy";
import { modelsKeys } from "./use-models";

/*
 * A push outlives the row that started it: after the last byte the device
 * verifies the file (its row then reads `verifying`), and a transfer of
 * hours goes on while the tab or the device page is closed. Pushes live per
 * workspace, keyed by device and job, with a tray item each (R9); only
 * "Stop sending" ends one early.
 */

const SHOW_EVERY_MS = 250;
const TRAY_EVERY_MS = 1_000;

interface Sending {
	kind: "sending";
	/** `local`: downloading into this computer's Bit store first; `device`: what the device holds. */
	phase: "local" | "device";
	bytes: number;
	/** Where the rate is measured from: the phase's first report. */
	from: { bytes: number; at: number };
	at: number;
}

type SendState =
	| { kind: "idle" }
	| Sending
	| { kind: "sent" }
	| { kind: "stopped" }
	| { kind: "failed"; error: string };

const IDLE: SendState = { kind: "idle" };

/** The pushes of one workspace. */
class ModelSends {
	private readonly states = new Map<string, SendState>();
	private readonly running = new Map<
		string,
		{ abort: AbortController; trayId: string }
	>();
	private readonly listeners = new Set<() => void>();
	constructor(private readonly workspace: DeviceWorkspace) {}
	subscribe = (listener: () => void) => {
		this.listeners.add(listener);
		return () => {
			this.listeners.delete(listener);
		};
	};
	state = (key: string): SendState => this.states.get(key) ?? IDLE;
	busy(key: string): boolean {
		return this.running.has(key);
	}
	set(key: string, state: SendState): void {
		this.states.set(key, state);
		for (const listener of this.listeners) listener();
	}
	/** Claims `key` for one push, followed by tray item `trayId`. */
	begin(key: string, trayId: string): AbortController {
		if (!this.running.size && typeof window !== "undefined")
			window.addEventListener("pagehide", this.orphan);
		const abort = new AbortController();
		this.running.set(key, { abort, trayId });
		return abort;
	}
	end(key: string): void {
		this.running.delete(key);
		if (!this.running.size && typeof window !== "undefined")
			window.removeEventListener("pagehide", this.orphan);
	}
	stop(key: string): void {
		this.running.get(key)?.abort.abort(new Error("Sending was stopped."));
	}
	/** A page that goes away leaves no push it can follow: the tray says it may have finished. */
	private readonly orphan = () => {
		for (const { trayId } of this.running.values())
			this.workspace.activity.finish(trayId, "unknown", { code: "no_reply" });
	};
}

const stores = new WeakMap<DeviceWorkspace, ModelSends>();

function sendsOf(workspace: DeviceWorkspace): ModelSends {
	const known = stores.get(workspace);
	if (known) return known;
	const sends = new ModelSends(workspace);
	stores.set(workspace, sends);
	return sends;
}

const sendKey = (deviceId: string, jobId: string) => `${deviceId}/${jobId}`;

interface SendInput {
	workspace: DeviceWorkspace;
	deviceId: string;
	deviceName: string;
	job: ModelJob;
	asset: PushableAsset;
	push: ModelPush;
	environment: PushEnvironment;
	/** Reads the device's downloads again once the push ended. */
	settled(): void;
}

/** The push's tray item: it leads back to the Models tab and counts what the device holds. */
function trayOf(input: SendInput) {
	const { activity } = input.workspace;
	const total = input.job.size;
	const progress = (done: number) => ({
		done,
		total,
		unit: "bytes" as const,
	});
	const id = activity.start({
		kind: "command",
		target: { deviceId: input.deviceId, deviceName: input.deviceName },
		state: "active",
		label: {
			code: "command",
			params: { command: "models", request: "push" },
		},
		detail: { code: "bytes_progress" },
		progress: progress(0),
		startedBy: "you",
		actions: [],
		href: { screen: "device", deviceId: input.deviceId, tab: "models" },
	});
	let shownAt = 0;
	return {
		id,
		progress(bytes: number) {
			const now = Date.now();
			if (bytes < total && now - shownAt < TRAY_EVERY_MS) return;
			shownAt = now;
			activity.update(id, { progress: progress(bytes) });
		},
		finish(outcome: "done" | "failed" | "cancelled") {
			if (outcome === "done") activity.finish(id, "done", { code: "done" });
			else activity.finish(id, "failed", { code: outcome });
		},
	};
}

/** Reports progress to the rows, at most every `SHOW_EVERY_MS` within a phase. */
function reporter(sends: ModelSends, key: string, total: number) {
	return (phase: Sending["phase"], bytes: number) => {
		const previous = sends.state(key);
		const now = Date.now();
		const measured =
			previous.kind === "sending" &&
			previous.phase === phase &&
			previous.from.bytes >= 0 &&
			bytes >= previous.bytes;
		if (measured && bytes < total && now - previous.at < SHOW_EVERY_MS) return;
		sends.set(key, {
			kind: "sending",
			phase,
			bytes,
			from: measured ? previous.from : { bytes, at: now },
			at: now,
		});
	};
}

/** Sends a job's file from this computer; the row only shows it. */
async function sendFile(input: SendInput): Promise<void> {
	const { job } = input;
	const sends = sendsOf(input.workspace);
	const key = sendKey(input.deviceId, job.job_id);
	if (sends.busy(key)) return;
	const tray = trayOf(input);
	const abort = sends.begin(key, tray.id);
	const shown = reporter(sends, key, job.size);
	const report = (phase: Sending["phase"], bytes: number) => {
		shown(phase, bytes);
		if (phase === "device") tray.progress(bytes);
	};
	const now = Date.now();
	sends.set(key, {
		kind: "sending",
		phase: "device",
		bytes: 0,
		from: { bytes: -1, at: now },
		at: now,
	});
	try {
		const file = await pushFile(input.asset, {
			...input.environment,
			signal: abort.signal,
			onLocalDownload: (bytes) => report("local", bytes),
		});
		try {
			await pushModelAsset({
				jobId: job.job_id,
				file,
				push: input.push,
				signal: abort.signal,
				onProgress: ({ bytes }) => report("device", bytes),
			});
		} finally {
			file.close();
		}
		sends.set(key, { kind: "sent" });
		tray.finish("done");
	} catch (error) {
		const stopped = abort.signal.aborted;
		sends.set(
			key,
			stopped
				? { kind: "stopped" }
				: {
						kind: "failed",
						error: error instanceof Error ? error.message : String(error),
					},
		);
		tray.finish(stopped ? "cancelled" : "failed");
	} finally {
		sends.end(key);
		input.settled();
	}
}

/** Why this computer can't send the file, or nothing when it can. */
function unavailable(
	t: DevicesT,
	canPush: boolean,
	asset: PushableAsset,
	desktop: boolean,
): string | undefined {
	if (!canPush)
		return t(
			"devices:models.send.unsupported",
			"This version of the app can't send model files to devices yet.",
		);
	if (asset.descriptor.sources?.length || (desktop && asset.bitHash))
		return undefined;
	return desktop
		? t(
				"devices:models.send.unknownDesktop",
				"The device lists no download address for this file, and this computer has no copy of it.",
			)
		: t(
				"devices:models.send.unknownWeb",
				"The device lists no download address for this file. The desktop app can send it from its own copy of the model.",
			);
}

/** "1.2 GiB of 4.9 GiB · 40 MiB/s · about 2 min left". */
function progressLine(t: DevicesT, state: Sending, size: number): string {
	const parts = [
		t("devices:models.send.bytes", "{{done}} of {{total}}", {
			done: bytesText(state.bytes),
			total: bytesText(size),
		}),
	];
	const seconds = (state.at - state.from.at) / 1000;
	const rate =
		seconds > 0 && state.from.bytes >= 0
			? (state.bytes - state.from.bytes) / seconds
			: 0;
	if (rate > 0 && state.bytes < size) {
		parts.push(rateText(t, rate));
		parts.push(
			t("devices:models.send.left", "{{eta}} left", {
				eta: etaText(t, (size - state.bytes) / rate),
			}),
		);
	}
	return parts.join(" · ");
}

/** "Send from this computer" for one device's download (plan §3.1 push fallback). */
function useSend(deviceId: string, job: ModelJob, asset: PushableAsset) {
	const workspace = useDeviceWorkspace();
	const { input } = useAttentionState();
	const backend = useBackend();
	const queryClient = useQueryClient();
	const sends = sendsOf(workspace);
	const key = sendKey(deviceId, job.job_id);
	const read = useCallback(() => sends.state(key), [sends, key]);
	const state = useSyncExternalStore(sends.subscribe, read, read);
	const push = modelPushOf(workspace.live, deviceId);
	const desktop = workspace.deps.platform === "desktop";
	const deviceName = deviceLabel(input, deviceId);
	const start = useCallback(() => {
		if (!push) return;
		void sendFile({
			workspace,
			deviceId,
			deviceName,
			job,
			asset,
			push,
			environment: desktop ? desktopPushEnvironment(backend.bitState) : {},
			settled: () =>
				void queryClient.invalidateQueries({
					queryKey: modelsKeys.jobs(workspace.scopeKey, deviceId),
				}),
		});
	}, [
		push,
		desktop,
		backend,
		asset,
		job,
		queryClient,
		workspace,
		deviceId,
		deviceName,
	]);
	const stop = useCallback(() => sends.stop(key), [sends, key]);
	return { state, start, stop, canPush: push !== undefined, desktop };
}

function SendProgress({
	job,
	state,
	onStop,
}: Readonly<{ job: ModelJob; state: Sending; onStop: () => void }>) {
	const { t } = useTranslation("devices");
	const local = state.phase === "local";
	const sent = !local && state.bytes >= job.size;
	return (
		<div className="flex flex-col gap-1.5" data-send-model={job.job_id}>
			<span className="flex flex-wrap items-center gap-2">
				<span className="text-xs font-medium">
					{local
						? t(
								"devices:models.send.downloadingHere",
								"Downloading to this computer first",
							)
						: t("devices:models.send.sending", "Sending from this computer")}
				</span>
				{sent ? null : (
					<DvButton size="xs" variant="ghost" icon={Square} onClick={onStop}>
						{t("devices:models.send.stop", "Stop sending")}
					</DvButton>
				)}
			</span>
			<ProgressBar
				value={(state.bytes / job.size) * 100}
				tone={local ? "info" : "warning"}
				label={t("devices:models.send.progress", "Sending {{file}}", {
					file: job.file_name,
				})}
				className="max-w-105"
			/>
			<span className="text-xs tabular-nums text-muted-foreground">
				{progressLine(t, state, job.size)}
			</span>
			{sent ? (
				<InlineResult tone="info">
					{t(
						"devices:models.send.verifying",
						"All of it is sent. The device is checking the file.",
					)}
				</InlineResult>
			) : null}
		</div>
	);
}

/**
 * "Send from this computer" under a download the device can't finish: from
 * this computer's own Bit store on desktop, else as a download streamed
 * through to the device. The device checks the file's fingerprint as always.
 * It stays while this computer sends, also once the device verifies.
 */
export function SendFromThisComputer({
	deviceId,
	job,
}: Readonly<{ deviceId: string; job: ModelJob }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const gate = gateView(t, time, useGate("models_manage", deviceId));
	const asset = useMemo(() => pushableAsset(job), [job]);
	const { state, start, stop, canPush, desktop } = useSend(
		deviceId,
		job,
		asset,
	);
	if (state.kind === "sending")
		return <SendProgress job={job} state={state} onStop={stop} />;
	if (job.state !== "failed" && job.state !== "awaiting_push") return null;
	const blocked = unavailable(t, canPush, asset, desktop);
	if (blocked)
		return <span className="text-xs text-muted-foreground">{blocked}</span>;
	return (
		<div
			className="flex flex-col items-start gap-1.5"
			data-send-model={job.job_id}
		>
			<GatedAction gate={gate?.gate}>
				<DvButton
					size="xs"
					icon={state.kind === "failed" ? RotateCw : Laptop}
					onClick={start}
				>
					{state.kind === "failed"
						? t("devices:models.send.retry", "Send again")
						: t(
								"devices:models.send.button",
								"Send from this computer ({{size}})",
								{
									size: bytesText(job.size),
								},
							)}
				</DvButton>
			</GatedAction>
			{state.kind === "sent" ? (
				<InlineResult tone="good">
					{t(
						"devices:models.send.sent",
						"Sent. The device checks the file before it uses it.",
					)}
				</InlineResult>
			) : null}
			{state.kind === "stopped" ? (
				<InlineResult tone="info">
					{t(
						"devices:models.send.stopped",
						"Stopped. The device keeps what it received; sending again continues from there.",
					)}
				</InlineResult>
			) : null}
			{state.kind === "failed" ? (
				<InlineResult tone="critical">
					{t("devices:models.send.failed", "Sending failed: {{error}}", {
						error: state.error,
					})}
				</InlineResult>
			) : null}
		</div>
	);
}

/** The Models tab's `sendFromThisComputer` slot for one device: under every download, empty unless it needs one. */
export function sendFromThisComputer(deviceId: string) {
	return (job: ModelJob) => (
		<SendFromThisComputer deviceId={deviceId} job={job} />
	);
}
