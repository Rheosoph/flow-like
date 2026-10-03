"use client";

import { useTranslation } from "@flow-like/locales";
import {
	type QueryClient,
	type QueryKey,
	useQueryClient,
} from "@tanstack/react-query";
import { useCallback, useMemo, useRef } from "react";
import {
	HubError,
	toHubError,
} from "../../../../lib/device-management/hub/endpoints";
import { evaluateGate } from "../../../../lib/device-management/model/gates";
import type {
	ActionId,
	GateFailure,
	NavTarget,
} from "../../../../lib/device-management/model/types";
import type { ManagementCall } from "../../../../lib/device-management/telemetry";
import { ManagementUnconfirmedError } from "../../../../lib/device-management/transport";
import {
	type ManagementRejection,
	type ManagementResponse,
	managementRejection,
} from "../../../../lib/device-management/types";
import {
	type DeviceFailure,
	classifyDeviceError,
	rejectionCode,
} from "../../../../lib/device-management/workspace/errors";
import type {
	ActivityDetailCode,
	ActivityItem,
	ActivityKind,
	ActivityTarget,
	CallLane,
	DeviceWorkspace,
	ResumeHandle,
} from "../../../../lib/device-management/workspace/types";
import type { DevicesT } from "../primitives/area-context";
import {
	type ConfirmOptions,
	type ConfirmResult,
	type ConfirmStrength,
	useConfirm,
} from "../primitives/confirm-sheet";
import type { ConsequenceRows } from "../primitives/consequence-preview";
import { hubErrorCopy } from "./area-context";
import {
	useDeviceWorkspace,
	useManagerValue,
} from "./device-workspace-provider";
import { type InlineResultEntry, inlineResultsOf } from "./use-activity";
import {
	type AttentionState,
	type GateTarget,
	buildGateContext,
	useAttentionState,
} from "./use-attention";

/** A device's definitive refusal; `rejection.error` is the device's own sentence. */
export class DeviceRejectedError extends Error {
	constructor(readonly rejection: ManagementRejection) {
		super(rejection.error || "The device rejected the request.");
		this.name = "DeviceRejectedError";
	}
}

function rejectionOf(
	response: ManagementResponse,
): ManagementRejection | undefined {
	if (response.state !== "rejected") return undefined;
	const coded = managementRejection(response);
	if (coded) return coded;
	const text = (response.result as { error?: unknown } | null)?.error;
	return {
		code: "",
		error: typeof text === "string" ? text : "",
		retryable: false,
	};
}

/** Sends one command; a rejection throws `DeviceRejectedError` so the action layer reports the device's reason. */
export async function requestOrReject(
	call: ManagementCall,
	command: Record<string, unknown>,
	operationId?: string,
): Promise<ManagementResponse> {
	const response = await call(command, operationId);
	const rejection = rejectionOf(response);
	if (rejection) throw new DeviceRejectedError(rejection);
	return response;
}

export interface DeviceActionContext {
	workspace: DeviceWorkspace;
	/** Device call on the action's lane; rejects when the action names no device. */
	call: ManagementCall;
	/** `call` that throws `DeviceRejectedError` on a rejection. */
	request(
		command: Record<string, unknown>,
		operationId?: string,
	): Promise<ManagementResponse>;
	/** What the confirm sheet returned (reason, note). */
	confirm: ConfirmResult;
	/** Updates the tray item of this run (progress, deadline). */
	track(patch: Partial<ActivityItem>): void;
}

export interface DeviceActionActivity<T> {
	kind: ActivityKind;
	/** Shown by the tray under the kind's label (`{ command: "stop" }`). */
	params?: Record<string, string | number>;
	deviceName?: string;
	serviceId?: string;
	projectId?: string;
	href?: NavTarget;
	/** Lets the tray re-check the operation after a reload. */
	resume?(result: T): ResumeHandle | undefined;
	/**
	 * Runs after the call returned and the control is free again: resolves once
	 * the device shows the outcome. Until then the tray item waits.
	 */
	settle?(result: T): Promise<"done" | "failed">;
}

