"use client";

import { useTranslation } from "@flow-like/locales";
import { Lock, Undo2, Variable } from "lucide-react";
import { useState } from "react";
import {
	botTokenEventId,
	isBotTokenKey,
} from "../../../../../lib/device-management/bot-config";
import {
	type DeploymentVariable,
	type PlacementConfiguration,
	variableText,
} from "../../../../../lib/device-management/deployment";
import type {
	DeployOverrides,
	PlanTarget,
	PlannedService,
} from "../../../../../lib/device-management/model/deploy-plan";
import { eventTypeLabel } from "../../copy/eligibility-copy";
import type { DevicesT } from "../../primitives/area-context";
import { Block } from "../../primitives/block";
import { DvButton } from "../../primitives/dv-button";
import {
	DvInput,
	DvTextarea,
	SecretInput,
	SwitchField,
} from "../../primitives/form-fields";
import { Segmented } from "../../primitives/segmented";
import { StateView } from "../../primitives/state-view";
import { WizardStepHeader } from "../../primitives/wizard";
import { eventName, issueText, planNames } from "../deploy-copy";
import {
	type UnresolvedOverride,
	eventVariables,
	savedTokenOf,
	variableUsers,
} from "../deploy-facts";
import { HubStamp, Note, TargetsStamp } from "../deploy-parts";
import type { PlanStepProps } from "../step-props";

/* Step 4 · Settings (APP §3.8): one table of the variables the chosen events use; a value belongs to the service, a device may differ. */

type State = PlanStepProps["state"];

const isJson = (variable: DeploymentVariable) =>
	variable.value_type !== "Normal";

const number = (t: DevicesT) =>
	t("devices:deploy.settings.typeNumber", "Number");

const SCALAR_LABEL: Record<string, (t: DevicesT) => string> = {
	Boolean: (t) => t("devices:deploy.settings.typeBoolean", "Yes or no"),
	Integer: number,
	Float: number,
	Byte: number,
	PathBuf: (t) => t("devices:deploy.settings.typePath", "Path"),
	Date: (t) => t("devices:deploy.settings.typeDate", "Date"),
};

/** The variable's type in plain words; lists and maps are typed as JSON. */
function typeLabel(t: DevicesT, variable: DeploymentVariable): string {
	if (variable.value_type === "HashMap")
		return t("devices:deploy.settings.typeMap", "Map");
	if (isJson(variable)) return t("devices:deploy.settings.typeList", "List");
	const scalar = SCALAR_LABEL[variable.data_type];
	return scalar ? scalar(t) : t("devices:deploy.settings.typeText", "Text");
}

/** The configuration a target's update starts from, once it was read live. */
function existingOf(
	state: State,
	target: PlanTarget,
	serviceKey: string,
): PlacementConfiguration | undefined {
	const service = target.services.find((row) => row.key === serviceKey);
	if (!service || service.kind === "new") return undefined;
	return state.configurations[target.deviceId]?.find(
		(row) => row.placement_id === service.serviceId,
	);
}

interface Stored {
	target: PlanTarget;
	/** The stored value as text; undefined for a secret. */
	text?: string;
	removed: boolean;
}

/** Values the updated services already have for this variable. */
function storedValues(
	state: State,
	service: PlannedService,
	variable: DeploymentVariable,
): Stored[] {
	return state.plan.targets.flatMap((target) => {
		const existing = existingOf(state, target, service.key);
		if (!existing) return [];
		const { variables, secret_overrides } = existing.config;
		const has = variable.secret
			? Object.hasOwn(secret_overrides, variable.id)
			: Object.hasOwn(variables, variable.id);
		if (!has) return [];
		const own = state.draft.targets.find(
			(row) => row.deviceId === target.deviceId,
		);
		return [
			{
				target,
				removed: own?.over.removeOverrides?.includes(variable.id) ?? false,
				...(variable.secret
					? {}
					: { text: variableText(variable, variables[variable.id]) }),
			},
		];
	});
}

const without = <T,>(record: Record<string, T>, id: string) =>
	Object.fromEntries(Object.entries(record).filter(([key]) => key !== id));

function setOver(
	state: State,
	deviceId: string,
	change: (over: DeployOverrides) => DeployOverrides,
) {
	state.updateTarget(deviceId, (target) => ({
		...target,
		over: change(target.over),
	}));
}

function setRemoved(
	state: State,
	stored: readonly Stored[],
	id: string,
	on: boolean,
) {
	for (const row of stored)
		setOver(state, row.target.deviceId, (over) => ({
			...over,
			removeOverrides: on
				? [...new Set([...(over.removeOverrides ?? []), id])]
				: (over.removeOverrides ?? []).filter((value) => value !== id),
		}));
}

