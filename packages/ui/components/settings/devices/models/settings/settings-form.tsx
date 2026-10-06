"use client";

import { useTranslation } from "@flow-like/locales";
import { Check } from "lucide-react";
import {
	ENGINE_FIELDS,
	type KvCacheType,
	type SettingsField,
	cacheNeedsFlash,
	flashForCache,
} from "../../../../../lib/device-management/model/models/fit";
import {
	MODEL_DEFAULT_IDLE_UNLOAD_SECONDS,
	MODEL_MAX_CTX_PER_SLOT,
	MODEL_MIN_CTX_PER_SLOT,
	type ModelEngine,
	type ModelSettings,
	type Residency,
} from "../../../../../lib/device-management/models";
import type { DevicesT } from "../../primitives/area-context";
import { DvButton } from "../../primitives/dv-button";
import {
	ChoiceCards,
	type ChoiceOption,
	DvSelect,
	type DvSelectOption,
	Field,
} from "../../primitives/form-fields";
import { GateInline } from "../../primitives/gate-notice";
import { residencyLabel } from "../models-copy";
import {
	ALL_LAYERS,
	deviceDefaultLabel,
	fieldHint,
	fieldLabel,
	flashNeededText,
	valueLabel,
} from "./settings-copy";

/*
 * Every engine setting of a hosted model (plan §3.7) with its recommended
 * value and why, and the model's residency. A field the model leaves out
 * shows "Device default": the device picks its value when the model loads.
 */

export interface SettingsAdviceView {
	/** A field left out recommends the device's default. */
	recommended: ModelSettings;
	/** One sentence per field: why its recommended value. */
	reasons: Partial<Record<SettingsField, string>>;
	residency: Residency;
	residencyReason: string;
}

export interface SettingsFormProps {
	id: string;
	engine: ModelEngine;
	value: ModelSettings;
	onChange(value: ModelSettings): void;
	residency: Residency;
	onResidency(value: Residency): void;
	advice: SettingsAdviceView;
	/** The model's context limit, when known. */
	contextLimit?: number;
	cores: number;
	/** Each field offers "Device default", which leaves it out. */
	deviceDefaults?: boolean;
}

interface ControlProps {
	value: ModelSettings;
	set(patch: ModelSettings): void;
	contextLimit?: number;
	cores: number;
	deviceDefaults: boolean;
}

interface SelectSpec {
	value: string;
	options: DvSelectOption<string>[];
	onValueChange(value: string): void;
	/** Why an option is off. */
	note?: string;
}

const CTX_STEPS = [
	512, 1_024, 2_048, 4_096, 8_192, 16_384, 32_768, 65_536, 131_072, 262_144,
];
const SLOT_STEPS = [1, 2, 3, 4, 6, 8, 12, 16];
const THREAD_STEPS = [1, 2, 4, 6, 8, 12, 16, 24, 32, 48, 64, 96, 128];
const IDLE_MINUTES = [5, 15, 30, 60, 240, 1_440];
const KV_TYPES: readonly KvCacheType[] = ["f16", "q8_0", "q4_0"];

/** Distinct, ascending, with the current value kept. */
function steps(values: readonly number[], current: number | undefined) {
	const all = new Set(values);
	if (current !== undefined) all.add(current);
	return [...all].sort((a, b) => a - b);
}

export const sameValue = (a: unknown, b: unknown) =>
	JSON.stringify(a) === JSON.stringify(b);

/** The option key of a setting's value; "Device default" can't collide with JSON. */
const keyOf = (value: unknown) => JSON.stringify(value);
const DEVICE_DEFAULT = "device-default";

interface Choices {
	values: readonly unknown[];
	/** Values the field can't take now, and why. */
	off?: { values: readonly unknown[]; note: string };
}

/** A field's select: its values, and "Device default" when the form offers it or the model leaves the field out. */
function fieldSelect(
	t: DevicesT,
	field: SettingsField,
	choices: Choices,
	props: Readonly<ControlProps>,
) {
	const current = props.value[field];
	const options: DvSelectOption<string>[] = [];
	if (props.deviceDefaults || current === undefined)
		options.push({ value: DEVICE_DEFAULT, label: deviceDefaultLabel(t) });
	const byKey = new Map<string, unknown>();
	const off = new Set((choices.off?.values ?? []).map(keyOf));
	for (const value of choices.values) {
		byKey.set(keyOf(value), value);
		options.push({
			value: keyOf(value),
			label: valueLabel(t, field, { [field]: value } as ModelSettings),
			disabled: off.has(keyOf(value)),
		});
	}
	const spec: SelectSpec = {
		value: current === undefined ? DEVICE_DEFAULT : keyOf(current),
		options,
		onValueChange: (key) =>
			props.set({ [field]: byKey.get(key) } as ModelSettings),
		...(off.size ? { note: choices.off?.note } : {}),
	};
	return spec;
}