export interface DeviceActionRequest<T> {
	action: ActionId;
	deviceId?: string;
	target?: GateTarget;
	/** Verb + object ("Stop support-bot"): confirm button, tray and inline result. */
	label: string;
	/** R8 rows; with them the confirm sheet opens first. Omit when the screen confirmed inline. */
	consequence?: ConsequenceRows;
	strength?: ConfirmStrength;
	/** Further sheet options (title, sub, typed, checkLabel, reasons, tone, …). */
	confirm?: Partial<
		Omit<ConfirmOptions, "rows" | "strength" | "confirmLabel" | "onConfirm">
	>;
	/** The one request this action sends. */
	call(context: DeviceActionContext): Promise<T>;
	lane?: CallLane;
	/** R9: a tray item for anything that outlives the click. */
	activity?: DeviceActionActivity<T>;
	/** Which control group shows the inline result; defaults to action + device + target. */
	resultKey?: string;
	invalidate?: readonly QueryKey[] | ((result: T) => readonly QueryKey[]);
}

export type DeviceActionOutcome<T> =
	| { status: "done"; result: T }
	| { status: "gated"; gate: GateFailure }
	| { status: "cancelled" }
	/** The same control is already running (double click). */
	| { status: "busy" }
	| {
			status: "rejected";
			rejection: ManagementRejection;
			failure: DeviceFailure;
	  }
	| { status: "failed"; failure: DeviceFailure; error: unknown }
	/** Sent, no reply: it may or may not have run. */
	| { status: "unknown"; error: unknown };

export interface DeviceActions {
	run<T>(request: DeviceActionRequest<T>): Promise<DeviceActionOutcome<T>>;
	/** True from the click until the outcome, for the control's busy state. */
	pending(resultKey: string): boolean;
}

export function actionResultKey(
	action: ActionId,
	deviceId?: string,
	target?: GateTarget,
): string {
	return [
		action,
		deviceId ?? "account",
		target?.placementId ?? target?.projectId ?? "",
	].join(":");
}

interface PendingStore {
	keys(): ReadonlySet<string>;
	/** False when the key is already running. */
	begin(key: string): boolean;
	end(key: string): void;
	subscribe(listener: () => void): () => void;
}

const pendingStores = new WeakMap<DeviceWorkspace, PendingStore>();

function pendingOf(workspace: DeviceWorkspace): PendingStore {
	const known = pendingStores.get(workspace);
	if (known) return known;
	let keys: ReadonlySet<string> = new Set();
	const listeners = new Set<() => void>();
	const set = (next: Set<string>) => {
		keys = next;
		for (const listener of [...listeners]) listener();
	};
	const store: PendingStore = {
		keys: () => keys,
		begin(key) {
			if (keys.has(key)) return false;
			set(new Set(keys).add(key));
			return true;
		},
		end(key) {
			if (!keys.has(key)) return;
			const next = new Set(keys);
			next.delete(key);
			set(next);
		},
		subscribe(listener) {
			listeners.add(listener);
			return () => listeners.delete(listener);
		},
	};
	pendingStores.set(workspace, store);
	return store;
}

interface Failure {
	state: "rejected" | "failed" | "unknown";
	failure: DeviceFailure;
	rejection?: ManagementRejection;
	error: NonNullable<InlineResultEntry["error"]>;
}

function rejectionIn(error: unknown): ManagementRejection | undefined {
	const rejection = (error as { rejection?: unknown } | null)?.rejection;
	if (!rejection || typeof rejection !== "object") return undefined;
	const { code, error: text } = rejection as Record<string, unknown>;
	return typeof code === "string" && typeof text === "string"
		? (rejection as ManagementRejection)
		: undefined;
}

const isHubFailure = (error: unknown) =>
	error instanceof HubError ||
	typeof (error as { status?: unknown } | null)?.status === "number";

function rejected(
	failure: DeviceFailure,
	rejection: ManagementRejection,
): Failure {
	const code = rejectionCode(rejection);
	return {
		state: "rejected",
		failure: { ...failure, code, rejection },
		rejection,
		error: { code, ...(rejection.error ? { reason: rejection.error } : {}) },
	};
}

/** Files what went wrong: no reply, a device refusal (with its reason), a hub answer, or a connection problem. */
function failureOf(error: unknown, t: DevicesT): Failure {
	const failure = classifyDeviceError(error);
	if (error instanceof ManagementUnconfirmedError)
		return { state: "unknown", failure, error: { code: "no_reply" } };
	const rejection = rejectionIn(error) ?? failure.rejection;
	if (rejection) return rejected(failure, rejection);
	const detail = isHubFailure(error)
		? { detail: hubErrorCopy(t, toHubError(error).code) }
		: {};
	return {
		state: "failed",
		failure,
		error: { code: failure.code, ...detail },
	};
}

