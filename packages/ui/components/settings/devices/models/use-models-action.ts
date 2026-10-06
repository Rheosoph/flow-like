"use client";

import { useTranslation } from "@flow-like/locales";
import { useMemo } from "react";
import type { z } from "zod";
import { deviceName } from "../../../../lib/device-management/model/device-view";
import { evaluateGate } from "../../../../lib/device-management/model/gates";
import type { GateResult } from "../../../../lib/device-management/model/types";
import {
	type HostedModel,
	type ModelAssetSummary,
	type ModelInstalled,
	type ModelJob,
	type ModelSettings,
	type ModelSpec,
	type ModelsRequest,
	type Residency,
	type RuntimeInfo,
	type RuntimeInstalled,
	hostedModelSchema,
	modelAssetSummarySchema,
	modelInstalledSchema,
	modelJobSchema,
	modelRemovedSchema,
	modelsCommand,
	runtimeInfoSchema,
	runtimeInstalledSchema,
} from "../../../../lib/device-management/models";
import type { DevicesT } from "../primitives/area-context";
import type { ConfirmStrength } from "../primitives/confirm-sheet";
import type { ConsequenceRows } from "../primitives/consequence-preview";
import {
	type DeviceActionContext,
	type DeviceActionOutcome,
	type DeviceActions,
	type GateSources,
	type GateTarget,
	buildGateContext,
	useAttentionState,
	useDeviceAction,
} from "../workspace";
import {
	type ActionCopy,
	type ModelsWriteKind,
	cancelJobCopy,
	configureCopy,
	ensureCopy,
	installCopy,
	installRuntimeCopy,
	loadCopy,
	removeCopy,
	removeRuntimeCopy,
	residencyCopy,
	unloadCopy,
	updateRuntimeCopy,
} from "./action-copy";
import { runtimeName } from "./models-copy";
import { sameResidency, sameSettings } from "./models-view";
import { modelsKeys } from "./use-models";

/*
 * Every write of the Models tab (plan §3.5) through `useDeviceAction`: gate,
 * confirm with consequence rows, one `models` command, a tray item that leads
 * back to the tab, an inline result under the command's `resultKey`, and the
 * tab's reads refreshed.
 */

export type ModelsActionId = "models_manage" | "models_ensure";

export interface ModelCommand<T> {
	action: ModelsActionId;
	/** The request kind the command sends. */
	request: ModelsWriteKind;
	/** R7: the control stays visible; a failing gate disables it with its reason. */
	gate: GateResult;
	label: string;
	/** "Load Qwen3-8B Q4_K_M?" */
	title: string;
	/** "on edge-berlin-01" */
	sub: string;
	rows: ConsequenceRows;
	strength: ConfirmStrength;
	checkLabel?: string;
	tone: "danger" | "default";
	/** `useInlineResults(resultKey)` shows the outcome next to the control. */
	resultKey: string;
	/** From the click until the device answered. */
	pending: boolean;
	/** Sends the command once; `confirmed` skips the sheet when the screen showed the rows itself. */
	run(options?: { confirmed?: boolean }): Promise<DeviceActionOutcome<T>>;
}

export interface InstallInput {
	modelId: string;
	model: ModelSpec;
	settings?: ModelSettings;
	residency?: Residency;
}

export interface EnsureInput {
	projectId: string;
	pins: { bit_id: string; metadata_sha256: string }[];
}

export interface ModelsActions {
	install(input: InstallInput): ModelCommand<ModelInstalled>;
	configure(
		model: HostedModel,
		change: { settings: ModelSettings; residency: Residency },
	): ModelCommand<HostedModel>;
	load(model: HostedModel): ModelCommand<HostedModel>;
	unload(model: HostedModel): ModelCommand<HostedModel>;
	remove(model: HostedModel): ModelCommand<{ model_id: string }>;
	installRuntime(runtime: RuntimeInfo): ModelCommand<RuntimeInstalled>;
	removeRuntime(runtime: RuntimeInfo): ModelCommand<RuntimeInfo>;
	cancelJob(job: ModelJob): ModelCommand<ModelJob>;
	/** The deploy step: gated like the deploy (`deploy` on the project), not by model permissions. */
	ensure(input: EnsureInput): ModelCommand<ModelAssetSummary>;
}