function setShared(
	{ state, update }: PlanStepProps,
	variable: DeploymentVariable,
	text: string | undefined,
) {
	const { draft } = state;
	const field = variable.secret ? "secrets" : "vars";
	const values =
		text === undefined
			? without(draft[field], variable.id)
			: { ...draft[field], [variable.id]: text };
	const edited =
		text === undefined
			? draft.edited.filter((id) => id !== variable.id)
			: [...new Set([...draft.edited, variable.id])];
	update({ [field]: values, edited });
}

function setOwn(
	props: PlanStepProps,
	variable: DeploymentVariable,
	deviceId: string,
	text: string | undefined,
) {
	const field = variable.secret ? "secrets" : "vars";
	setOver(props.state, deviceId, (over) => ({
		...over,
		[field]:
			text === undefined
				? without(over[field] ?? {}, variable.id)
				: { ...over[field], [variable.id]: text },
	}));
	if (text !== undefined && !props.state.draft.edited.includes(variable.id))
		props.update({ edited: [...props.state.draft.edited, variable.id] });
}

function ValueInput({
	variable,
	id,
	label,
	value,
	invalid,
	onChange,
}: Readonly<{
	variable: DeploymentVariable;
	id: string;
	label: string;
	value: string;
	invalid: boolean;
	onChange(text: string): void;
}>) {
	const { t } = useTranslation("devices");
	if (isJson(variable))
		return (
			<DvTextarea
				id={id}
				aria-label={label}
				aria-invalid={invalid || undefined}
				rows={3}
				spellCheck={false}
				className="font-mono"
				value={value}
				onChange={(event) => onChange(event.target.value)}
			/>
		);
	if (variable.data_type === "Boolean")
		return (
			<Segmented
				label={label}
				value={value === "true" ? "true" : "false"}
				onChange={onChange}
				options={[
					{ value: "true", label: t("deploy.settings.yes", "Yes") },
					{ value: "false", label: t("deploy.settings.no", "No") },
				]}
			/>
		);
	const numeric = ["Integer", "Float", "Byte"].includes(variable.data_type);
	return (
		<DvInput
			id={id}
			aria-label={label}
			aria-invalid={invalid || undefined}
			mono={variable.data_type === "PathBuf"}
			numeric={numeric}
			inputMode={numeric ? "decimal" : undefined}
			spellCheck={false}
			value={value}
			onChange={(event) => onChange(event.target.value)}
		/>
	);
}

interface RowProps extends PlanStepProps {
	service: PlannedService;
	variable: DeploymentVariable;
}

function PerDeviceValues(props: Readonly<RowProps & { shared: string }>) {
	const { t } = useTranslation("devices");
	const { state, variable, shared, check } = props;
	const { plan, draft } = state;
	return (
		<ul className="flex flex-col gap-1.5 rounded-lg border border-hairline bg-surface-sunken px-3 py-2">
			{plan.targets.map((target) => {
				const own = draft.targets.find(
					(row) => row.deviceId === target.deviceId,
				)?.over.vars?.[variable.id];
				const invalid = check.issues.some(
					(issue) =>
						issue.step === "settings" &&
						issue.deviceId === target.deviceId &&
						issue.params?.variable === variable.name,
				);
				return (
					<li
						key={target.deviceId}
						className="grid grid-cols-[minmax(0,160px)_minmax(0,1fr)_auto] items-center gap-2 @max-[560px]/settingsstep:grid-cols-1"
					>
						<span className="truncate font-mono text-ui">{target.name}</span>
						{own === undefined ? (
							<span className="text-ui text-muted-foreground">
								{t("deploy.settings.sameAsShared", "Same as shared")}
							</span>
						) : (
							<ValueInput
								variable={variable}
								id={`deploy-var-${variable.id}-${target.deviceId}`}
								label={t(
									"deploy.settings.valueOn",
									"{{variable}} on {{device}}",
									{ variable: variable.name, device: target.name },
								)}
								value={own}
								invalid={invalid}
								onChange={(text) =>
									setOwn(props, variable, target.deviceId, text)
								}
							/>
						)}
						<DvButton
							size="xs"
							variant={own === undefined ? "default" : "ghost"}
							onClick={() =>
								setOwn(
									props,
									variable,
									target.deviceId,
									own === undefined ? shared : undefined,
								)
							}
						>
							{own === undefined
								? t("deploy.settings.setForDevice", "Set for this device")
								: t("deploy.settings.useShared", "Use shared")}
						</DvButton>
					</li>
				);
			})}
		</ul>
	);
}

