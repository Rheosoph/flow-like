"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { useQueries, useQueryClient } from "@tanstack/react-query";
import {
	CircleCheck,
	CirclePause,
	Info,
	Lock,
	Play,
	Sparkles,
	Upload,
} from "lucide-react";
import { useMemo, useState, useSyncExternalStore } from "react";
import {
	type ArtifactTransferStatus,
	abortProjectArtifact,
	abortRefusalSettles,
	forgetArtifactTransfer,
} from "../../../../../lib/device-management/artifacts";
import { deviceKeys } from "../../../../../lib/device-management/hub/queries";
import type {
	ApprovalDraft,
	DeployPlan,
	PlanTarget,
} from "../../../../../lib/device-management/model/deploy-plan";
import type { DeployRoute } from "../../../../../lib/device-management/model/types";
import type { DeviceWorkspace } from "../../../../../lib/device-management/workspace/types";
import { humanFileSize } from "../../../../../lib/utils";
import { CloudApprovalFields } from "../../cloud/approval-fields";
import { eligibilityCopy } from "../../copy/eligibility-copy";
import { errorCopy } from "../../copy/error-copy";
import { gateCopy } from "../../copy/gate-copy";
import {
	type AreaTime,
	type DevicesT,
	useAreaTime,
} from "../../primitives/area-context";
import { Block } from "../../primitives/block";
import { DvButton } from "../../primitives/dv-button";
import { CellSub, DvTable, Td, Th, Tr } from "../../primitives/dv-table";
import { SwitchField } from "../../primitives/form-fields";
import { FreshnessStamp } from "../../primitives/freshness-stamp";
import { GateNotice } from "../../primitives/gate-notice";
import { ProgressBar } from "../../primitives/meter";
import { StateView } from "../../primitives/state-view";
import { WizardStepHeader } from "../../primitives/wizard";
import { useDevicesRoute } from "../../routing/use-devices-route";
import { stampOf } from "../../shell/attention-popover";
import {
	type DeviceActionContext,
	type DeviceActionOutcome,
	type DeviceActionRequest,
	type DeviceActions,
	useDeviceAction,
	useDeviceRows,
	useDeviceWorkspace,
	useOverlay,
} from "../../workspace";
import type {
	DeployDeviceCheck,
	DeployPrepared,
	DeployStepProps,
} from "../step-props";
import {
	type CopyEventCheck,
	type CopyRefusedEvent,
	type CopyRow,
	type DeviceCopyFacts,
	copyRowOf,
	manifestFileSizes,
} from "../update-path";
import {
	checkCopyEvents,
	copyUploadsOf,
	deployRunExtras,
	readDeviceCopyFacts,
	sendCopy,
	useLiveTargets,
} from "../use-deploy-run";
import {
	ApprovalsLine,
	KeptBlock,
	SpendingBlock,
	defaultSpending,
	keptServices,
	newServices,
} from "./access-cost-step";

/* Step 6 offline · Copy & upload (APP §3.11): the copy goes to each device; uploads resume and can run before Review. */

type Bundle = DeployPrepared | null;

/** A block head's count when it carries words ("2 devices"); `Block.count` takes a number. */
export const HEAD_CHIP =
	"inline-flex h-5 items-center rounded-full bg-muted px-1.5 font-mono text-xs font-medium whitespace-nowrap tabular-nums text-ink-2";

function factsQuery(
	workspace: DeviceWorkspace,
	target: PlanTarget,
	bundle: Bundle,
) {
	const sha = bundle?.artifact.descriptor.manifest_sha256 ?? "";
	const key = ["deploy-copy", target.deviceId, sha];
	async function queryFn(): Promise<DeviceCopyFacts> {
		if (!bundle) return {};
		return readDeviceCopyFacts(workspace, target.deviceId, bundle);
	}
	return {
		queryKey: [...deviceKeys.root(workspace.scopeKey), ...key],
		queryFn,
		enabled: !target.locked && bundle !== null,
		staleTime: 60_000,
		retry: false,
	};
}

interface DeviceFactsRead {
	facts: Map<string, DeviceCopyFacts>;
	/** Unix seconds of the newest device read; undefined while no device has answered. */
	readAt?: number;
}