/** The context limit within what a device accepts per slot. */
const withinProtocol = (limit: number) =>
	Math.min(MODEL_MAX_CTX_PER_SLOT, Math.max(MODEL_MIN_CTX_PER_SLOT, limit));

function ctxSelect(t: DevicesT, props: Readonly<ControlProps>) {
	const limit = props.contextLimit
		? withinProtocol(props.contextLimit)
		: undefined;
	const values: number[] = limit ? [limit] : [];
	for (const step of CTX_STEPS)
		if (step >= MODEL_MIN_CTX_PER_SLOT && (!limit || step <= limit))
			values.push(step);
	return fieldSelect(
		t,
		"ctx_per_slot",
		{ values: steps(values, props.value.ctx_per_slot) },
		props,
	);
}

const slotsSelect = (t: DevicesT, props: Readonly<ControlProps>) =>
	fieldSelect(
		t,
		"parallel",
		{ values: steps(SLOT_STEPS, props.value.parallel) },
		props,
	);

function threadsSelect(t: DevicesT, props: Readonly<ControlProps>) {
	const most = Math.max(8, props.cores * 2);
	const values = [props.cores];
	for (const count of THREAD_STEPS) if (count <= most) values.push(count);
	return fieldSelect(
		t,
		"threads",
		{ values: steps(values, props.value.threads) },
		props,
	);
}

const kvSelect = (t: DevicesT, props: Readonly<ControlProps>) =>
	fieldSelect(t, "kv_cache_type", { values: KV_TYPES }, props);

type GpuLayers = NonNullable<ModelSettings["gpu_layers"]>;

function gpuLayersSelect(t: DevicesT, props: Readonly<ControlProps>) {
	const values: GpuLayers[] = ["auto", { count: 0 }, { count: ALL_LAYERS }];
	const current = props.value.gpu_layers;
	if (current && !values.some((value) => sameValue(value, current)))
		values.push(current);
	return fieldSelect(t, "gpu_layers", { values }, props);
}

/** Off can't go with an 8-bit or 4-bit cache. */
function flashSelect(t: DevicesT, props: Readonly<ControlProps>) {
	const off = cacheNeedsFlash(props.value)
		? { values: [false], note: flashNeededText(t) }
		: undefined;
	return fieldSelect(
		t,
		"flash_attn",
		{ values: [true, false], ...(off ? { off } : {}) },
		props,
	);
}

const SELECTS: Record<
	SettingsField,
	(t: DevicesT, props: Readonly<ControlProps>) => SelectSpec
> = {
	ctx_per_slot: ctxSelect,
	parallel: slotsSelect,
	kv_cache_type: kvSelect,
	gpu_layers: gpuLayersSelect,
	threads: threadsSelect,
	flash_attn: flashSelect,
};

const idleSeconds = (residency: Residency) =>
	residency.mode === "on_demand"
		? residency.idle_unload_after_seconds
		: MODEL_DEFAULT_IDLE_UNLOAD_SECONDS;

export const sameResidency = (a: Residency, b: Residency) =>
	a.mode === b.mode && idleSeconds(a) === idleSeconds(b);

interface RecommendedLineProps {
	isRecommended: boolean;
	value: string;
	reason?: string;
	onUse(): void;
}

/** "Recommended. Every layer fits on the GPU." or "Recommended: 8-bit. … Use recommended". */
function RecommendedLine({
	isRecommended,
	value,
	reason,
	onUse,
}: Readonly<RecommendedLineProps>) {
	const { t } = useTranslation("devices");
	if (isRecommended)
		return (
			<p
				data-recommended=""
				className="flex items-start gap-1 text-xs text-ink-2"
			>
				<Check aria-hidden className="mt-px size-3.25 shrink-0 text-good" />
				<span>
					<b className="font-medium">
						{t("devices:models.settings.recommended", "Recommended.")}
					</b>{" "}
					{reason}
				</span>
			</p>
		);
	return (
		<p data-recommended="other" className="text-xs text-muted-foreground">
			{t(
				"devices:models.settings.recommendedValue",
				"Recommended: {{value}}.",
				{
					value,
				},
			)}{" "}
			{reason}{" "}
			<DvButton
				variant="link"
				size="xs"
				className="h-auto p-0 text-xs"
				onClick={onUse}
			>
				{t("devices:models.settings.useRecommended", "Use recommended")}
			</DvButton>
		</p>
	);
}

interface FieldRowProps extends ControlProps {
	formId: string;
	field: SettingsField;
	advice: SettingsAdviceView;
}