function PlainValue(props: Readonly<RowProps & { stored: readonly Stored[] }>) {
	const { t } = useTranslation("devices");
	const { state, variable, stored, check } = props;
	const { draft, plan } = state;
	const multi = plan.targets.length > 1;
	const differs = draft.targets.some(
		(target) => target.over.vars?.[variable.id] !== undefined,
	);
	const [perDevice, setPerDevice] = useState(differs);
	const current = stored.find((row) => !row.removed)?.text;
	const text = draft.vars[variable.id] ?? current ?? "";
	const invalid = check.issues.some(
		(issue) =>
			issue.step === "settings" && issue.params?.variable === variable.name,
	);
	return (
		<>
			<ValueInput
				variable={variable}
				id={`deploy-var-${variable.id}`}
				label={variable.name}
				value={text}
				invalid={invalid}
				onChange={(next) => setShared(props, variable, next)}
			/>
			{isJson(variable) ? (
				<p className="text-xs text-muted-foreground">
					{variable.value_type === "HashMap"
						? t("deploy.settings.jsonMap", "A JSON map of values.")
						: t("deploy.settings.jsonList", "A JSON list of values.")}
				</p>
			) : null}
			{current !== undefined && current !== text ? (
				<p className="text-xs text-muted-foreground">
					{t("deploy.settings.now", "Now:")}{" "}
					<code className="rounded-sm bg-muted px-1 font-mono">{current}</code>
				</p>
			) : null}
			{multi ? (
				<SwitchField
					id={`deploy-var-${variable.id}-differs`}
					checked={perDevice || differs}
					onCheckedChange={(on) => {
						setPerDevice(on);
						if (!on)
							for (const target of plan.targets)
								setOwn(props, variable, target.deviceId, undefined);
					}}
				>
					{t("deploy.settings.differs", "Different on some devices")}
				</SwitchField>
			) : null}
			{multi && (perDevice || differs) ? (
				<PerDeviceValues {...props} shared={text} />
			) : null}
		</>
	);
}

function SecretValue(
	props: Readonly<RowProps & { stored: readonly Stored[] }>,
) {
	const { t } = useTranslation("devices");
	const { state, variable, stored } = props;
	const { draft, plan } = state;
	const kept = stored.some((row) => !row.removed);
	const typed = draft.secrets[variable.id] !== undefined;
	const [setting, setSetting] = useState(typed);
	const entering = !kept || setting || typed;
	const perDevice =
		draft.secretsMode === "per_device" && plan.targets.length > 1;
	const hint = t(
		"deploy.settings.secretHint",
		"Stored encrypted on the device, never read back",
	);
	return (
		<>
			{kept ? (
				<Segmented
					label={t("deploy.settings.secretValue", "{{variable}} value", {
						variable: variable.name,
					})}
					value={entering ? "new" : "keep"}
					onChange={(next) => {
						setSetting(next === "new");
						if (next === "keep") setShared(props, variable, undefined);
					}}
					options={[
						{
							value: "keep",
							label: t("deploy.settings.keepStored", "Keep stored"),
						},
						{ value: "new", label: t("deploy.settings.setNew", "Set new") },
					]}
				/>
			) : null}
			{!entering ? (
				<p className="flex items-center gap-1.5 text-ui text-ink-2">
					<Lock aria-hidden className="size-3.5 text-muted-foreground" />
					{t(
						"deploy.settings.storedSecret",
						"Stored secret · can't be read back, not even by you.",
					)}
				</p>
			) : perDevice ? (
				<ul className="flex flex-col gap-2">
					{plan.targets.map((target) => (
						<li key={target.deviceId} className="flex flex-col gap-1">
							<span className="font-mono text-ui">{target.name}</span>
							<SecretInput
								id={`deploy-secret-${variable.id}-${target.deviceId}`}
								aria-label={t(
									"deploy.settings.valueOn",
									"{{variable}} on {{device}}",
									{ variable: variable.name, device: target.name },
								)}
								autoComplete="new-password"
								value={
									draft.targets.find((row) => row.deviceId === target.deviceId)
										?.over.secrets?.[variable.id] ?? ""
								}
								onValueChange={(text) =>
									setOwn(props, variable, target.deviceId, text)
								}
								minBytes={1}
								maxBytes={4096}
							/>
						</li>
					))}
				</ul>
			) : (
				<SecretInput
					id={`deploy-secret-${variable.id}`}
					aria-label={variable.name}
					autoComplete="new-password"
					value={draft.secrets[variable.id] ?? ""}
					onValueChange={(text) => setShared(props, variable, text)}
					minBytes={1}
					maxBytes={4096}
				/>
			)}
			{entering ? (
				<p className="text-xs text-muted-foreground">{hint}</p>
			) : null}
		</>
	);
}