function useDeviceFacts(plan: DeployPlan, bundle: Bundle): DeviceFactsRead {
	const workspace = useDeviceWorkspace();
	useLiveTargets(plan.targets);
	const queries = plan.targets.map(function query(target) {
		return factsQuery(workspace, target, bundle);
	});
	const results = useQueries({ queries });
	const facts = new Map<string, DeviceCopyFacts>();
	let newest = 0;
	plan.targets.forEach(function keep(target, index) {
		const result = results[index];
		if (!result?.data) return;
		facts.set(target.deviceId, result.data);
		newest = Math.max(newest, result.dataUpdatedAt);
	});
	return { facts, readAt: newest ? Math.floor(newest / 1000) : undefined };
}

interface UploadRows {
	rows: CopyRow[];
	readAt?: number;
}

function useRows(plan: DeployPlan, bundle: Bundle): UploadRows {
	const workspace = useDeviceWorkspace();
	const store = copyUploadsOf(workspace);
	const entries = useSyncExternalStore(
		store.subscribe,
		store.snapshot,
		store.snapshot,
	);
	const { facts, readAt } = useDeviceFacts(plan, bundle);
	const sizes = useMemo(
		function sizes() {
			return bundle ? manifestFileSizes(bundle) : [];
		},
		[bundle],
	);
	if (!bundle) return { rows: [] };
	const input = { plan, bundle, sizes, facts, entries };
	const rows = plan.targets.map(function row(target) {
		return copyRowOf(input, target);
	});
	return { rows, readAt };
}

/* Sending. */

interface UploadContext {
	workspace: DeviceWorkspace;
	actions: DeviceActions;
	t: DevicesT;
	time: AreaTime;
	bundle: DeployPrepared;
	route: DeployRoute;
	/** Hands a device's event check to the wizard: Where and Settings then show it. */
	report: DeployStepProps["reportDeviceCheck"];
}

/** The device's check as the plan reads it: an event it refuses always carries a sentence. */
function deviceCheckOf(t: DevicesT, check: CopyEventCheck): DeployDeviceCheck {
	const refusals: Record<string, string> = {};
	const fallback = t(
		"devices:deployShip.copy.refusedNoReason",
		"The device can't run this event.",
	);
	// A copy prepared elsewhere can't carry a flow version for an event that follows Latest: the cause is known, whatever the device words.
	const latestCopy = eligibilityCopy(t, {
		code: "latest_flow",
		eventType: "",
		latestFlow: "copy",
	}).long;
	for (const event of check.refused)
		refusals[event.id] =
			event.cause === "latest_copy" ? latestCopy : event.reason || fallback;
	return { refusals, variables: check.variables };
}

function failureText(
	c: UploadContext,
	outcome: DeviceActionOutcome<ArtifactTransferStatus>,
): string | undefined {
	const { t } = c;
	if (outcome.status === "gated")
		return gateCopy(t, outcome.gate, c.time).inline;
	if (outcome.status === "rejected" || outcome.status === "failed")
		return errorCopy(t, outcome.failure.code);
	if (outcome.status === "unknown")
		return t(
			"devices:deployShip.copy.noReply",
			"No reply from the device. The upload can resume.",
		);
	return undefined;
}

type UploadRequest = DeviceActionRequest<ArtifactTransferStatus>;

function uploadRequest(c: UploadContext, row: CopyRow): UploadRequest {
	const { workspace, bundle } = c;
	const { deviceId, name } = row.target;
	const projectId = bundle.artifact.descriptor.project_id;
	const label = c.t(
		"devices:deployShip.copy.action",
		"Upload the copy to {{device}}",
		{ device: name },
	);
	const href: DeployRoute = { ...c.route, step: "copy_upload" };
	const activity: UploadRequest["activity"] = {
		kind: "upload",
		deviceName: name,
		projectId,
		href,
	};
	function call(context: DeviceActionContext) {
		const track = context.track.bind(context);
		return sendCopy({ workspace, bundle, row, call: context.call, track });
	}
	return {
		action: "upload_revision",
		deviceId,
		target: { projectId },
		label,
		lane: "operation",
		resultKey: `copy:${row.key}`,
		call,
		activity,
	};
}

