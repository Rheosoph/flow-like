"use client";

import { useTranslation } from "@flow-like/locales";
import { Boxes, Plus, Settings2, Trash2 } from "lucide-react";
import { useId, useMemo } from "react";
import {
	type HostedModel,
	MODEL_DEFAULT_IDLE_UNLOAD_SECONDS,
	type ModelsOverview,
	type Residency,
} from "../../../../lib/device-management/models";
import { gateView } from "../device/use-device-page";
import { bytesText } from "../observe/observe-data";
import type { ObserveTarget } from "../observe/use-observe-target";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { CellSub, DvTable, Td, Th, Tr } from "../primitives/dv-table";
import {
	FreshnessStamp,
	type FreshnessStampProps,
} from "../primitives/freshness-stamp";
import { type Gate, GateInline, GatedAction } from "../primitives/gate-notice";
import { InlineResult } from "../primitives/inline-result";
import { StateView } from "../primitives/state-view";
import { StatusChip } from "../primitives/status-chip";
import {
	useGate,
	useInlineResults,
	useOverlay,
	usePersonNames,
} from "../workspace";
import {
	STATE_TONE,
	backendLabel,
	compactNumber,
	engineLabel,
	failureLabel,
	kindLabel,
	residencyLabel,
	stateLabel,
} from "./models-copy";
import {
	type Consumer,
	type ModelUsage,
	backendOf,
	consumersOf,
	memoryOf,
	slotsOf,
	usageOf,
} from "./models-view";
import { UseModelButton } from "./use";
import { useModelsDayUsage } from "./use-models";
import {
	type ModelCommand,
	type ModelsActions,
	modelsResultKey,
	useModelsAction,
} from "./use-models-action";

const COLS = [
	"17%",
	"8%",
	"10%",
	"11%",
	"11%",
	"6%",
	"6%",
	"7%",
	"11%",
	"13%",
] as const;
const SHOWN_CONSUMERS = 3;
const LOADABLE = new Set<HostedModel["state"]>(["stopped", "failed"]);
const UNLOADABLE = new Set<HostedModel["state"]>(["loaded", "loading"]);

/** "Add model…": opens the add-model overlay; gated like every model change. */
export function AddModelButton({
	deviceId,
	size = "sm",
}: Readonly<{ deviceId: string; size?: "sm" | "xs" }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const overlay = useOverlay();
	const view = gateView(t, time, useGate("models_manage", deviceId));
	return (
		<GatedAction gate={view?.gate}>
			<DvButton
				size={size}
				icon={Plus}
				onClick={() => overlay.openModelAdd(deviceId)}
			>
				{t("devices:models.actions.add", "Add model…")}
			</DvButton>
		</GatedAction>
	);
}

interface Labels {
	model: string;
	kind: string;
	engine: string;
	state: string;
	memory: string;
	slots: string;
	speed: string;
	requests: string;
	usedBy: string;
	actions: string;
}

function columnLabels(t: DevicesT): Labels {
	return {
		model: t("devices:models.table.model", "Model"),
		kind: t("devices:models.table.kind.label", "Kind"),
		engine: t("devices:models.table.engine.label", "Engine"),
		state: t("devices:models.table.state.label", "State"),
		memory: t("devices:models.table.memory", "Memory"),
		slots: t("devices:models.table.slots", "Slots busy"),
		speed: t("devices:models.table.speed", "Tok/s"),
		requests: t("devices:models.table.requests", "Requests 24 h"),
		usedBy: t("devices:models.table.usedBy.label", "Used by"),
		actions: t("devices:models.table.actions", "Actions"),
	};
}

function consumerLabel(
	t: DevicesT,
	consumer: Consumer,
	name: (userId: string) => string | undefined,
): string {
	if (consumer.kind === "you")
		return t("devices:models.table.usedBy.you", "You");
	if (consumer.kind === "owner")
		return t("devices:models.table.usedBy.owner", "The owner");
	if (consumer.kind === "service") return consumer.serviceId;
	const known = consumer.userId ? name(consumer.userId) : undefined;
	return known ?? t("devices:models.table.usedBy.person", "One person");
}