function isOn(
	state: State,
	variable: DeploymentVariable,
	stored: readonly Stored[],
): boolean {
	const { draft } = state;
	const typed = variable.secret
		? draft.secrets[variable.id] !== undefined ||
			draft.targets.some(
				(target) => target.over.secrets?.[variable.id] !== undefined,
			)
		: draft.vars[variable.id] !== undefined;
	return typed || stored.some((row) => !row.removed);
}

type TokenSource = "keep" | "saved" | "enter";

/** The devices this plan sends the bot to; a token entered for a device lands there. */
function botDevices(state: State, eventId: string): string[] {
	return state.plan.targets
		.filter((target) =>
			target.services.some((service) => service.events.includes(eventId)),
		)
		.map((target) => target.deviceId);
}

/**
 * Writes a bot's token where the plan reads it: shared, or for each device
 * that gets the bot when secrets differ per device. `undefined` takes the
 * value back (the token saved in Events, or the one stored on the device).
 */
function setBotToken(
	props: PlanStepProps,
	key: string,
	eventId: string,
	value: string | undefined,
) {
	const { state, update } = props;
	const { draft } = state;
	const edited =
		value === undefined
			? draft.edited.filter((id) => id !== key)
			: [...new Set([...draft.edited, key])];
	if (draft.secretsMode === "same") {
		update({
			secrets:
				value === undefined
					? without(draft.secrets, key)
					: { ...draft.secrets, [key]: value },
			edited,
		});
		return;
	}
	for (const deviceId of botDevices(state, eventId))
		setOver(state, deviceId, (over) => ({
			...over,
			secrets:
				value === undefined
					? without(over.secrets ?? {}, key)
					: { ...over.secrets, [key]: value },
		}));
	update({ edited });
}

/** The token a person entered for the bot: shared, else the first device's. */
function enteredValue(state: State, key: string): string | undefined {
	const { draft } = state;
	return (
		draft.secrets[key] ??
		draft.targets.find((target) => target.over.secrets?.[key] !== undefined)
			?.over.secrets?.[key]
	);
}

function tokenSource(
	entered: string | undefined,
	saved: string | null,
	kept: boolean,
	setting: boolean,
): TokenSource {
	if (entered === undefined)
		return kept && !setting ? "keep" : saved ? "saved" : "enter";
	return saved !== null && entered === saved ? "saved" : "enter";
}

/**
 * A bot's token (§1.10): the token saved on the event in Events, or one
 * entered here; a service that has the bot keeps its stored token unless it
 * is set anew. Shape-checked here; it can't be read back from the device.
 */