async function uploadOne(c: UploadContext, row: CopyRow) {
	const { workspace, bundle } = c;
	const store = copyUploadsOf(workspace);
	const held = { done: row.have, total: row.total };
	store.set(row.key, { phase: "sending", ...held });
	const outcome = await c.actions.run(uploadRequest(c, row));
	if (outcome.status !== "done") {
		const reason = failureText(c, outcome);
		const failed = reason
			? { phase: "failed" as const, ...held, reason }
			: undefined;
		store.set(row.key, failed);
		return;
	}
	const sent = { workspace, bundle, row };
	const check = await checkCopyEvents(sent, outcome.result);
	if (check && c.report)
		c.report(row.target.deviceId, deviceCheckOf(c.t, check));
	const at = Math.floor(workspace.clock.now() / 1000);
	const total = row.total;
	store.set(row.key, { phase: "done", done: total, total, at, check });
}

/* Cells. */

/** Nothing was sent from here: only a device that lists its versions can say it holds none of this one. */
const unsentText = (t: DevicesT, row: CopyRow): string => {
	if (row.target.locked)
		return t(
			"devices:deployShip.copy.lockedUnknown",
			"Locked: unlock to see what it holds",
		);
	if (!row.usage)
		return t(
			"devices:deployShip.copy.checkedOnUpload",
			"Checked when the upload starts",
		);
	return t("devices:deployShip.copy.nothing", "Nothing of this version");
};

function onDeviceText(t: DevicesT, row: CopyRow): string {
	if (row.skip === "keep")
		return t(
			"devices:deployShip.copy.keeps",
			"Keeps its version: nothing is uploaded",
		);
	if (row.skip === "has_version")
		return t(
			"devices:deployShip.copy.hasVersion",
			"Already has this version: nothing to send",
		);
	if (row.entry?.phase === "done")
		return t("devices:deployShip.copy.hasAll", {
			count: row.total,
			defaultValue_one: "Has the file of this version",
			defaultValue_other: "Has all {{count, number}} files of this version",
		});
	if (row.have > 0)
		return t(
			"devices:deployShip.copy.hasSome",
			"Has {{have, number}} of {{total, number}} files of this version · resumes",
			{ have: row.have, total: row.total },
		);
	if (row.paused)
		return t(
			"devices:deployShip.copy.resumes",
			"An earlier upload of this version resumes",
		);
	return unsentText(t, row);
}

const isOpen = (row: CopyRow) => !row.skip && row.entry?.phase !== "done";

function toSendText(t: DevicesT, row: CopyRow): string {
	if (!isOpen(row)) return "–";
	const size = humanFileSize(row.sendBytes);
	return t("devices:deployShip.copy.toSend", {
		count: row.sendFiles,
		size,
		defaultValue_one: "{{count, number}} file · {{size}}",
		defaultValue_other: "{{count, number}} files · {{size}}",
	});
}

interface RowProps {
	row: CopyRow;
}

function refusedText(t: DevicesT, event: CopyRefusedEvent): string {
	if (!event.reason)
		return t(
			"devices:deployShip.copy.checkRefused",
			"{{event}} can't run here",
			{ event: event.name },
		);
	return t(
		"devices:deployShip.copy.checkRefusedWhy",
		"{{event}} can't run here: {{reason}}",
		{ event: event.name, reason: event.reason },
	);
}

function CheckLine({ row }: Readonly<RowProps>) {
	const { t } = useTranslation("devices");
	const check = row.entry?.check;
	if (!check) return null;
	const events = check.runs.join(", ");
	const runs = t(
		"deployShip.copy.checkRuns",
		"{{device}} checked the events: {{events}} can run",
		{ device: row.target.name, events },
	);
	const refused = check.refused.map(function line(event) {
		return (
			<CellSub key={event.id} className="text-warning">
				{refusedText(t, event)}
			</CellSub>
		);
	});
	return (
		<>
			{check.runs.length ? <CellSub>{runs}</CellSub> : null}
			{refused}
		</>
	);
}

interface RowControls {
	resume(row: CopyRow): void;
	abort(row: CopyRow): void;
	busy: boolean;
}

interface RowControlProps extends RowProps {
	controls: RowControls;
}