const FAILED_DETAIL: Record<Failure["state"], ActivityDetailCode> = {
	rejected: "rejected",
	failed: "failed",
	unknown: "no_reply",
};

interface Run<T> {
	request: DeviceActionRequest<T>;
	workspace: DeviceWorkspace;
	queryClient: QueryClient;
	t: DevicesT;
	key: string;
	/** The inline result of this run; rewritten as the run moves on. */
	entry: InlineResultEntry;
	itemId?: string;
}

function trayTarget<T>(
	deviceId: string,
	activity: DeviceActionActivity<T> | undefined,
) {
	const target: ActivityTarget = { deviceId };
	if (activity?.deviceName) target.deviceName = activity.deviceName;
	if (activity?.serviceId) target.serviceId = activity.serviceId;
	if (activity?.projectId) target.projectId = activity.projectId;
	return target;
}

function startTrayItem<T>(run: Run<T>) {
	const { activity, deviceId, action } = run.request;
	if (!activity || !deviceId) return undefined;
	return run.workspace.activity.start({
		kind: activity.kind,
		target: trayTarget(deviceId, activity),
		state: "active",
		label: { code: activity.kind, params: { action, ...activity.params } },
		progress: "indeterminate",
		startedBy: "you",
		actions: [],
		...(activity.href ? { href: activity.href } : {}),
	});
}

/** The live manager files its own "no reply" item with the operation handle: that one stays, ours goes. */
function unknownItem<T>(run: Run<T>, error: unknown) {
	const tracker = run.workspace.activity;
	const operationId =
		error instanceof ManagementUnconfirmedError ? error.operationId : undefined;
	const filed = tracker
		.list()
		.find(
			(item) =>
				item.resume?.type === "operation" &&
				item.resume.operationId === operationId,
		);
	const own = run.itemId;
	if (!own) return filed?.id;
	tracker.finish(own, "unknown", { code: "no_reply" });
	if (!filed || filed.id === own) return own;
	tracker.dismiss(own);
	return filed.id;
}

/** Done now, or waiting until the handle or the screen's `settle` says so. */
function settleTrayItem<T>(run: Run<T>, result: T) {
	const { itemId, workspace } = run;
	const spec = run.request.activity;
	if (!itemId || !spec) return;
	const tracker = workspace.activity;
	const handle = spec.resume?.(result);
	if (!spec.settle && !handle) {
		tracker.finish(itemId, "done", { code: "done" });
		return;
	}
	tracker.update(itemId, {
		state: "waiting",
		detail: { code: "waiting_for_apply" },
		actions: handle ? ["check_again"] : [],
		...(handle ? { resume: handle } : {}),
	});
	void spec.settle?.(result).then(
		(outcome) =>
			tracker.finish(itemId, outcome, {
				code: outcome === "done" ? "done" : "failed",
			}),
		() => undefined,
	);
}

const NO_DEVICE: ManagementCall = () =>
	Promise.reject(new Error("This action is not bound to a device."));

/** Calls on the action's lane; a lost reply is filed under the action's kind and target. */
function deviceCallOf<T>(run: Run<T>): ManagementCall {
	const { workspace, request } = run;
	const { deviceId, activity } = request;
	if (!deviceId) return NO_DEVICE;
	const send = workspace.live.call(deviceId, {
		lane: request.lane ?? "user",
		trackUnconfirmed: {
			kind: activity?.kind ?? "command",
			target: trayTarget(deviceId, activity),
		},
	});
	return (command, operationId) => {
		workspace.touch(deviceId);
		return send(command, operationId);
	};
}

function contextOf<T>(
	run: Run<T>,
	confirm: ConfirmResult,
): DeviceActionContext {
	const call = deviceCallOf(run);
	return {
		workspace: run.workspace,
		call,
		request: (command, operationId) =>
			requestOrReject(call, command, operationId),
		confirm,
		track: (patch) => {
			if (run.itemId) run.workspace.activity.update(run.itemId, patch);
		},
	};
}