function BotTokenRow(props: Readonly<RowProps>) {
	const { t } = useTranslation("devices");
	const { state, service, variable, check } = props;
	const { plan } = state;
	const key = variable.id;
	const eventId = botTokenEventId(key) ?? "";
	const event = plan.app?.events.find((row) => row.id === eventId);
	const saved = savedTokenOf(event);
	const kept = storedValues(state, service, variable).some(
		(row) => !row.removed,
	);
	const entered = enteredValue(state, key);
	const [setting, setSetting] = useState(entered !== undefined);
	const source = tokenSource(entered, saved, kept, setting);
	const label = t("deploy.settings.botToken.label", "Bot token of {{event}}", {
		event: variable.name,
	});
	const devices = botDevices(state, eventId);
	const [only] = devices;
	const device =
		devices.length === 1 && only
			? (plan.targets.find((target) => target.deviceId === only)?.name ?? only)
			: t("deploy.settings.botToken.eachDevice", "each device");
	const errors = check.issues
		.filter(
			(issue) =>
				issue.step === "settings" &&
				(issue.code === "bot_token_missing" ||
					issue.code === "bot_token_shape") &&
				issue.params?.event === eventId,
		)
		.map((issue) => issueText(t, issue, planNames(t, plan)));
	const choose = (next: TokenSource) => {
		setSetting(next !== "keep");
		if (next === "keep") setBotToken(props, key, eventId, undefined);
		// An added bot takes the saved token by itself; a kept one only when it is chosen.
		else if (next === "saved")
			setBotToken(props, key, eventId, kept ? (saved ?? "") : undefined);
		else setBotToken(props, key, eventId, "");
	};
	return (
		<li
			data-variable={key}
			data-bot-token={eventId}
			className="grid grid-cols-[minmax(0,230px)_minmax(0,1fr)] gap-x-4 gap-y-2 border-t border-hairline px-4 py-3 first:border-t-0 @max-[560px]/settingsstep:grid-cols-1"
		>
			<div className="flex min-w-0 flex-col gap-0.5">
				<b className="text-ui font-semibold">{label}</b>
				<span className="text-xs text-muted-foreground">
					{event ? eventTypeLabel(t, event.event_type) : null}
					{t("deploy.settings.secretTag", " · secret")}
				</span>
			</div>
			<div className="flex min-w-0 flex-col gap-2">
				{kept ? (
					<Segmented<"keep" | "new">
						label={t("deploy.settings.secretValue", "{{variable}} value", {
							variable: label,
						})}
						value={source === "keep" ? "keep" : "new"}
						onChange={(next) =>
							choose(next === "keep" ? "keep" : saved ? "saved" : "enter")
						}
						options={[
							{
								value: "keep",
								label: t("deploy.settings.keepStored", "Keep stored"),
							},
							{ value: "new", label: t("deploy.settings.setNew", "Set new") },
						]}
					/>
				) : null}
				{source === "keep" ? (
					<p className="flex items-center gap-1.5 text-ui text-ink-2">
						<Lock aria-hidden className="size-3.5 text-muted-foreground" />
						{t(
							"deploy.settings.storedSecret",
							"Stored secret · can't be read back, not even by you.",
						)}
					</p>
				) : (
					<>
						{saved ? (
							<Segmented<"saved" | "enter">
								label={label}
								value={source === "saved" ? "saved" : "enter"}
								onChange={choose}
								wrap
								options={[
									{
										value: "saved",
										label: t(
											"deploy.settings.botToken.saved",
											"Use the token saved in Events",
										),
									},
									{
										value: "enter",
										label: t("deploy.settings.botToken.enter", "Enter a token"),
									},
								]}
							/>
						) : null}
						{source === "enter" ? (
							<SecretInput
								id={`deploy-bot-token-${eventId}`}
								aria-label={label}
								autoComplete="new-password"
								value={entered ?? ""}
								onValueChange={(text) => setBotToken(props, key, eventId, text)}
								minBytes={1}
								maxBytes={4096}
							/>
						) : null}
						<p className="text-xs text-muted-foreground">
							{t(
								"deploy.settings.botToken.hint",
								"Stored on {{device}} as a secret. It can't be read back.",
								{ device },
							)}
						</p>
					</>
				)}
				{errors.map((error) => (
					<p key={error} className="text-xs text-critical">
						{error}
					</p>
				))}
			</div>
		</li>
	);
}

function VariableRow(props: Readonly<RowProps>) {
	const { t } = useTranslation("devices");
	const { state, service, variable, check } = props;
	const { plan } = state;
	const stored = storedValues(state, service, variable);
	const on = isOn(state, variable, stored);
	const users = variableUsers(plan.app, service.events, variable.id);
	const errors = [
		...new Set(
			check.issues
				.filter(
					(issue) =>
						issue.step === "settings" &&
						issue.params?.variable === variable.name,
				)
				.map((issue) => issueText(t, issue, planNames(t, plan))),
		),
	];
	const toggle = (next: boolean) => {
		setRemoved(state, stored, variable.id, !next);
		if (!next) setShared(props, variable, undefined);
		else if (!stored.length)
			setShared(
				props,
				variable,
				variable.data_type === "Boolean" && !isJson(variable) ? "false" : "",
			);
	};
	return (
		<li
			data-variable={variable.id}
			className="grid grid-cols-[minmax(0,230px)_minmax(0,1fr)] gap-x-4 gap-y-2 border-t border-hairline px-4 py-3 first:border-t-0 @max-[560px]/settingsstep:grid-cols-1"
		>
			<div className="flex min-w-0 flex-col gap-0.5">
				<b className="text-ui font-semibold">{variable.name}</b>
				<span className="text-xs text-muted-foreground">
					{typeLabel(t, variable)}
					{variable.secret ? t("deploy.settings.secretTag", " · secret") : ""}
				</span>
				<span className="text-xs text-ink-2">
					{t("deploy.settings.usedBy", "Used by {{events}}", {
						events: users.length
							? users.map((eventId) => eventName(plan, eventId)).join(", ")
							: t("deploy.settings.usedByApp", "the app"),
					})}
				</span>
			</div>
			<div className="flex min-w-0 flex-col gap-2">
				<SwitchField
					id={`deploy-var-${variable.id}-on`}
					checked={on}
					onCheckedChange={toggle}
				>
					{t("deploy.settings.valueForService", "Value for this service")}
				</SwitchField>
				{!on ? (
					<p className="text-ui text-muted-foreground">
						{variable.secret
							? t(
									"deploy.settings.secretUnset",
									"Not set. The flow gets no value.",
								)
							: t("deploy.settings.usesDefault", "Uses the app's default.")}
					</p>
				) : variable.secret ? (
					<SecretValue {...props} stored={stored} />
				) : (
					<PlainValue {...props} stored={stored} />
				)}
				{errors.map((error) => (
					<p key={error} className="text-xs text-critical">
						{error}
					</p>
				))}
			</div>
		</li>
	);
}