function PausedStatus({ row, controls }: Readonly<RowControlProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const off = controls.busy || row.target.locked;
	// Why this window's attempt stopped, when it did; an upload from an earlier session has no reason.
	const stopped = row.entry?.phase === "failed" ? row.entry.reason : undefined;
	const expires = time.at(row.paused?.expiresAt ?? 0);
	const until = t(
		"deployShip.copy.resumableUntil",
		"Resumable until {{time}}",
		{ time: expires },
	);
	function resume() {
		controls.resume(row);
	}
	function abort() {
		controls.abort(row);
	}
	return (
		<>
			<span className="inline-flex items-center gap-1 text-warning">
				<CirclePause aria-hidden className="size-3.5" />
				{t("deployShip.copy.paused", "Paused")}
			</span>
			{stopped ? <CellSub className="text-critical">{stopped}</CellSub> : null}
			<CellSub>{until}</CellSub>
			<span className="mt-1 flex flex-wrap gap-1.5">
				<DvButton size="xs" icon={Play} disabled={off} onClick={resume}>
					{t("deployShip.copy.resume", "Resume")}
				</DvButton>
				<DvButton
					size="xs"
					variant="danger-ghost"
					disabled={off}
					onClick={abort}
				>
					{t("deployShip.copy.abort", "Abort…")}
				</DvButton>
			</span>
		</>
	);
}

function SendingStatus({ row }: Readonly<RowProps>) {
	const { t } = useTranslation("devices");
	const done = row.entry?.done ?? 0;
	const total = row.entry?.total ?? 0;
	const busy = row.entry?.phase === "busy";
	const label = t("deployShip.copy.progress", "Upload to {{device}}", {
		device: row.target.name,
	});
	const sending = t(
		"deployShip.copy.sending",
		"Sending files · {{done, number}} of {{total, number}}",
		{ done, total },
	);
	return (
		<>
			<ProgressBar
				value={total ? Math.round((done / total) * 100) : 0}
				tone={busy ? "warning" : "info"}
				label={label}
			/>
			<CellSub>
				{busy ? t("deployShip.copy.busy", "Device busy, retrying…") : sending}
			</CellSub>
		</>
	);
}

function DoneStatus({ row }: Readonly<RowProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const at = time.clock(row.entry?.at ?? 0);
	const done = t("deployShip.copy.done", "Done · {{time}}", { time: at });
	return (
		<>
			<span className="inline-flex items-center gap-1 text-good">
				<CircleCheck aria-hidden className="size-3.5" />
				{done}
			</span>
			<CheckLine row={row} />
		</>
	);
}

function IdleStatus({ row }: Readonly<RowProps>) {
	const { t } = useTranslation("devices");
	if (row.skip)
		return (
			<span className="inline-flex items-center gap-1 text-good">
				<CircleCheck aria-hidden className="size-3.5" />
				{t("deployShip.copy.nothingToSend", "Nothing to send")}
			</span>
		);
	if (row.entry?.phase !== "failed")
		return (
			<span className="text-muted-foreground">
				{t("deployShip.copy.notStarted", "Not started")}
			</span>
		);
	return (
		<>
			<span className="text-critical">
				{t("deployShip.copy.failed", "Failed")}
			</span>
			<CellSub>{row.entry.reason}</CellSub>
		</>
	);
}

function StatusCell({ row, controls }: Readonly<RowControlProps>) {
	const phase = row.entry?.phase;
	if (phase === "sending" || phase === "busy")
		return <SendingStatus row={row} />;
	if (phase === "done") return <DoneStatus row={row} />;
	if (row.paused) return <PausedStatus row={row} controls={controls} />;
	return <IdleStatus row={row} />;
}

interface TableProps {
	rows: readonly CopyRow[];
	controls: RowControls;
}

const UPLOAD_COLS = ["24%", "30%", "20%", "26%"];