interface UsedByProps {
	consumers: readonly Consumer[];
	name(userId: string): string | undefined;
}

function UsedBy({ consumers, name }: Readonly<UsedByProps>) {
	const { t } = useTranslation("devices");
	if (!consumers.length) return null;
	const shown = consumers.slice(0, SHOWN_CONSUMERS);
	const more = consumers.length - shown.length;
	return (
		<span className="flex flex-wrap gap-x-1.5 gap-y-0.5">
			{shown.map((consumer) => (
				<span
					key={consumer.key}
					className={consumer.kind === "service" ? "font-mono" : undefined}
				>
					{consumerLabel(t, consumer, name)}
				</span>
			))}
			{more > 0 ? (
				<span className="text-muted-foreground">
					{t("devices:models.table.usedBy.more", {
						count: more,
						defaultValue_one: "+{{count, number}} more",
						defaultValue_other: "+{{count, number}} more",
					})}
				</span>
			) : null}
		</span>
	);
}

function MemoryCell({ model }: Readonly<{ model: HostedModel }>) {
	const { t } = useTranslation("devices");
	const memory = memoryOf(model);
	if (!memory) return null;
	const ram = t("devices:models.table.memoryRam", "{{size}} RAM", {
		size: bytesText(memory.ram),
	});
	if (!memory.vram) return <>{ram}</>;
	return (
		<>
			{t("devices:models.table.memoryGpu", "{{size}} GPU", {
				size: bytesText(memory.vram),
			})}
			<CellSub>{ram}</CellSub>
		</>
	);
}

const gateOf = (
	t: DevicesT,
	time: ReturnType<typeof useAreaTime>,
	command: ModelCommand<unknown> | null,
): Gate | null =>
	command ? (gateView(t, time, command.gate)?.gate ?? null) : null;

const ON_DEMAND: Residency = {
	mode: "on_demand",
	idle_unload_after_seconds: MODEL_DEFAULT_IDLE_UNLOAD_SECONDS,
};

type Toggle = {
	kind: "load" | "unload" | "on_demand";
	command: ModelCommand<HostedModel>;
};

/**
 * Load a stopped model, unload a loaded one. The device reloads a model kept
 * loaded within seconds, so that one offers "Load on demand" instead. It
 * refuses to load a model kept off: that one changes its residency in
 * Settings… first.
 */
function toggleOf(actions: ModelsActions, model: HostedModel): Toggle | null {
	if (LOADABLE.has(model.state))
		return model.residency.mode === "pinned_off"
			? null
			: { kind: "load", command: actions.load(model) };
	if (!UNLOADABLE.has(model.state)) return null;
	if (model.residency.mode !== "always_on")
		return { kind: "unload", command: actions.unload(model) };
	const change = { settings: model.settings, residency: ON_DEMAND };
	return { kind: "on_demand", command: actions.configure(model, change) };
}

function toggleLabel(t: DevicesT, kind: Toggle["kind"]) {
	const labels = {
		load: () => t("devices:models.actions.loadButton", "Load…"),
		unload: () => t("devices:models.actions.unloadButton", "Unload…"),
		on_demand: () =>
			t("devices:models.actions.onDemandButton", "Load on demand…"),
	} satisfies Record<Toggle["kind"], () => string>;
	return labels[kind]();
}