/** Result groups: one per model, runtime, download, the add flow and each app's deploy step. */
export const modelsResultKey = {
	model: (deviceId: string, modelId: string) =>
		`models:${deviceId}:model:${modelId}`,
	install: (deviceId: string) => `models:${deviceId}:install`,
	runtime: (
		deviceId: string,
		runtime: Pick<RuntimeInfo, "runtime" | "backend">,
	) => `models:${deviceId}:runtime:${runtime.runtime}-${runtime.backend}`,
	job: (deviceId: string, jobId: string) => `models:${deviceId}:job:${jobId}`,
	ensure: (deviceId: string, projectId: string) =>
		`models:${deviceId}:ensure:${projectId}`,
};

interface CommandSpec<T> {
	action: ModelsActionId;
	request: ModelsRequest & { kind: ModelsWriteKind };
	copy: ActionCopy;
	resultKey: string;
	target?: GateTarget;
	parse: Parser<T>;
}

/** A schema of an answer; its input is whatever the device sent. */
type Parser<T> = z.ZodType<T, z.ZodTypeDef, unknown>;

interface Bound {
	t: DevicesT;
	actions: DeviceActions;
	sources: GateSources;
	deviceId: string;
	device: string;
	invalidate: readonly (readonly unknown[])[];
}

/** The answer of one write: anything but a completed, well-formed answer is an error. */
async function send<T>(
	context: DeviceActionContext,
	request: ModelsRequest,
	parse: Parser<T>,
): Promise<T> {
	const response = await context.request(modelsCommand(request));
	if (response.state !== "completed")
		throw new Error(
			`The device answered the models ${request.kind} request with state "${response.state}" instead of a result.`,
		);
	const parsed = parse.safeParse(response.result);
	if (!parsed.success)
		throw new Error(
			`The device returned an invalid answer to the models ${request.kind} request (${parsed.error.issues[0]?.message ?? "invalid"}).`,
		);
	return parsed.data;
}

function command<T>(bound: Bound, spec: CommandSpec<T>): ModelCommand<T> {
	const { t, actions, deviceId, device } = bound;
	const { copy, request, resultKey, target } = spec;
	const sub = t("devices:models.actions.sub", "on {{device}}", { device });
	return {
		action: spec.action,
		request: request.kind,
		gate: evaluateGate(
			spec.action,
			buildGateContext(bound.sources, deviceId, target),
		),
		label: copy.label,
		title: t("devices:action.confirm.title", "{{label}}?", {
			label: copy.label,
		}),
		sub,
		rows: copy.rows,
		strength: copy.strength,
		...(copy.checkLabel ? { checkLabel: copy.checkLabel } : {}),
		tone: copy.tone,
		resultKey,
		pending: actions.pending(resultKey),
		run: (options = {}) => {
			const skipSheet = options.confirmed === true && copy.strength === "none";
			return actions.run<T>({
				action: spec.action,
				deviceId,
				...(target ? { target } : {}),
				label: copy.label,
				...(skipSheet ? {} : { consequence: copy.rows }),
				strength: copy.strength,
				confirm: {
					sub,
					tone: copy.tone,
					...(copy.checkLabel ? { checkLabel: copy.checkLabel } : {}),
				},
				resultKey,
				call: (context) => send(context, request, spec.parse),
				activity: {
					kind: "command",
					params: { command: "models", request: request.kind },
					deviceName: device,
					href: { screen: "device", deviceId, tab: "models" },
				},
				invalidate: bound.invalidate,
			});
		},
	};
}

/** The device loads every model on its next request, except one kept off. */
const loadsOnRequest = (model: HostedModel) =>
	model.residency.mode !== "pinned_off";

/** Settings restart a loaded model; a change of residency alone restarts nothing. */
function configureCopyOf(
	t: DevicesT,
	names: { model: string; device: string },
	model: HostedModel,
	change: { settings: ModelSettings; residency: Residency },
): ActionCopy {
	const settingsChanged = !sameSettings(model.settings, change.settings);
	if (!settingsChanged && !sameResidency(model.residency, change.residency))
		return residencyCopy(t, names, change.residency);
	return configureCopy(
		t,
		{ ...names, loaded: model.state === "loaded", settingsChanged },
		change.residency,
	);
}