function UploadsTable({ rows, controls }: Readonly<TableProps>) {
	const { t } = useTranslation("devices");
	const device = t("deployShip.copy.colDevice", "Device");
	const on = t("deployShip.copy.colOn", "On the device");
	const send = t("deployShip.copy.colSend", "To send");
	const status = t("deployShip.copy.colStatus", "Status");
	const head = (
		<tr>
			<Th>{device}</Th>
			<Th>{on}</Th>
			<Th numeric>{send}</Th>
			<Th>{status}</Th>
		</tr>
	);
	return (
		<DvTable
			label={t("deployShip.copy.uploads", "Uploads")}
			cols={UPLOAD_COLS}
			stackAt={560}
			head={head}
		>
			{rows.map((row) => (
				<Tr key={row.key}>
					<Td label={device} kind="mono">
						{row.target.name}
						{row.target.locked ? (
							<Lock aria-hidden className="ml-1 inline size-3" />
						) : null}
					</Td>
					<Td label={on}>{onDeviceText(t, row)}</Td>
					<Td label={send} className="text-right tabular-nums wrap-normal">
						{toSendText(t, row)}
					</Td>
					<Td label={status}>
						<StatusCell row={row} controls={controls} />
					</Td>
				</Tr>
			))}
		</DvTable>
	);
}

/* Upload now, resume, abort. */

function deployRouteOf(
	route: ReturnType<typeof useDevicesRoute>["route"],
	plan: DeployPlan,
): DeployRoute {
	if (route.screen === "deploy") return route;
	const deviceIds = plan.targets.map((target) => target.deviceId);
	return { screen: "deploy", deviceIds };
}

async function uploadRows(c: UploadContext, rows: readonly CopyRow[]) {
	for (const row of rows) await uploadOne(c, row);
}

function useUploader(props: Readonly<DeployStepProps>, bundle: Bundle) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const workspace = useDeviceWorkspace();
	const actions = useDeviceAction();
	const queryClient = useQueryClient();
	const { route } = useDevicesRoute();
	const [busy, setBusy] = useState(false);
	const { plan, reportDeviceCheck: report } = props;
	const queryKey = [...deviceKeys.root(workspace.scopeKey), "deploy-copy"];
	async function refresh() {
		await queryClient.invalidateQueries({ queryKey });
	}
	async function settle() {
		setBusy(false);
		await refresh();
	}
	async function upload(rows: readonly CopyRow[]) {
		if (!bundle || busy) return;
		setBusy(true);
		const routeOf = deployRouteOf(route, plan);
		const c = { workspace, actions, t, time, bundle, route: routeOf, report };
		await uploadRows(c, rows).finally(settle);
	}
	return { upload, busy, refresh, actions, workspace };
}

type Uploader = ReturnType<typeof useUploader>;

function abortConsequence(t: DevicesT, device: string) {
	const what = t(
		"devices:deployShip.copy.abortWhat",
		"Discards the files of this upload that {{device}} already holds.",
		{ device },
	);
	const who = t(
		"devices:deployShip.copy.abortWho",
		"Nobody: no service uses them yet.",
	);
	const when = t("devices:deployShip.copy.abortWhen", "Right away.");
	const text = t(
		"devices:deployShip.copy.abortUndo",
		"Upload again; it starts from the beginning.",
	);
	return { what, who, when, undo: { reversible: true, text } };
}

interface AbortInput {
	t: DevicesT;
	workspace: DeviceWorkspace;
	projectId: string;
	row: CopyRow;
	paused: NonNullable<CopyRow["paused"]>;
}

type AbortRequest = DeviceActionRequest<void>;

function abortRequest(input: AbortInput) {
	const { t, row, paused, projectId } = input;
	const { deviceId, name } = row.target;
	const { scope } = input.workspace.deps;
	const label = t(
		"devices:deployShip.copy.abortLabel",
		"Abort the upload to {{device}}",
		{ device: name },
	);
	async function call(context: DeviceActionContext) {
		const id = paused.transferId;
		try {
			await abortProjectArtifact(context.call, projectId, id);
		} catch (error) {
			if (!abortRefusalSettles(error, paused.confirmed)) throw error;
		}
		forgetArtifactTransfer(deviceId, id, scope);
	}
	const consequence = abortConsequence(t, name);
	return {
		action: "upload_revision",
		deviceId,
		target: { projectId },
		label,
		strength: "none",
		consequence,
		resultKey: `copy-abort:${row.key}`,
		call,
	} satisfies AbortRequest;
}