function unresolvedText(t: DevicesT, row: UnresolvedOverride): string {
	switch (row.issue) {
		case "outside":
			return t(
				"devices:deploy.settings.unresolvedOutside",
				"Set for an event this update no longer serves. Serve the event again, or remove the value.",
			);
		case "kind_changed":
			return t(
				"devices:deploy.settings.unresolvedKind",
				"It changed between a plain value and a secret. Enter a new value above, or remove it.",
			);
		case "type_changed":
			return t(
				"devices:deploy.settings.unresolvedType",
				"The stored secret was written as {{from}}; it is {{to}} now. Enter a new value above, or remove it.",
				{
					from: row.previous ? typeLabel(t, row.previous) : "",
					to: row.current ? typeLabel(t, row.current) : "",
				},
			);
		default:
			return t(
				"devices:deploy.settings.unresolvedInvalid",
				"The stored value doesn't fit its type any more. Enter a new value above, or remove it.",
			);
	}
}

interface RemovedRow {
	target: PlanTarget;
	id: string;
	name: string;
	secret: boolean;
}

/** A stored value by its variable's name; one whose variable is gone has only its id left. */
function StoredName({ name, id }: Readonly<{ name: string; id: string }>) {
	return (
		<b className={name === id ? "font-mono font-medium" : "font-semibold"}>
			{name}
		</b>
	);
}

/** What the update drops on purpose: every id in a target's `removeOverrides` that the service still has stored. */
function removedRows(state: State): RemovedRow[] {
	const { plan, draft } = state;
	return plan.targets.flatMap((target) => {
		const own = draft.targets.find((row) => row.deviceId === target.deviceId);
		return target.services.flatMap((service) => {
			const existing = existingOf(state, target, service.key);
			if (!existing) return [];
			const definitions = eventVariables(plan.app, service.events);
			return (own?.over.removeOverrides ?? [])
				.filter(
					(id) =>
						Object.hasOwn(existing.config.variables, id) ||
						Object.hasOwn(existing.config.secret_overrides, id),
				)
				.map((id) => ({
					target,
					id,
					name: definitions.find((row) => row.id === id)?.name ?? id,
					secret: Object.hasOwn(existing.config.secret_overrides, id),
				}));
		});
	});
}

function toggleRemoved(
	state: State,
	deviceId: string,
	id: string,
	on: boolean,
) {
	setOver(state, deviceId, (over) => ({
		...over,
		removeOverrides: on
			? [...new Set([...(over.removeOverrides ?? []), id])]
			: (over.removeOverrides ?? []).filter((value) => value !== id),
	}));
}

/** Updates: stored values that need a decision, then the ones this update removes. */
function StoredOverrides({ state }: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const removed = removedRows(state);
	const { unresolved } = state;
	const row =
		"flex flex-wrap items-center gap-2 border-t border-hairline px-4 py-2.5 text-ui first:border-t-0";
	return (
		<Block
			title={t("deploy.settings.removed", "Removed overrides")}
			icon={Undo2}
			count={removed.length}
			flush={removed.length + unresolved.length > 0}
			stamp={
				<TargetsStamp targets={state.plan.targets} devices={state.devices} />
			}
		>
			{unresolved.length ? (
				<ul
					aria-label={t(
						"deploy.settings.unresolvedLabel",
						"Stored values that need a decision",
					)}
					className="flex flex-col border-b border-warning-line bg-warning-bg"
				>
					{unresolved.map((item) => (
						<li
							key={`${item.deviceId}/${item.serviceId}/${item.variableId}`}
							data-unresolved={item.issue}
							className={row}
						>
							<span className="min-w-0 flex-1">
								<StoredName name={item.name} id={item.variableId} />{" "}
								<span className="text-ink-2">· {item.device}</span>
								<span className="block text-xs text-ink-2">
									{unresolvedText(t, item)}
								</span>
							</span>
							<DvButton
								size="sm"
								onClick={() =>
									toggleRemoved(state, item.deviceId, item.variableId, true)
								}
							>
								{t("deploy.settings.removeValue", "Remove value")}
							</DvButton>
						</li>
					))}
				</ul>
			) : null}
			{removed.length ? (
				<ul className="flex flex-col">
					{removed.map((item) => (
						<li key={`${item.id}/${item.target.deviceId}`} className={row}>
							<span className="min-w-0 flex-1">
								<StoredName name={item.name} id={item.id} />{" "}
								<span className="text-muted-foreground">
									· {item.target.name}
								</span>
								<span className="block text-xs text-muted-foreground">
									{item.secret
										? t(
												"deploy.settings.removedSecret",
												"After the update it has no value. The stored secret is deleted.",
											)
										: t(
												"deploy.settings.removedValue",
												"After the update it uses the app's default.",
											)}
								</span>
							</span>
							<DvButton
								size="sm"
								onClick={() =>
									toggleRemoved(state, item.target.deviceId, item.id, false)
								}
							>
								{t("deploy.settings.keepIt", "Keep it")}
							</DvButton>
						</li>
					))}
				</ul>
			) : unresolved.length ? null : (
				<p className="text-ui text-muted-foreground">
					{t(
						"deploy.settings.removedNone",
						"None. Every value set for these services carries over.",
					)}
				</p>
			)}
		</Block>
	);
}