function modelCommands(bound: Bound) {
	const { t, deviceId, device } = bound;
	const names = (model: HostedModel) => ({
		model: model.display_name,
		device,
	});
	const manage = <T>(
		request: CommandSpec<T>["request"],
		copy: ActionCopy,
		resultKey: string,
		parse: Parser<T>,
	) =>
		command<T>(bound, {
			action: "models_manage",
			request,
			copy,
			resultKey,
			parse,
		});
	return {
		install: (input: InstallInput) =>
			manage(
				{
					kind: "install",
					model_id: input.modelId,
					model: input.model,
					...(input.settings ? { settings: input.settings } : {}),
					...(input.residency ? { residency: input.residency } : {}),
				},
				installCopy(t, {
					model: input.model.display_name,
					device,
					size: input.model.assets.reduce((sum, asset) => sum + asset.size, 0),
					files: input.model.assets.length,
				}),
				modelsResultKey.install(deviceId),
				modelInstalledSchema,
			),
		configure: (
			model: HostedModel,
			change: { settings: ModelSettings; residency: Residency },
		) =>
			manage(
				{
					kind: "configure",
					model_id: model.id,
					expected_revision: model.revision,
					settings: change.settings,
					residency: change.residency,
				},
				configureCopyOf(t, names(model), model, change),
				modelsResultKey.model(deviceId, model.id),
				hostedModelSchema,
			),
		load: (model: HostedModel) =>
			manage(
				{ kind: "load", model_id: model.id },
				loadCopy(t, names(model)),
				modelsResultKey.model(deviceId, model.id),
				hostedModelSchema,
			),
		unload: (model: HostedModel) =>
			manage(
				{ kind: "unload", model_id: model.id },
				unloadCopy(t, {
					...names(model),
					loadsOnRequest: loadsOnRequest(model),
				}),
				modelsResultKey.model(deviceId, model.id),
				hostedModelSchema,
			),
		remove: (model: HostedModel) =>
			manage(
				{
					kind: "remove",
					model_id: model.id,
					expected_revision: model.revision,
				},
				removeCopy(t, names(model)),
				modelsResultKey.model(deviceId, model.id),
				modelRemovedSchema,
			),
	};
}

function hostCommands(bound: Bound) {
	const { t, deviceId, device } = bound;
	const manage = <T>(
		request: CommandSpec<T>["request"],
		copy: ActionCopy,
		resultKey: string,
		parse: Parser<T>,
	) =>
		command<T>(bound, {
			action: "models_manage",
			request,
			copy,
			resultKey,
			parse,
		});
	return {
		installRuntime: (runtime: RuntimeInfo) =>
			manage(
				{
					kind: "install_runtime",
					runtime: runtime.runtime,
					backend: runtime.backend,
				},
				runtime.installed
					? updateRuntimeCopy(t, { runtime: runtimeName(t, runtime), device })
					: installRuntimeCopy(t, {
							runtime: runtimeName(t, runtime),
							device,
							size: runtime.size,
						}),
				modelsResultKey.runtime(deviceId, runtime),
				runtimeInstalledSchema,
			),
		removeRuntime: (runtime: RuntimeInfo) =>
			manage(
				{
					kind: "remove_runtime",
					runtime: runtime.runtime,
					backend: runtime.backend,
				},
				removeRuntimeCopy(t, { runtime: runtimeName(t, runtime), device }),
				modelsResultKey.runtime(deviceId, runtime),
				runtimeInfoSchema,
			),
		cancelJob: (job: ModelJob) =>
			manage(
				{ kind: "cancel_job", job_id: job.job_id },
				cancelJobCopy(t, { file: job.file_name, device }),
				modelsResultKey.job(deviceId, job.job_id),
				modelJobSchema,
			),
		ensure: (input: EnsureInput) =>
			command(bound, {
				action: "models_ensure",
				request: {
					kind: "ensure",
					project_id: input.projectId,
					pins: input.pins,
				},
				copy: ensureCopy(t, { device, pins: input.pins.length }),
				resultKey: modelsResultKey.ensure(deviceId, input.projectId),
				target: { projectId: input.projectId },
				parse: modelAssetSummarySchema,
			}),
	};
}

/** Install, configure, load, unload, remove, runtimes, downloads and the deploy step on one device. */
export function useModelsAction(deviceId: string): ModelsActions {
	const { t } = useTranslation("devices");
	const state = useAttentionState();
	const actions = useDeviceAction();
	const { workspace, input } = state;
	return useMemo(() => {
		const row = input.devices.find((device) => device.device_id === deviceId);
		const bound: Bound = {
			t,
			actions,
			sources: state,
			deviceId,
			device: row ? deviceName(row) : deviceId,
			invalidate: [
				modelsKeys.overview(workspace.scopeKey, deviceId),
				modelsKeys.models(workspace.scopeKey, deviceId),
				modelsKeys.jobs(workspace.scopeKey, deviceId),
				modelsKeys.recommendations(workspace.scopeKey, deviceId),
			],
		};
		return { ...modelCommands(bound), ...hostCommands(bound) };
	}, [t, state, actions, workspace.scopeKey, input.devices, deviceId]);
}