function useAbort(uploader: Uploader, bundle: Bundle) {
	const { t } = useTranslation("devices");
	const { actions, workspace, refresh } = uploader;
	return async function abort(row: CopyRow) {
		const paused = row.paused;
		if (!bundle || !paused) return;
		const projectId = bundle.artifact.descriptor.project_id;
		const request = abortRequest({ t, workspace, projectId, row, paused });
		const outcome = await actions.run(request);
		// Cancelled or refused: the row keeps what it said about the upload.
		if (outcome.status === "done")
			copyUploadsOf(workspace).set(row.key, undefined);
		await refresh();
	};
}

/* Blocks. */

const isLocked = (row: CopyRow) => row.target.locked;

/** What one device says its app versions take; undefined when it doesn't report it. */
function roomOf(t: DevicesT, app: string, row: CopyRow): string | undefined {
	const bytes = row.usage?.project?.bytes;
	if (!bytes) return undefined;
	const used = humanFileSize(bytes.used);
	const device = row.target.name;
	if (bytes.max === null)
		return t(
			"devices:deployShip.copy.roomUsed",
			"{{used}} used by {{app}} on {{device}}.",
			{ used, app, device },
		);
	const max = humanFileSize(bytes.max);
	return t(
		"devices:deployShip.copy.room",
		"{{used}} of {{max}} used by {{app}} on {{device}}.",
		{ used, max, app, device },
	);
}

/** One sentence per device that reports its storage (BG17); none does on older agents. */
function storageText(t: DevicesT, app: string, rows: readonly CopyRow[]) {
	const rooms = rows.flatMap((row) => roomOf(t, app, row) ?? []);
	return rooms.length
		? rooms.join(" ")
		: t(
				"devices:deployShip.copy.roomUnknown",
				"Room for this copy on the device: unknown.",
			);
}

function uploadGate(t: DevicesT, open: readonly CopyRow[], busy: boolean) {
	if (busy)
		return t("devices:deployShip.copy.gateBusy", "An upload is running.");
	if (!open.length)
		return t("devices:deployShip.copy.gateNothing", "Nothing is left to send.");
	if (open.some(isLocked))
		return t(
			"devices:deployShip.copy.gateLocked",
			"Unlock every device to upload now.",
		);
	return undefined;
}

function openBytes(rows: readonly CopyRow[]) {
	let bytes = 0;
	for (const row of rows) {
		if (isOpen(row)) bytes += row.sendBytes;
	}
	return bytes;
}

/** The earliest time a paused upload stops being resumable; undefined when none is paused. */
function firstExpiry(rows: readonly CopyRow[]) {
	let first: number | undefined;
	for (const row of rows) {
		if (!row.paused) continue;
		const at = row.paused.expiresAt;
		first = first === undefined ? at : Math.min(first, at);
	}
	return first;
}

interface RowsProps {
	rows: readonly CopyRow[];
}

const BOLD = { 1: <b className="font-semibold text-foreground" /> };

function TotalLine({ rows }: Readonly<RowsProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const size = humanFileSize(openBytes(rows));
	const expiry = firstExpiry(rows);
	const until = expiry === undefined ? "" : time.at(expiry);
	const values = { size, time: until };
	if (expiry === undefined)
		return (
			<Trans
				t={t}
				i18nKey="deployShip.copy.total"
				defaults="<1>{{size}}</1> to send in all · an upload stays resumable for 24 hours once it starts"
				values={values}
				components={BOLD}
			/>
		);
	return (
		<Trans
			t={t}
			i18nKey="deployShip.copy.totalUntil"
			defaults="<1>{{size}}</1> to send in all · uploads stay resumable until {{time}}"
			values={values}
			components={BOLD}
		/>
	);
}

interface UploadsProps {
	props: DeployStepProps;
	bundle: DeployPrepared;
	rows: readonly CopyRow[];
	/** Unix seconds the devices were last read; undefined while none has answered. */
	readAt?: number;
	uploader: Uploader;
}