function ModelRowActions({
	deviceId,
	model,
}: Readonly<{ deviceId: string; model: HostedModel }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const overlay = useOverlay();
	const actions = useModelsAction(deviceId);
	const results = useInlineResults(modelsResultKey.model(deviceId, model.id));
	const reasonId = useId();
	const toggle = toggleOf(actions, model);
	const remove = actions.remove(model);
	const gate =
		gateOf(t, time, toggle?.command ?? null) ?? gateOf(t, time, remove);
	const gated = gate ? { "aria-describedby": reasonId } : {};
	return (
		<span className="flex min-w-0 flex-col items-start gap-1">
			<span className="flex flex-wrap items-center gap-1.5">
				{toggle ? (
					<DvButton
						size="xs"
						busy={toggle.command.pending}
						aria-disabled={gateOf(t, time, toggle.command) ? true : undefined}
						{...gated}
						onClick={() => void toggle.command.run()}
					>
						{toggleLabel(t, toggle.kind)}
					</DvButton>
				) : null}
				<DvButton
					size="xs"
					variant="ghost"
					icon={Settings2}
					onClick={() => overlay.openModelSettings(deviceId, model.id)}
				>
					{t("devices:models.actions.settingsButton", "Settings…")}
				</DvButton>
				<UseModelButton deviceId={deviceId} model={model} />
				<DvButton
					size="xs"
					variant="danger-ghost"
					icon={Trash2}
					busy={remove.pending}
					aria-disabled={gateOf(t, time, remove) ? true : undefined}
					{...gated}
					onClick={() => void remove.run()}
				>
					{t("devices:models.actions.removeButton", "Remove…")}
				</DvButton>
			</span>
			{gate ? (
				<GateInline kind={gate.kind} id={reasonId}>
					{gate.reason}
				</GateInline>
			) : null}
			{results.map((result) => (
				<InlineResult
					key={result.id}
					tone={result.tone}
					onDismiss={result.dismiss}
				>
					{result.text}
				</InlineResult>
			))}
		</span>
	);
}

interface RowProps {
	deviceId: string;
	model: HostedModel;
	overview: ModelsOverview;
	usage: ModelUsage | undefined;
	consumers: readonly Consumer[];
	labels: Labels;
	name(userId: string): string | undefined;
}

function EngineCell({
	model,
	runtimes,
}: Readonly<{ model: HostedModel; runtimes: ModelsOverview["runtimes"] }>) {
	const { t } = useTranslation("devices");
	const backend = backendOf(model.engine, runtimes);
	return (
		<>
			{engineLabel(t, model.engine)}
			{backend ? <CellSub>{backendLabel(t, backend)}</CellSub> : null}
		</>
	);
}

function StateCell({ model }: Readonly<{ model: HostedModel }>) {
	const { t } = useTranslation("devices");
	return (
		<>
			<StatusChip tone={STATE_TONE[model.state]} wrap>
				{stateLabel(t, model.state)}
			</StatusChip>
			{model.state === "failed" ? (
				<CellSub>{failureLabel(t, model.reason)}</CellSub>
			) : null}
		</>
	);
}

/** The row's numbers as text; `null` where the device reports none. */
function rowNumbers(
	t: DevicesT,
	locale: string,
	model: HostedModel,
	usage: ModelUsage | undefined,
) {
	const slots = slotsOf(model);
	const speed = usage?.tokensPerSecond;
	const decimal = new Intl.NumberFormat(locale, { maximumFractionDigits: 1 });
	return {
		slots: slots
			? t(
					"devices:models.table.slotsValue",
					"{{busy, number}} of {{slots, number}}",
					slots,
				)
			: null,
		speed: speed === undefined ? null : decimal.format(speed),
		requests:
			usage === undefined ? null : compactNumber(locale, usage.requests),
	};
}

function ModelRow(props: Readonly<RowProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { model, labels } = props;
	const numbers = rowNumbers(t, time.locale, model, props.usage);
	const residency = residencyLabel(t, model.residency);
	const kind = kindLabel(t, model.kind);
	return (
		<Tr data-model={model.id}>
			<Td label={labels.model} kind="name">
				<span className="font-medium">{model.display_name}</span>
				<CellSub>{residency}</CellSub>
			</Td>
			<Td label={labels.kind}>{kind}</Td>
			<Td label={labels.engine}>
				<EngineCell model={model} runtimes={props.overview.runtimes} />
			</Td>
			<Td label={labels.state}>
				<StateCell model={model} />
			</Td>
			<Td label={labels.memory} emptyDash>
				<MemoryCell model={model} />
			</Td>
			<Td label={labels.slots} kind="num" emptyDash>
				{numbers.slots}
			</Td>
			<Td label={labels.speed} kind="num" emptyDash>
				{numbers.speed}
			</Td>
			<Td label={labels.requests} kind="num" emptyDash>
				{numbers.requests}
			</Td>
			<Td label={labels.usedBy} emptyDash>
				<UsedBy consumers={props.consumers} name={props.name} />
			</Td>
			<Td label={labels.actions} kind="act">
				<ModelRowActions deviceId={props.deviceId} model={model} />
			</Td>
		</Tr>
	);
}