function recommendation(t: DevicesT, props: Readonly<FieldRowProps>) {
	const { field, advice, value } = props;
	const recommended = advice.recommended[field];
	return {
		isRecommended: sameValue(value[field], recommended),
		value: valueLabel(t, field, advice.recommended),
		reason: advice.reasons[field],
		onUse: () => props.set({ [field]: recommended }),
	};
}

function FieldRow(props: Readonly<FieldRowProps>) {
	const { t } = useTranslation("devices");
	const { field, formId } = props;
	const spec = SELECTS[field](t, props);
	const id = `${formId}-${field}`;
	return (
		<div data-setting={field} className="flex flex-col gap-1.5">
			<Field id={id} label={fieldLabel(t, field)} hint={fieldHint(t, field)}>
				<DvSelect
					value={spec.value}
					onValueChange={spec.onValueChange}
					options={spec.options}
				/>
			</Field>
			{spec.note ? (
				<GateInline kind="unsupported" className="max-w-none">
					{spec.note}
				</GateInline>
			) : null}
			<RecommendedLine {...recommendation(t, props)} />
		</div>
	);
}

function idleOptions(t: DevicesT, current: number) {
	const options: DvSelectOption<string>[] = [];
	for (const minutes of steps(IDLE_MINUTES, current))
		options.push({
			value: String(minutes),
			label: residencyLabel(t, {
				mode: "on_demand",
				idle_unload_after_seconds: minutes * 60,
			}),
		});
	return options;
}

type ResidencyProps = Pick<
	SettingsFormProps,
	"id" | "residency" | "onResidency" | "advice"
>;

function IdleField({ id, residency, onResidency }: Readonly<ResidencyProps>) {
	const { t } = useTranslation("devices");
	const minutes = Math.round(idleSeconds(residency) / 60);
	return (
		<Field
			id={`${id}-idle`}
			label={t("devices:models.settings.residency.idle", "Unload after")}
		>
			<DvSelect
				value={String(minutes)}
				onValueChange={(value) =>
					onResidency({
						mode: "on_demand",
						idle_unload_after_seconds: Number(value) * 60,
					})
				}
				options={idleOptions(t, minutes)}
			/>
		</Field>
	);
}

function residencyChoices(
	t: DevicesT,
	props: Readonly<ResidencyProps>,
): ChoiceOption<Residency["mode"]>[] {
	return [
		{
			value: "on_demand",
			title: t("devices:models.settings.residency.onDemand", "Loads on demand"),
			hint: t(
				"devices:models.settings.residency.onDemandHint",
				"The first request loads it; it unloads after a while without requests.",
			),
			detail: <IdleField {...props} />,
		},
		{
			value: "always_on",
			title: t("devices:models.settings.residency.alwaysOn", "Always loaded"),
			hint: t(
				"devices:models.settings.residency.alwaysOnHint",
				"Answers without a start-up wait and keeps its memory all the time.",
			),
		},
		{
			value: "pinned_off",
			title: t("devices:models.settings.residency.pinnedOff", "Kept off"),
			hint: t(
				"devices:models.settings.residency.pinnedOffHint",
				"Never loads, not even for a request, until you change this.",
			),
		},
	];
}

const residencyOf = (mode: Residency["mode"], current: Residency): Residency =>
	mode === "on_demand"
		? { mode, idle_unload_after_seconds: idleSeconds(current) }
		: { mode };

function ResidencyField(props: Readonly<ResidencyProps>) {
	const { t } = useTranslation("devices");
	const { id, residency, onResidency, advice } = props;
	return (
		<div data-setting="residency" className="flex flex-col gap-1.5">
			<ChoiceCards
				id={`${id}-residency`}
				legend={t("devices:models.settings.residency.label", "When it runs")}
				value={residency.mode}
				onValueChange={(mode) => onResidency(residencyOf(mode, residency))}
				options={residencyChoices(t, props)}
			/>
			<RecommendedLine
				isRecommended={sameResidency(residency, advice.residency)}
				value={residencyLabel(t, advice.residency)}
				reason={advice.residencyReason}
				onUse={() => onResidency(advice.residency)}
			/>
		</div>
	);
}

export function SettingsForm(props: Readonly<SettingsFormProps>) {
	const { engine, value, onChange, advice } = props;
	const set = (patch: ModelSettings) =>
		onChange(flashForCache(engine, { ...value, ...patch }));
	return (
		<div data-settings-form="" className="flex flex-col gap-4">
			{ENGINE_FIELDS[engine].map((field) => (
				<FieldRow
					key={field}
					formId={props.id}
					field={field}
					value={value}
					set={set}
					advice={advice}
					contextLimit={props.contextLimit}
					cores={props.cores}
					deviceDefaults={props.deviceDefaults === true}
				/>
			))}
			<ResidencyField {...props} />
		</div>
	);
}