function UploadFoot({ rows, uploader }: Readonly<UploadsProps>) {
	const { t } = useTranslation("devices");
	const { openUnlockSeveral } = useOverlay();
	const open = rows.filter(isOpen);
	const gate = uploadGate(t, open, uploader.busy);
	const gateId = gate ? "dp-upload-gate" : undefined;
	const locked = open.some(isLocked);
	function uploadNow() {
		if (!gate) void uploader.upload(open);
	}
	return (
		<div className="flex flex-col gap-2 border-t border-hairline px-4 py-3">
			{open.length ? (
				<p className="flex items-start gap-1.5 text-ui text-ink-2">
					<Upload aria-hidden className="mt-0.5 size-3.5 shrink-0" />
					<span>
						<TotalLine rows={rows} />
					</span>
				</p>
			) : null}
			<div className="flex flex-wrap items-center gap-2">
				<DvButton
					icon={Upload}
					busy={uploader.busy}
					aria-disabled={gate ? true : undefined}
					aria-describedby={gateId}
					onClick={uploadNow}
				>
					{t("deployShip.copy.uploadNow", "Upload now")}
				</DvButton>
				{locked ? (
					<DvButton size="sm" variant="ghost" onClick={openUnlockSeveral}>
						{t("deployShip.copy.unlock", "Unlock devices…")}
					</DvButton>
				) : null}
				{gate ? (
					<span id={gateId} className="text-xs text-ink-2">
						{gate}
					</span>
				) : null}
			</div>
			<p className="text-xs text-muted-foreground">
				{t(
					"deployShip.copy.optional",
					"Optional: sends the copy before Review, so each device can check the events against it. Otherwise the uploads run as the first step of the rollout.",
				)}
			</p>
		</div>
	);
}

interface StampProps {
	readAt?: number;
}

/** What the devices hold is read over the live connection; until one answers the rows come from the hub's list. */
function UploadsStamp({ readAt }: Readonly<StampProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { freshness } = useDeviceRows();
	if (readAt === undefined) return <FreshnessStamp {...stampOf(freshness)} />;
	const ago = time.ago(Math.min(readAt, time.nowS));
	return (
		<FreshnessStamp
			source="live"
			age="current"
			observedAt={readAt}
			text={t("deployShip.copy.readAgo", "read {{ago}}", { ago })}
		/>
	);
}

function UploadsBlock(input: Readonly<UploadsProps>) {
	const { props, bundle, rows, uploader } = input;
	const { t } = useTranslation("devices");
	const abort = useAbort(uploader, bundle);
	const { descriptor } = bundle.artifact;
	const controls: RowControls = {
		busy: uploader.busy,
		resume(row) {
			void uploader.upload([row]);
		},
		abort(row) {
			void abort(row);
		},
	};
	const size = humanFileSize(descriptor.total_bytes);
	const summary = t("deployShip.copy.each", {
		count: descriptor.file_count,
		size,
		defaultValue_one: "{{count, number}} file · {{size}} each",
		defaultValue_other: "{{count, number}} files · {{size}} each",
	});
	return (
		<Block
			id="dp-uploads"
			icon={Upload}
			title={t("deployShip.copy.uploads", "Uploads")}
			summary={<span className={HEAD_CHIP}>{summary}</span>}
			stamp={<UploadsStamp readAt={input.readAt} />}
			flush
			foot={storageText(t, props.plan.app?.name ?? "", rows)}
		>
			<UploadsTable rows={rows} controls={controls} />
			<UploadFoot {...input} />
		</Block>
	);
}

interface ModelAccessProps {
	props: DeployStepProps;
	fresh: ReturnType<typeof newServices>;
}

function ModelAccessBlock({ props, fresh }: Readonly<ModelAccessProps>) {
	const { t } = useTranslation("devices");
	const { draft, update, plan } = props;
	const prepared =
		props.prepared ?? deployRunExtras(draft.deploymentId).prepared;
	const { freshness } = useDeviceRows();
	const [on, setOn] = useState(draft.approval.models.length > 0);
	function toggle(next: boolean) {
		setOn(next);
		if (next) return;
		update({ approval: { ...draft.approval, models: [] }, spending: null });
	}
	function setApproval(approval: ApprovalDraft) {
		const spending = draft.spending ?? defaultSpending(approval);
		update({
			approval: { ...approval, files: "none" },
			spending: approval.models.length ? spending : null,
		});
	}
	return (
		<Block
			id="dp-models"
			icon={Sparkles}
			title={t("deployShip.copy.modelAccess", "Model access")}
			stamp={<FreshnessStamp {...stampOf(freshness)} />}
			bodyClassName="flex flex-col gap-3"
		>
			<SwitchField id="dp-model-on" checked={on} onCheckedChange={toggle}>
				{t("deployShip.copy.modelSwitch", "Let the services use hosted models")}
			</SwitchField>
			{on ? (
				<>
					<CloudApprovalFields
						deviceId={plan.targets[0]?.deviceId ?? ""}
						appId={null}
						modelsAppId={plan.app?.id}
						modelBits={prepared?.artifact.bits}
						serviceMaxInstances={draft.maxInstances}
						value={draft.approval}
						onChange={setApproval}
						modelOnly
					/>
					<ApprovalsLine plan={plan} fresh={fresh} modelOnly />
				</>
			) : null}
			<p className="flex items-start gap-1.5 text-xs text-muted-foreground">
				<Info aria-hidden className="mt-px size-3.5 shrink-0" />
				{t(
					"deployShip.copy.localOnly",
					"Local-only apps can't get access to cloud files; only hosted models need the internet.",
				)}
			</p>
		</Block>
	);
}