function SecretsMode({ state, update }: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	return (
		<div className="flex flex-col gap-1.5 border-b border-hairline px-4 py-3">
			<div className="flex flex-wrap items-center gap-3">
				<span className="text-ui font-medium">
					{t("deploy.settings.secrets", "Secrets")}
				</span>
				<Segmented
					label={t("deploy.settings.secrets", "Secrets")}
					value={state.draft.secretsMode}
					onChange={(secretsMode) => update({ secretsMode })}
					options={[
						{
							value: "same",
							label: t("deploy.settings.secretsSame", "Same on every device"),
						},
						{
							value: "per_device",
							label: t(
								"deploy.settings.secretsPerDevice",
								"Different per device",
							),
						},
					]}
				/>
			</div>
			<p className="text-xs text-muted-foreground">
				{t(
					"deploy.settings.secretsHint",
					"Stored secrets can't be read back, not even by you. Each device stores its own encrypted copy.",
				)}
			</p>
		</div>
	);
}

function SettingsBlock(props: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const { state } = props;
	const { plan, draft } = state;
	const [tab, setTab] = useState(plan.services[0]?.key ?? "main");
	const planned =
		plan.services.find((row) => row.key === tab) ?? plan.services[0];
	const updating = draft.entry === "update";
	// An update sets the values of the services it updates, each with the events it serves.
	const updated = updating
		? plan.targets.flatMap((target) =>
				target.services.filter(
					(row) => row.key === planned.key && row.kind !== "new",
				),
			)
		: [];
	const service = updated.length
		? {
				...planned,
				events: [...new Set(updated.flatMap((row) => row.events))],
			}
		: planned;
	const variables = eventVariables(plan.app, service.events);
	const multi = plan.targets.length > 1;
	const name =
		updated.length === 1 ? (
			<span className="font-mono">{updated[0].serviceId}</span>
		) : updated.length > 1 ? (
			t("deploy.settings.forServices", "{{count, number}} services", {
				count: updated.length,
			})
		) : (
			<span className="font-mono">{service.id}</span>
		);
	return (
		<>
			<Block
				title={
					<>
						{t("deploy.settings.for", "Settings for")} {name}
					</>
				}
				icon={Variable}
				count={variables.length}
				stamp={
					updating ? (
						<TargetsStamp targets={plan.targets} devices={state.devices} />
					) : (
						<HubStamp />
					)
				}
				toolbar={
					plan.services.length > 1 ? (
						<Segmented
							label={t("deploy.settings.services", "Services")}
							value={service.key}
							onChange={setTab}
							wrap
							options={plan.services.map((row) => ({
								value: row.key,
								label: <span className="font-mono">{row.id}</span>,
							}))}
						/>
					) : undefined
				}
				flush
				foot={
					multi
						? t(
								"deploy.settings.footMulti",
								"Settings belong to the service, so events in one service share a value. A value set for one device applies to that device only. Secret values are stored encrypted on the device and can't be read back.",
							)
						: t(
								"deploy.settings.foot",
								"Settings belong to the service, so events in one service share a value. Secret values are stored encrypted on the device and can't be read back.",
							)
				}
			>
				{multi && variables.some((variable) => variable.secret) ? (
					<SecretsMode {...props} />
				) : null}
				{variables.length ? (
					<ul className="flex flex-col">
						{variables.map((variable) =>
							isBotTokenKey(variable.id) ? (
								<BotTokenRow
									key={variable.id}
									{...props}
									service={service}
									variable={variable}
								/>
							) : (
								<VariableRow
									key={variable.id}
									{...props}
									service={service}
									variable={variable}
								/>
							),
						)}
					</ul>
				) : (
					<div className="px-4 py-3">
						<StateView
							kind="empty"
							icon={Variable}
							title={t("deploy.settings.none", "{{service}} has no settings", {
								service: updated[0]?.serviceId ?? service.id,
							})}
							text={t(
								"deploy.settings.noneText",
								"The events you picked don't use any variables you can set per service.",
							)}
						/>
					</div>
				)}
			</Block>
			{updating ? <StoredOverrides {...props} /> : null}
		</>
	);
}