function putResult<T>(
	run: Run<T>,
	patch: Pick<InlineResultEntry, "state"> & Partial<InlineResultEntry>,
	itemId = run.itemId,
) {
	inlineResultsOf(run.workspace).put({
		...run.entry,
		...patch,
		...(itemId ? { activityId: itemId } : {}),
	});
}

function succeeded<T>(run: Run<T>, result: T): DeviceActionOutcome<T> {
	const { invalidate } = run.request;
	settleTrayItem(run, result);
	putResult(run, { state: "done" });
	const keys =
		typeof invalidate === "function" ? invalidate(result) : (invalidate ?? []);
	for (const queryKey of keys)
		void run.queryClient.invalidateQueries({ queryKey });
	return { status: "done", result };
}

function failed<T>(run: Run<T>, error: unknown): DeviceActionOutcome<T> {
	const { state, failure, rejection, error: shown } = failureOf(error, run.t);
	if (state === "unknown") {
		putResult(run, { state, error: shown }, unknownItem(run, error));
		return { status: "unknown", error };
	}
	if (run.itemId)
		run.workspace.activity.finish(run.itemId, "failed", {
			code: FAILED_DETAIL[state],
		});
	putResult(run, { state, error: shown });
	return rejection
		? { status: "rejected", rejection, failure }
		: { status: "failed", failure, error };
}

/** Tray item, inline result, the one call, then the outcome. */
async function execute<T>(
	run: Run<T>,
	confirmed: ConfirmResult,
): Promise<DeviceActionOutcome<T>> {
	const { request, workspace } = run;
	inlineResultsOf(workspace).clear(run.key);
	run.itemId = startTrayItem(run);
	putResult(run, { state: "running" });
	const release =
		request.deviceId && request.activity
			? workspace.live.acquire(request.deviceId, "operation")
			: undefined;
	try {
		return succeeded(run, await request.call(contextOf(run, confirmed)));
	} catch (error) {
		return failed(run, error);
	} finally {
		release?.();
	}
}

/**
 * The one path for every mutating control (R8 + R9): gate → confirm (rows and
 * strength) → exactly one call → tray item → inline result → invalidation.
 * Cancel and a failing gate send nothing; a device refusal reports the
 * device's reason and frees the control.
 */
export function useDeviceAction(): DeviceActions {
	const { t } = useTranslation("devices");
	const workspace = useDeviceWorkspace();
	const attention = useAttentionState();
	const latest = useRef<AttentionState>(attention);
	latest.current = attention;
	const confirm = useConfirm();
	const queryClient = useQueryClient();
	const store = pendingOf(workspace);
	const readPending = useCallback(() => store.keys(), [store]);
	const pendingKeys = useManagerValue(store.subscribe, readPending);

	const confirmed = useCallback(
		<T>(request: DeviceActionRequest<T>): Promise<ConfirmResult> =>
			request.consequence
				? confirm({
						title: t("devices:action.confirm.title", "{{label}}?", {
							label: request.label,
						}),
						...request.confirm,
						rows: request.consequence,
						strength: request.strength ?? "none",
						confirmLabel: request.label,
					})
				: Promise.resolve({ ok: true }),
		[confirm, t],
	);

	const run = useCallback(
		async <T>(
			request: DeviceActionRequest<T>,
		): Promise<DeviceActionOutcome<T>> => {
			const { action, deviceId, target, label } = request;
			const key =
				request.resultKey ?? actionResultKey(action, deviceId, target);
			if (!store.begin(key)) return { status: "busy" };
			try {
				const gate = evaluateGate(
					action,
					buildGateContext(latest.current, deviceId, target),
				);
				if (!gate.ok) return { status: "gated", gate };
				const answer = await confirmed(request);
				if (!answer.ok) return { status: "cancelled" };
				if (deviceId) workspace.touch(deviceId);
				const entry: InlineResultEntry = {
					id: crypto.randomUUID(),
					scopeKey: key,
					label,
					state: "running",
					...(deviceId ? { deviceId } : {}),
					at: Date.now(),
				};
				return await execute(
					{ request, workspace, queryClient, t, key, entry },
					answer,
				);
			} finally {
				store.end(key);
			}
		},
		[store, confirmed, workspace, queryClient, t],
	);

	return useMemo(
		() => ({ run, pending: (key: string) => pendingKeys.has(key) }),
		[run, pendingKeys],
	);
}