/* The step. */

interface BodyProps {
	props: DeployStepProps;
}

function NotPrepared({ props }: Readonly<BodyProps>) {
	const { t } = useTranslation("devices");
	function goHow() {
		props.goTo("how");
	}
	return (
		<StateView
			kind="notloaded"
			title={t("deployShip.copy.notPrepared", "The copy isn't prepared yet")}
			text={t(
				"deployShip.copy.notPreparedText",
				"How it runs prepares the copy on this computer. Each device's upload shows here afterwards.",
			)}
			actions={
				<DvButton size="sm" onClick={goHow}>
					{t("deployShip.copy.goHow", "Go to How it runs")}
				</DvButton>
			}
		/>
	);
}

function Uploads({ props }: Readonly<BodyProps>) {
	const { plan, draft } = props;
	const stored = deployRunExtras(draft.deploymentId).prepared;
	const bundle = props.prepared ?? stored ?? null;
	const { rows, readAt } = useRows(plan, bundle);
	const uploader = useUploader(props, bundle);
	if (!bundle) return <NotPrepared props={props} />;
	return (
		<UploadsBlock
			props={props}
			bundle={bundle}
			rows={rows}
			readAt={readAt}
			uploader={uploader}
		/>
	);
}

function Body({ props }: Readonly<BodyProps>) {
	const { t } = useTranslation("devices");
	const { plan, draft, update } = props;
	const fresh = useMemo(() => newServices(plan), [plan]);
	const kept = useMemo(() => keptServices(plan), [plan]);
	const models = fresh.length > 0 && draft.approval.models.length > 0;
	if (!plan.targets.length)
		return (
			<StateView
				kind="notloaded"
				title={t("deployShip.copy.noDevices", "No devices yet")}
				text={t(
					"deployShip.copy.noDevicesText",
					"Pick devices in Where; each one's upload shows here.",
				)}
			/>
		);
	return (
		<>
			<Uploads props={props} />
			{fresh.length ? <ModelAccessBlock props={props} fresh={fresh} /> : null}
			{models ? (
				<SpendingBlock
					plan={plan}
					draft={draft}
					update={update}
					fresh={fresh}
					blocked={false}
				/>
			) : null}
			<KeptBlock plan={plan} kept={kept} modelOnly />
		</>
	);
}

export function CopyUploadStep(props: Readonly<DeployStepProps>) {
	const { t } = useTranslation("devices");
	const workspace = useDeviceWorkspace();
	const web =
		workspace.deps.platform === "web" && props.plan.mode === "offline";
	return (
		<div className="flex min-w-0 flex-col gap-4">
			<WizardStepHeader
				step={6}
				total={8}
				title={t("deployShip.step.copyUpload", "Copy & upload")}
				lede={t(
					"deployShip.copy.lede",
					"The copy goes to each device over its live connection. Uploads resume if they stop.",
				)}
			/>
			{web ? (
				<GateNotice
					kind="platform"
					title={t(
						"deployShip.copy.webGate",
						"Local-only apps deploy from the desktop app.",
					)}
					text={t(
						"deployShip.copy.webGateText",
						"Open this deploy there to upload the copy.",
					)}
				/>
			) : (
				<Body props={props} />
			)}
		</div>
	);
}