/** No definitions yet: say which plane will bring them (R6: never "empty"). */
function DefinitionsPending({ state, prepare, goTo }: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const { definitions, draft } = state;
	const loading = (
		<StateView
			kind="loading"
			title={t("deploy.settings.loading", "Reading the app's settings…")}
		/>
	);
	if (definitions.loading) return loading;
	if (definitions.error)
		return (
			<StateView
				kind="error"
				title={t("deploy.settings.readError", "Couldn't read the settings")}
				text={definitions.error}
			/>
		);
	if (state.mode === "offline" && draft.entry === "update")
		return (
			<StateView
				kind="notloaded"
				title={t(
					"deploy.settings.updatePending",
					"Settings load once the device is connected",
				)}
				text={t(
					"deploy.settings.updatePendingText",
					"The device describes the settings of the version its service runs. Unlock it in Where; they show here as soon as it is connected live.",
				)}
				actions={
					<DvButton size="sm" onClick={() => goTo("where")}>
						{t("deploy.settings.goToWhere", "Go to Where")}
					</DvButton>
				}
			/>
		);
	if (state.mode === "offline")
		return (
			<StateView
				kind="notloaded"
				title={t(
					"deploy.settings.offlinePending",
					"Settings show once the copy is on a device",
				)}
				text={t(
					"deploy.settings.offlinePendingText",
					"The device reads a local-only app's settings from the copy. Upload it in step 6, then set values here; until then every setting uses the app's default.",
				)}
				actions={
					<DvButton size="sm" onClick={() => goTo("copy_upload")}>
						{t("deploy.settings.goToCopy", "Go to Copy & upload")}
					</DvButton>
				}
			/>
		);
	if (prepare.state === "blocked")
		return (
			<StateView
				kind="notloaded"
				title={t(
					"deploy.settings.blocked",
					"Settings show once the version is prepared",
				)}
				text={t(
					"deploy.settings.blockedText",
					"The preparation in How it runs stopped; fix it there first.",
				)}
				actions={
					<DvButton size="sm" onClick={() => goTo("how")}>
						{t("deploy.settings.goToHow", "Go to How it runs")}
					</DvButton>
				}
			/>
		);
	return loading;
}

export function SettingsStep(props: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const { state, goTo } = props;
	const { plan, app } = state;
	const multi = plan.targets.length > 1;
	return (
		<div className="@container/settingsstep flex min-w-0 flex-col gap-4">
			<WizardStepHeader
				step={4}
				total={8}
				title={t("deploy.step.settings", "Settings")}
				lede={
					multi
						? t(
								"deploy.settings.ledeMulti",
								"Values for the app's variables. Shared by every device unless you change one for a device.",
							)
						: t(
								"deploy.settings.lede",
								"Values for the app's variables. Each setting uses the app's default unless you give it a value for this service.",
							)
				}
			/>
			{!app || !plan.services.length ? (
				<StateView
					kind="notloaded"
					title={t("deploy.settings.pickEvents", "Pick events first")}
					text={t(
						"deploy.settings.pickEventsText",
						"The settings depend on the events you deploy.",
					)}
					actions={
						<DvButton size="sm" onClick={() => goTo("what")}>
							{t("deploy.goToWhat", "Go to What")}
						</DvButton>
					}
				/>
			) : plan.app?.variables ? (
				<>
					{plan.targets.length ? null : (
						<Note>
							{t(
								"deploy.settings.pickDevices",
								"Pick devices in Where to give a device its own value.",
							)}
						</Note>
					)}
					{/* A bot's token is asked before the flows' own settings are known. */}
					{state.definitions.known ? null : <DefinitionsPending {...props} />}
					<SettingsBlock {...props} />
				</>
			) : (
				<DefinitionsPending {...props} />
			)}
		</div>
	);
}