/** Grant id → account from the device's verified access rules. */
function useGrantUsers(target: ObserveTarget) {
	const grants = target.policy?.grants;
	return useMemo(() => {
		const users = new Map<string, string>();
		for (const grant of grants ?? []) users.set(grant.grant_id, grant.user_id);
		return {
			users: [...new Set(users.values())],
			grantUser: (grantId: string) => users.get(grantId),
		};
	}, [grants]);
}

interface ModelsBlockProps {
	target: ObserveTarget;
	overview: ModelsOverview;
	stamp: FreshnessStampProps;
}

function ModelsTable({ target, overview }: Readonly<ModelsBlockProps>) {
	const { t } = useTranslation("devices");
	const { deviceId } = target;
	const labels = columnLabels(t);
	const ids = useMemo(
		() => overview.models.map((model) => model.id),
		[overview.models],
	);
	const dayUsage = useModelsDayUsage(deviceId, ids);
	const grants = useGrantUsers(target);
	const name = usePersonNames(grants.users);
	const context = {
		owner: target.owner,
		myGrantId: target.view?.keys.grantId,
		grantUser: grants.grantUser,
	};
	return (
		<DvTable
			label={t("devices:models.table.label", "Models on {{device}}", {
				device: target.name,
			})}
			cols={COLS}
			head={
				<tr>
					<Th>{labels.model}</Th>
					<Th>{labels.kind}</Th>
					<Th>{labels.engine}</Th>
					<Th>{labels.state}</Th>
					<Th>{labels.memory}</Th>
					<Th numeric>{labels.slots}</Th>
					<Th numeric>{labels.speed}</Th>
					<Th numeric>{labels.requests}</Th>
					<Th>{labels.usedBy}</Th>
					<Th>
						<span className="sr-only">{labels.actions}</span>
					</Th>
				</tr>
			}
		>
			{overview.models.map((model) => {
				const stats = dayUsage.get(model.id);
				const usage = stats === undefined ? undefined : usageOf(stats);
				const consumers =
					usage === undefined ? [] : consumersOf(usage.consumers, context);
				return (
					<ModelRow
						key={model.id}
						deviceId={deviceId}
						model={model}
						overview={overview}
						usage={usage}
						consumers={consumers}
						labels={labels}
						name={name}
					/>
				);
			})}
		</DvTable>
	);
}

/** The hosted models: what runs, how busy it is and who uses it; empty with one way to add one. */
export function ModelsBlock(props: Readonly<ModelsBlockProps>) {
	const { t } = useTranslation("devices");
	const { target, overview, stamp } = props;
	const models = overview.models.length;
	return (
		<Block
			id="models-list"
			icon={Boxes}
			title={t("devices:models.table.title", "Models")}
			count={models}
			stamp={<FreshnessStamp {...stamp} />}
			tools={models ? <AddModelButton deviceId={target.deviceId} /> : null}
			flush={models > 0}
			foot={
				models
					? t(
							"devices:models.table.foot",
							"Requests, speed and who used a model cover the last 24 hours. Memory and busy slots are what the device reports now.",
						)
					: null
			}
		>
			{models ? (
				<ModelsTable {...props} />
			) : (
				<StateView
					kind="empty"
					icon={Boxes}
					title={t(
						"devices:models.states.emptyTitle",
						"No models on {{device}} yet",
						{ device: target.name },
					)}
					text={t(
						"devices:models.states.emptyText",
						"Add a model from the Flow-Like hub, Hugging Face or your own Bits. The device downloads it itself and serves it to your apps.",
					)}
					actions={<AddModelButton deviceId={target.deviceId} />}
				/>
			)}
		</Block>
	);
}
