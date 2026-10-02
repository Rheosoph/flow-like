"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Braces,
	Diff,
	KeyRound,
	Lock,
	Pencil,
	RefreshCw,
	Undo2,
} from "lucide-react";
import { type ReactNode, useEffect, useMemo, useState } from "react";
import type { PlacementConfiguration } from "../../../../lib/device-management/deployment";
import type {
	GateNoticeKind,
	GateResult,
	LiveDeviceInput,
} from "../../../../lib/device-management/model/types";
import type { ResourceGrant } from "../../../../lib/device-resources";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../../ui/select";
import { gateCopy } from "../copy/gate-copy";
import {
	type AreaTime,
	type DevicesT,
	useAreaTime,
} from "../primitives/area-context";
import { Banner } from "../primitives/banner";
import { ConsequencePreview } from "../primitives/consequence-preview";
import { DvButton } from "../primitives/dv-button";
import { DvSheet } from "../primitives/dv-sheet";
import {
	DvInput,
	DvTextarea,
	Field,
	InputWithUnit,
} from "../primitives/form-fields";
import { InlineResult } from "../primitives/inline-result";
import { TONE_TEXT, cx } from "../primitives/tone";
import { useAppView, useAttentionState } from "../workspace";
import {
	ApplyChoices,
	type ApplyDraft,
	ConfigDiff,
	type DiffNames,
	applyDraftOf,
	applyHowError,
	exposureText,
	howOf,
	quickUpdateRows,
} from "./config-diff";
import {
	CONFIG_MAX_BYTES,
	type DraftError,
	type DraftFacts,
	type DraftResult,
	EMPTY_DRAFT,
	type FieldContext,
	type FieldLock,
	type FieldSection,
	type FieldUnit,
	type JsonError,
	type PlacementConfig,
	type SettingField,
	type SettingsDraft,
	applyDraft,
	checkApplyHow,
	exposureOf,
	hostingOf,
	jsonBytes,
	parseJsonSettings,
	settingFields,
} from "./config-model";
import {
	type Note,
	SELECT_CONTENT,
	SELECT_ITEM,
	SELECT_TRIGGER,
} from "./config-parts";
import {
	type ApplyOutcome,
	type DefinitionsRead,
	type ServiceConfigRead,
	type SettingsApply,
	type SettingsGates,
	useSettingsApply,
	useSettingsGates,
	useVariableDefinitions,
} from "./use-service-config";

/* Edit settings… (Change → Review) and Edit as JSON… (SPEC §5.3 Configuration). */

export interface SettingsEditor {
	deviceId: string;
	serviceId: string;
	read: ServiceConfigRead;
	configuration: PlacementConfiguration;
	gates: SettingsGates;
	apply: SettingsApply;
	definitions: DefinitionsRead;
	context: FieldContext;
	facts: DraftFacts;
	names: DiffNames;
	running: boolean;
	/** One sentence each; set when that way of applying can't run now. */
	safeUnavailable: string | undefined;
	quickUnavailable: string | undefined;
	/** Why no settings change can be applied at all (an update in progress is `gates.rollout`). */
	editGate: { kind: GateNoticeKind; text: string } | null;
}

/** The device's sandbox policy blocks a plain settings change; the wizard can switch the isolation. */
function editGateOf(
	t: DevicesT,
	time: AreaTime,
	gates: SettingsGates,
): SettingsEditor["editGate"] {
	const quick = gates.gates.update_service;
	if (quick.ok || gates.gates.update_with_checks.ok) return null;
	const text = gateCopy(t, quick, time).inline;
	return {
		kind: quick.kind,
		text:
			quick.gate === "G10"
				? `${text} ${t(
						"devices:serviceConfig.gate.policyHint",
						"Update… changes how the service is isolated in the deploy wizard.",
					)}`
				: text,
	};
}

/** Ids up to this length are shown whole when a variable's name isn't known. */
const SHORT_ID = 24;

function useNames(
	deviceId: string,
	definitions: DefinitionsRead,
	projectId: string | undefined,
): { names: DiffNames; hosted: (eventId: string) => boolean | undefined } {
	const { t } = useTranslation("devices");
	const { input } = useAttentionState();
	const { view } = useAppView(projectId);
	const certificates = input.live[deviceId]?.certificates?.certificates;
	return useMemo(() => {
		const events = new Map(
			view
				? [...view.events.rows, ...view.events.ineligible].map((row) => [
						row.eventId,
						row,
					])
				: [],
		);
		return {
			names: {
				variable: (id) =>
					definitions.definitions?.find((entry) => entry.id === id)?.name ||
					t("serviceConfig.variable.unnamed", "Unnamed variable {{id}}", {
						id: id.length > SHORT_ID ? `${id.slice(0, 8)}…` : id,
					}),
				event: (id) =>
					events.get(id)?.name ??
					(view
						? t("serviceConfig.event.removed", "A removed event")
						: t("serviceConfig.event.unread", "An event of this app")),
				certificate: (id) =>
					certificates?.find((entry) => entry.certificate_id === id)?.label ??
					t("serviceConfig.cert.unnamed", "Certificate {{id}}", {
						id: id.slice(0, 8),
					}),
			},
			hosted: (id) => events.get(id)?.eligibility.hosted,
		};
	}, [t, view, definitions.definitions, certificates]);
}

type LiveFacts = LiveDeviceInput | undefined;

const certificateIds = (live: LiveFacts) =>
	(live?.certificates?.certificates ?? []).map((entry) => entry.certificate_id);

/** What the device and the hub say that a changed port or instance count has to respect. */
function draftFactsOf(
	serviceId: string,
	config: PlacementConfig,
	live: LiveFacts,
	grants: readonly ResourceGrant[] | undefined,
): DraftFacts {
	const otherPorts = Object.entries(live?.placements ?? {}).flatMap(
		([id, entry]) =>
			id !== serviceId && entry.port ? [{ port: entry.port, service: id }] : [],
	);
	const approval =
		config.source === "online"
			? grants?.find(
					(grant) =>
						grant.placement_id === serviceId && grant.status === "active",
				)
			: undefined;
	return {
		otherPorts,
		port80Reserved: (live?.acme?.length ?? 0) > 0,
		...(approval ? { approvalMaxInstances: approval.max_instances } : {}),
	};
}

const gateSentence = (t: DevicesT, time: AreaTime, gate: GateResult) =>
	gate.ok ? undefined : gateCopy(t, gate, time).inline;

function safeUnavailableOf(
	t: DevicesT,
	time: AreaTime,
	gates: SettingsGates,
	config: PlacementConfig,
): string | undefined {
	const blocked = gateSentence(t, time, gates.gates.update_with_checks);
	if (blocked) return blocked;
	return config.source === "online" && !config.resource_grant
		? t(
				"devices:serviceConfig.apply.noApproval",
				"This online service has no cloud approval to check a new version with.",
			)
		: undefined;
}

/** Everything the two settings sheets need for one service; `null` until its settings were read. */
export function useSettingsEditor(
	deviceId: string,
	serviceId: string,
	read: ServiceConfigRead,
): SettingsEditor | null {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { input } = useAttentionState();
	const { configuration, service } = read;
	const gates = useSettingsGates(deviceId, serviceId, service, configuration);
	const apply = useSettingsApply(deviceId, serviceId, gates, read.deviceLabel);
	const definitions = useVariableDefinitions(
		deviceId,
		configuration,
		read.gate === null,
	);
	const { names, hosted } = useNames(deviceId, definitions, service?.projectId);
	const live = input.live[deviceId];
	const grants = input.resources[deviceId]?.grants;
	return useMemo(() => {
		if (!configuration) return null;
		const { config } = configuration;
		return {
			deviceId,
			serviceId,
			read,
			configuration,
			gates,
			apply,
			definitions,
			context: {
				...(definitions.definitions
					? { definitions: definitions.definitions }
					: {}),
				certificates: certificateIds(live),
				canAssignCertificate: gates.gates.change_tls.ok,
				multiInstance:
					config.max_replicas > 1 ||
					config.events.every((event) => hosted(event.event_id) === true),
			},
			facts: draftFactsOf(serviceId, config, live, grants),
			names,
			running: (configuration.desired_state ?? service?.desired) === "running",
			safeUnavailable: safeUnavailableOf(t, time, gates, config),
			quickUnavailable: gateSentence(t, time, gates.gates.update_service),
			editGate: editGateOf(t, time, gates),
		};
	}, [
		t,
		time,
		deviceId,
		serviceId,
		read,
		configuration,
		service?.desired,
		gates,
		apply,
		definitions,
		names,
		hosted,
		live,
		grants,
	]);
}

/* Copy of fields, sections and errors. */

const SECTION_LABEL: Record<FieldSection, (t: DevicesT) => string> = {
	variables: (t) => t("devices:serviceConfig.section.variables", "Variables"),
	instances: (t) => t("devices:serviceConfig.section.instances", "Instances"),
	endpoint: (t) => t("devices:serviceConfig.section.endpoint", "Web endpoint"),
	isolation: (t) =>
		t("devices:serviceConfig.section.isolation", "Isolation (sandbox limits)"),
	buffering: (t) =>
		t("devices:serviceConfig.section.buffering", "Write buffering"),
};

const FIELD_LABEL: Record<string, (t: DevicesT) => string> = {
	max: (t) => t("devices:serviceConfig.field.max", "Max instances"),
	port: (t) => t("devices:serviceConfig.field.port", "Port"),
	inflight: (t) =>
		t("devices:serviceConfig.field.inflight", "Max parallel requests"),
	timeout: (t) => t("devices:serviceConfig.field.timeout", "Request timeout"),
	bind: (t) => t("devices:serviceConfig.field.bind", "Who can reach it"),
	cert: (t) => t("devices:serviceConfig.field.cert", "Certificate"),
	cpu: (t) => t("devices:serviceConfig.field.cpu", "CPU cores"),
	mem: (t) => t("devices:serviceConfig.field.mem", "Memory"),
	procs: (t) => t("devices:serviceConfig.field.procs", "Processes"),
	disk: (t) => t("devices:serviceConfig.field.disk", "Disk"),
	qmib: (t) => t("devices:serviceConfig.field.qmib", "Queue budget"),
	qage: (t) => t("devices:serviceConfig.field.qage", "Max age"),
};

export function fieldLabel(
	t: DevicesT,
	names: DiffNames,
	field: Pick<SettingField, "id" | "variable">,
): string {
	return field.variable
		? names.variable(field.variable.id)
		: (FIELD_LABEL[field.id]?.(t) ?? field.id);
}

const UNIT_LABEL: Record<FieldUnit, (t: DevicesT) => string> = {
	cores: (t) => t("devices:serviceConfig.unit.cores", "cores"),
	gib: (t) => t("devices:serviceConfig.unit.gib", "GiB"),
	mib: (t) => t("devices:serviceConfig.unit.mib", "MiB"),
	days: (t) => t("devices:serviceConfig.unit.days", "days"),
	seconds: (t) => t("devices:serviceConfig.unit.seconds", "s"),
};

const LOCK_TEXT: Record<FieldLock, (t: DevicesT) => string> = {
	buffering_single: (t) =>
		t(
			"devices:serviceConfig.lock.buffering",
			"Write buffering needs exactly one instance.",
		),
	background_single: (t) =>
		t(
			"devices:serviceConfig.lock.background",
			"Only services whose events all answer web requests can run more than one instance.",
		),
	certificates: (t) =>
		t(
			"devices:serviceConfig.lock.certificates",
			"Needs Manage certificates on the whole device.",
		),
};

/** "Text", "Number", "Yes/No", "List": the value's type in plain words. */
export function typeWord(t: DevicesT, field: SettingField): string {
	const shape = field.variable?.definition?.value_type;
	if (shape === "Array" || shape === "HashSet")
		return t("devices:serviceConfig.type.list", "List");
	if (shape === "HashMap")
		return t("devices:serviceConfig.type.map", "Named values");
	const words: Record<SettingField["kind"], string> = {
		text: t("devices:serviceConfig.type.text", "Text"),
		number: t("devices:serviceConfig.type.number", "Number"),
		bool: t("devices:serviceConfig.type.bool", "Yes/No"),
		json: t("devices:serviceConfig.type.json", "JSON"),
		select: t("devices:serviceConfig.type.text", "Text"),
		secret: t("devices:serviceConfig.type.secret", "Secret"),
	};
	return words[field.kind];
}

function draftErrorText(
	t: DevicesT,
	error: DraftError,
	label: string,
	type: string,
): string {
	const params = { label, type, ...error.params };
	const unused = error.params?.issue === "unused";
	const texts: Record<DraftError["code"], string> = {
		nothing_changed: t(
			"devices:serviceConfig.error.nothingChanged",
			"Nothing changed yet. Change a value, or cancel.",
		),
		not_a_number: t(
			"devices:serviceConfig.error.notANumber",
			"{{label}} needs a number.",
			params,
		),
		not_whole: t(
			"devices:serviceConfig.error.notWhole",
			"{{label}} needs a whole number.",
			params,
		),
		out_of_range: t(
			"devices:serviceConfig.error.outOfRange",
			"{{label}} is {{value}}. Use {{min}} to {{max}}.",
			params,
		),
		invalid_value: t(
			"devices:serviceConfig.error.invalidValue",
			"{{label}} doesn't fit its type ({{type}}).",
			params,
		),
		must_resolve: unused
			? t(
					"devices:serviceConfig.error.unused",
					"{{label}} isn't used by this service's events any more. Remove it to continue.",
					params,
				)
			: t(
					"devices:serviceConfig.error.incompatible",
					"{{label}} holds a value that doesn't fit its type ({{type}}). Enter a new value or remove it.",
					params,
				),
		port_in_use: t(
			"devices:serviceConfig.error.portInUse",
			"Port {{port}} is used by {{service}} on this device. Pick another port.",
			params,
		),
		port_reserved: t(
			"devices:serviceConfig.error.portReserved",
			"Port 80 is used by Let's Encrypt on this device. Pick another port.",
		),
		instances_over_approval: t(
			"devices:serviceConfig.error.instancesOverApproval",
			"Its cloud approval covers {{max}} instances. Replace the approval on Cloud access first.",
			params,
		),
		too_large: t(
			"devices:serviceConfig.error.tooLarge",
			"The settings are {{bytes, number}} bytes. The device accepts up to {{max, number}}. Remove something and try again.",
			params,
		),
	};
	return texts[error.code];
}

function jsonErrorText(t: DevicesT, error: JsonError): string {
	const texts: Record<JsonError["code"], string> = {
		invalid_json: t(
			"devices:serviceConfig.json.invalid",
			"That isn't valid JSON: {{detail}}. Nothing was applied.",
			error.params,
		),
		not_an_object: t(
			"devices:serviceConfig.json.notObject",
			"The settings must be one JSON object. Nothing was applied.",
		),
		identity_changed: t(
			"devices:serviceConfig.json.identity",
			"The service's identity can't change (it now says {{value}}). Deploy a new service instead.",
			error.params,
		),
		nothing_changed: t(
			"devices:serviceConfig.json.nothingChanged",
			"Nothing changed yet.",
		),
		too_large: t(
			"devices:serviceConfig.error.tooLarge",
			"The settings are {{bytes, number}} bytes. The device accepts up to {{max, number}}. Remove something and try again.",
			error.params,
		),
	};
	return texts[error.code];
}

/** What an apply that didn't finish says, next to the control that started it. */
export function outcomeNote(
	t: DevicesT,
	time: AreaTime,
	outcome: ApplyOutcome,
	names: { service: string; device: string },
): Note | null {
	switch (outcome.status) {
		case "done":
		case "busy":
		case "cancelled":
			return null;
		case "gated":
			return { tone: "warning", text: gateCopy(t, outcome.gate, time).inline };
		case "unknown":
			return {
				tone: "unknown",
				text: t(
					"devices:serviceConfig.outcome.unknown",
					"No reply was received. The change may or may not have reached {{device}}; Activity lets you check.",
					names,
				),
			};
		case "stale":
		case "review":
			return {
				tone: "warning",
				text: t(
					"devices:serviceConfig.outcome.stale",
					"The settings changed on {{device}} since you opened them. Reload them and review your change again.",
					names,
				),
			};
		case "rolled_back":
			return {
				tone: "warning",
				text: t(
					"devices:serviceConfig.outcome.rolledBack",
					"The new settings weren't healthy in time. {{device}} restored the settings that ran before.",
					names,
				),
			};
		case "update_cancelled":
			return {
				tone: "warning",
				text: t(
					"devices:serviceConfig.outcome.cancelled",
					"The update was discarded before it finished. The earlier settings keep running.",
				),
			};
		case "update_failed":
			return {
				tone: "critical",
				text: t(
					"devices:serviceConfig.outcome.updateFailed",
					"The update failed and {{service}} may be stopped. Status shows what the device reports.",
					names,
				),
			};
		case "queue_waiting":
			return {
				tone: "warning",
				text: t("devices:serviceConfig.outcome.queueWaiting", {
					count: outcome.pending,
					defaultValue_one:
						"{{count, number}} buffered change still waits. Write buffering can be turned off or narrowed once the queue is empty; try it again or discard it on Write buffering first.",
					defaultValue_other:
						"{{count, number}} buffered changes still wait. Write buffering can be turned off or narrowed once the queue is empty; try them again or discard them on Write buffering first.",
				}),
			};
		case "too_large":
			return {
				tone: "critical",
				text: t(
					"devices:serviceConfig.error.tooLarge",
					"The settings are {{bytes, number}} bytes. The device accepts up to {{max, number}}. Remove something and try again.",
					{ bytes: outcome.bytes, max: CONFIG_MAX_BYTES },
				),
			};
		case "refused":
			return {
				tone: "critical",
				text: outcome.reason
					? t(
							"devices:serviceConfig.outcome.refused",
							"The device refused the change: “{{reason}}”",
							{ reason: outcome.reason },
						)
					: t(
							"devices:serviceConfig.outcome.refusedPlain",
							"The device refused the change. Nothing was applied.",
						),
			};
		default:
			return {
				tone: "critical",
				text: t(
					"devices:serviceConfig.outcome.failed",
					"The device didn't confirm the change. Reload the settings and check Status before trying again.",
				),
			};
	}
}

/* Running an apply from a sheet. */

interface ApplyRun {
	draft: ApplyDraft;
	setDraft(draft: ApplyDraft): void;
	error: string | null;
	note: Note | null;
	stale: boolean;
	busy: boolean;
	reset(): void;
	run(base: PlacementConfiguration, config: PlacementConfig): Promise<void>;
}

function doneNote(
	t: DevicesT,
	editor: SettingsEditor,
	safe: boolean,
	revision: number,
	/** The clock time the device finished, not the time of the click. */
	finishedAt: string,
): Note {
	const params = {
		service: editor.serviceId,
		version: revision,
		time: finishedAt,
	};
	if (safe)
		return {
			tone: "good",
			text: t(
				"devices:serviceConfig.outcome.safeDone",
				"{{service}} runs settings v{{version}}. Updated at {{time}}.",
				params,
			),
		};
	return {
		tone: "good",
		text: editor.running
			? t(
					"devices:serviceConfig.outcome.quickDone",
					"{{service}} restarted with settings v{{version}} at {{time}}.",
					params,
				)
			: t(
					"devices:serviceConfig.outcome.storedDone",
					"Settings v{{version}} were stored at {{time}}. {{service}} stays stopped and uses them at its next start.",
					params,
				),
	};
}

/**
 * Applies from a sheet: a safe update closes the sheet once the device took
 * it and reports the end on the page; everything that fails before that stays
 * in the sheet, with the person's input kept.
 */
function useApplyRun(
	editor: SettingsEditor,
	onClose: () => void,
	onNote: (note: Note | null) => void,
): ApplyRun {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { workspace } = useAttentionState();
	const [draft, setDraft] = useState<ApplyDraft>(() =>
		applyDraftOf(editor.safeUnavailable ? "quick" : "safe"),
	);
	const [error, setError] = useState<string | null>(null);
	const [note, setNote] = useState<Note | null>(null);
	const [stale, setStale] = useState(false);
	const names = { service: editor.serviceId, device: editor.read.deviceLabel };
	// A safe update ends minutes after the click: its sentences read the clock when they are written.
	const clockNow = () => time.clock(workspace.clock.now() / 1000);
	const run = async (base: PlacementConfiguration, config: PlacementConfig) => {
		const how = howOf(draft);
		const invalid = checkApplyHow(how);
		if (invalid) {
			setError(applyHowError(t, invalid, draft));
			return;
		}
		setError(null);
		setNote(null);
		const safe = how.mode === "safe";
		let started = false;
		const outcome = await editor.apply.apply({
			existing: base,
			config,
			how,
			onStarted: () => {
				started = true;
				onClose();
				onNote({
					tone: "info",
					text: t(
						"serviceConfig.outcome.safeStarted",
						"Safe update to settings v{{version}} started at {{time}}. Follow it on Status.",
						{
							version: base.config_revision + 1,
							time: clockNow(),
						},
					),
				});
			},
		});
		if (outcome.status === "done") {
			if (!started) onClose();
			onNote(doneNote(t, editor, safe, outcome.revision, clockNow()));
			return;
		}
		const failed = outcomeNote(t, time, outcome, names);
		if (started) {
			onNote(failed);
			return;
		}
		if (outcome.status === "stale" || outcome.status === "review")
			setStale(true);
		setNote(failed);
	};
	return {
		draft,
		setDraft,
		error,
		note,
		stale,
		busy: editor.apply.pending,
		reset: () => {
			setDraft(applyDraftOf(editor.safeUnavailable ? "quick" : "safe"));
			setError(null);
			setNote(null);
			setStale(false);
		},
		run,
	};
}

/**
 * A sheet stays while its apply is on the way: what the device answers is said
 * in the sheet, so Esc, the close button and a click outside wait for it like
 * Cancel does.
 */
const whileIdle =
	(run: ApplyRun, onOpenChange: (open: boolean) => void) => (open: boolean) => {
		if (open || !run.busy) onOpenChange(open);
	};

function ApplySection({
	editor,
	run,
	config,
}: Readonly<{
	editor: SettingsEditor;
	run: ApplyRun;
	/** The settings about to be applied, for the address in the consequence rows. */
	config: PlacementConfig;
}>) {
	const { t } = useTranslation("devices");
	const hosting = hostingOf(config);
	return (
		<>
			<ApplyChoices
				draft={run.draft}
				onChange={run.setDraft}
				safeUnavailable={editor.safeUnavailable}
				quickUnavailable={editor.quickUnavailable}
				singleInstance={config.max_replicas <= 1}
				running={editor.running}
				error={run.error}
			/>
			{run.draft.mode === "quick" && !editor.quickUnavailable ? (
				<ConsequencePreview
					compact
					rows={quickUpdateRows(t, {
						running: editor.running,
						...(hosting ? { address: `${hosting.host}:${hosting.port}` } : {}),
						...(editor.safeUnavailable
							? { safeUnavailable: editor.safeUnavailable }
							: {}),
					})}
				/>
			) : null}
			{run.note ? (
				<InlineResult tone={run.note.tone}>{run.note.text}</InlineResult>
			) : null}
		</>
	);
}

function StaleBanner({
	editor,
	base,
	keepsInput,
	onReload,
}: Readonly<{
	editor: SettingsEditor;
	base: PlacementConfiguration;
	/** Reloading keeps what the person typed (fields) or replaces it (the JSON text). */
	keepsInput: boolean;
	onReload(): void;
}>) {
	const { t } = useTranslation("devices");
	const current = editor.configuration.config_revision;
	const version =
		current !== base.config_revision
			? t(
					"serviceConfig.stale.version",
					"Settings v{{current}} replaced v{{base}}.",
					{ current, base: base.config_revision },
				)
			: t(
					"serviceConfig.stale.versionUnknown",
					"Someone else changed or updated this service.",
				);
	return (
		<Banner
			tone="warning"
			title={t(
				"serviceConfig.stale.title",
				"The settings changed on {{device}} while this was open.",
				{ device: editor.read.deviceLabel },
			)}
			actions={
				<DvButton
					size="sm"
					icon={RefreshCw}
					data-act="settings-reload"
					onClick={onReload}
				>
					{t("serviceConfig.stale.reload", "Reload settings")}
				</DvButton>
			}
		>
			{version}{" "}
			{keepsInput
				? t(
						"serviceConfig.stale.keeps",
						"Reload them; what you typed is kept so you can review it against the new settings.",
					)
				: t(
						"serviceConfig.stale.replaces",
						"Reloading replaces the text below with what the device holds now, so copy your changes first.",
					)}
		</Banner>
	);
}

/* Edit settings: fields. */

/** Radix refuses an empty item value. */
const NONE = "__none__";

/** A locked field reads as fixed, not as an empty input. */
const LOCKED_LOOK = "disabled:bg-surface-sunken";

function optionLabel(
	t: DevicesT,
	names: DiffNames,
	field: SettingField,
	value: string,
): string {
	if (field.id === "cert")
		return value
			? names.certificate(value)
			: t("devices:serviceConfig.cert.none", "None · plain HTTP");
	if (field.kind === "bool")
		return value === "true"
			? t("devices:serviceConfig.bool.yes", "Yes")
			: t("devices:serviceConfig.bool.no", "No");
	return t("devices:serviceConfig.bind.option", "{{exposure}} ({{host}})", {
		exposure: exposureText(t, exposureOf(value)),
		host: value,
	});
}

/** What `Field` hands its control: the ids of its hint and error, and whether it is invalid. */
interface Described {
	"aria-describedby"?: string;
	"aria-invalid"?: boolean;
}

interface ControlProps extends Described {
	field: SettingField;
	id: string;
	value: string;
	names: DiffNames;
	onChange(value: string): void;
}

const described = (props: Described): Described => ({
	"aria-describedby": props["aria-describedby"],
	"aria-invalid": props["aria-invalid"],
});

function SelectControl({
	field,
	id,
	value,
	names,
	onChange,
	...rest
}: ControlProps) {
	const { t } = useTranslation("devices");
	const options =
		field.kind === "bool" ? ["true", "false"] : (field.options ?? []);
	return (
		<Select
			value={value === "" ? NONE : value}
			onValueChange={(next) => onChange(next === NONE ? "" : next)}
			disabled={!!field.lock}
		>
			<SelectTrigger id={id} className={SELECT_TRIGGER} {...described(rest)}>
				<SelectValue
					placeholder={t("serviceConfig.field.appDefault", "App default")}
				/>
			</SelectTrigger>
			<SelectContent className={SELECT_CONTENT}>
				{options.map((option) => (
					<SelectItem
						key={option || NONE}
						value={option || NONE}
						className={SELECT_ITEM}
					>
						{optionLabel(t, names, field, option)}
					</SelectItem>
				))}
			</SelectContent>
		</Select>
	);
}

function TextControl({ field, id, value, onChange, ...rest }: ControlProps) {
	const { t } = useTranslation("devices");
	const common = {
		id,
		value,
		disabled: !!field.lock,
		spellCheck: false,
		autoComplete: "off",
		placeholder: field.variable?.stored
			? undefined
			: t("serviceConfig.field.appDefault", "App default"),
		...described(rest),
	};
	if (field.kind === "json")
		return (
			<DvTextarea
				{...common}
				rows={3}
				className={cx("font-mono", LOCKED_LOOK)}
				onChange={(event) => onChange(event.target.value)}
			/>
		);
	const numeric = field.kind === "number";
	const props = {
		...common,
		mono: !numeric,
		numeric,
		inputMode: numeric ? ("decimal" as const) : undefined,
		onChange: (event: { target: { value: string } }) =>
			onChange(event.target.value),
	};
	return field.unit ? (
		<InputWithUnit {...props} unit={UNIT_LABEL[field.unit](t)} />
	) : (
		<DvInput {...props} className={LOCKED_LOOK} />
	);
}

/** Why a stored value needs attention, next to its field. */
function issueText(t: DevicesT, field: SettingField): string | null {
	const issue = field.variable?.issue;
	if (!issue) return null;
	if (issue === "unused")
		return t(
			"devices:serviceConfig.issue.unused",
			"No event of this service uses it any more. Remove it.",
		);
	return field.kind === "secret"
		? t(
				"devices:serviceConfig.issue.secretKind",
				"The app no longer treats it as a secret. Remove it, then set the value as a plain variable.",
			)
		: t(
				"devices:serviceConfig.issue.incompatible",
				"The stored value doesn't fit its type ({{type}}). Enter a new value or remove it.",
				{ type: typeWord(t, field) },
			);
}

function VariableNotes({
	field,
	removed,
	onRemove,
}: Readonly<{ field: SettingField; removed: boolean; onRemove(): void }>) {
	const { t } = useTranslation("devices");
	const variable = field.variable;
	if (!variable?.stored) return null;
	const issue = issueText(t, field);
	return (
		<div className="flex flex-wrap items-center gap-x-2 gap-y-1">
			{issue ? (
				<span
					data-issue={variable.issue}
					className={cx("text-xs", TONE_TEXT.warning)}
				>
					{issue}
				</span>
			) : null}
			<DvButton
				size="xs"
				variant="ghost"
				icon={removed ? Undo2 : undefined}
				data-act="variable-remove"
				onClick={onRemove}
			>
				{removed
					? t("serviceConfig.field.keep", "Keep the stored value")
					: field.kind === "secret"
						? t("serviceConfig.field.removeSecret", "Remove stored secret")
						: t("serviceConfig.field.useDefault", "Use the app's default")}
			</DvButton>
		</div>
	);
}

function FieldRow({
	field,
	names,
	draft,
	error,
	onChange,
	onRemove,
}: Readonly<{
	field: SettingField;
	names: DiffNames;
	draft: SettingsDraft;
	error?: string;
	onChange(value: string): void;
	onRemove(): void;
}>) {
	const { t } = useTranslation("devices");
	const label = fieldLabel(t, names, field);
	const removed = !!field.variable && draft.removed.includes(field.variable.id);
	const wide =
		field.id === "cert" ||
		(field.section === "variables" && field.kind !== "number");
	const fixed = removed
		? t(
				"serviceConfig.field.removed",
				"The stored value is removed; the app's default applies.",
			)
		: field.kind === "secret"
			? t(
					"serviceConfig.summary.storedSecret",
					"stored secret · can't be read back",
				)
			: null;
	return (
		<div
			data-field-id={field.id}
			className={cx(
				"flex min-w-0 flex-col gap-1",
				wide && "col-span-2 max-[560px]:col-span-full",
			)}
		>
			{fixed ? (
				<div className="flex flex-col gap-1.5">
					<span className="text-[13px]/[18px] font-medium">{label}</span>
					<span
						data-removed={removed ? "" : undefined}
						className="text-ui text-muted-foreground"
					>
						{fixed}
					</span>
				</div>
			) : (
				<EditableField
					field={field}
					label={label}
					value={draft.values[field.id] ?? field.value}
					names={names}
					error={error}
					onChange={onChange}
				/>
			)}
			<VariableNotes field={field} removed={removed} onRemove={onRemove} />
		</div>
	);
}

function EditableField({
	field,
	label,
	value,
	names,
	error,
	onChange,
}: Readonly<{
	field: SettingField;
	label: string;
	value: string;
	names: DiffNames;
	error?: string;
	onChange(value: string): void;
}>) {
	const { t } = useTranslation("devices");
	const id = `svc-edit-${field.id.replace(/[^A-Za-z0-9_-]/g, "-")}`;
	const control: ControlProps = { field, id, value, names, onChange };
	const picks = field.kind === "select" || field.kind === "bool";
	return (
		<Field
			id={id}
			label={
				field.variable ? (
					<TypedLabel label={label} type={typeWord(t, field)} />
				) : (
					label
				)
			}
			error={error}
			hint={field.lock ? <LockHint lock={field.lock} /> : undefined}
		>
			{picks ? <SelectControl {...control} /> : <TextControl {...control} />}
		</Field>
	);
}

/** A variable's name with its type in plain words after it. */
function TypedLabel({
	label,
	type,
}: Readonly<{ label: string; type: string }>) {
	return (
		<>
			{label} <span className="font-normal text-muted-foreground">{type}</span>
		</>
	);
}

/** Why a field can't be changed here (R7: the reason is always visible). */
function LockHint({ lock }: Readonly<{ lock: FieldLock }>) {
	const { t } = useTranslation("devices");
	return (
		<span className="inline-flex items-start gap-1">
			<Lock aria-hidden className="mt-0.5 size-3 shrink-0" />
			{LOCK_TEXT[lock](t)}
		</span>
	);
}

interface Section {
	id: FieldSection;
	fields: SettingField[];
}

function sectionsOf(fields: readonly SettingField[]): Section[] {
	const sections: Section[] = [];
	for (const field of fields) {
		const known = sections.find((section) => section.id === field.section);
		if (known) known.fields.push(field);
		else sections.push({ id: field.section, fields: [field] });
	}
	return sections;
}

function ChangeStep({
	editor,
	fields,
	draft,
	result,
	showErrors,
	onDraft,
	wizardNote,
}: Readonly<{
	editor: SettingsEditor;
	fields: readonly SettingField[];
	draft: SettingsDraft;
	result: DraftResult;
	showErrors: boolean;
	onDraft(draft: SettingsDraft): void;
	wizardNote?: ReactNode;
}>) {
	const { t } = useTranslation("devices");
	const { locale } = useAreaTime();
	const { names } = editor;
	const errorOf = (field: SettingField) => {
		const error = result.errors.find((entry) => entry.field === field.id);
		return showErrors && error
			? draftErrorText(
					t,
					error,
					fieldLabel(t, names, field),
					typeWord(t, field),
				)
			: undefined;
	};
	const general = result.errors.filter((error) => !error.field);
	const secrets = fields
		.filter((field) => field.kind === "secret")
		.map((field) => fieldLabel(t, names, field));
	const toggleRemoved = (id: string) =>
		onDraft({
			...draft,
			removed: draft.removed.includes(id)
				? draft.removed.filter((entry) => entry !== id)
				: [...draft.removed, id],
		});
	return (
		<>
			<div className="flex flex-col">
				{sectionsOf(fields).map((section) => (
					<section
						key={section.id}
						data-section={section.id}
						className="flex flex-col gap-3 border-t border-hairline py-3.5 first:border-t-0 first:pt-0 last:pb-0"
					>
						<h3 className="text-[13px]/[18px] font-semibold">
							{SECTION_LABEL[section.id](t)}
						</h3>
						<div className="grid grid-cols-[repeat(auto-fill,minmax(170px,1fr))] gap-x-4 gap-y-3">
							{section.fields.map((field) => (
								<FieldRow
									key={field.id}
									field={field}
									names={names}
									draft={draft}
									error={errorOf(field)}
									onChange={(value) =>
										onDraft({
											...draft,
											values: { ...draft.values, [field.id]: value },
										})
									}
									onRemove={() => toggleRemoved(field.variable?.id ?? "")}
								/>
							))}
						</div>
					</section>
				))}
			</div>
			{editor.definitions.definitions === undefined &&
			!editor.definitions.loading &&
			fields.some((field) => field.section === "variables") ? (
				<p className="text-xs text-muted-foreground">
					{t(
						"serviceConfig.field.noDefinitions",
						"The names and types of this version's variables couldn't be read, so stored values are edited by their current type.",
					)}
				</p>
			) : null}
			{secrets.length ? (
				<p className="flex items-start gap-1.5 text-xs text-muted-foreground">
					<KeyRound aria-hidden className="mt-0.5 size-3 shrink-0" />
					<span>
						{t(
							"serviceConfig.edit.secretsNote",
							"Secret values aren't edited here. To change {{names}}, use Change secret value on the Configuration tab; it needs no update.",
							{ names: new Intl.ListFormat(locale).format(secrets) },
						)}
					</span>
				</p>
			) : null}
			{wizardNote}
			{showErrors && general.length ? (
				<div role="alert" data-edit-errors="" className="flex flex-col gap-1">
					{general.map((error) => (
						<p key={error.code} className={cx("text-xs", TONE_TEXT.critical)}>
							{draftErrorText(t, error, "", "")}
						</p>
					))}
				</div>
			) : null}
		</>
	);
}

function ExposureWarnings({
	base,
	next,
}: Readonly<{ base: PlacementConfig; next: PlacementConfig }>) {
	const { t } = useTranslation("devices");
	const before = hostingOf(base);
	const after = hostingOf(next);
	if (!after || after.exposure === "loopback" || next.tls_certificate_id)
		return null;
	const lost = !!base.tls_certificate_id;
	const opened = before?.exposure === "loopback";
	if (!lost && !opened) return null;
	return (
		<Banner
			tone="warning"
			title={
				lost
					? t(
							"serviceConfig.review.lostCert",
							"The service page loses its certificate.",
						)
					: t(
							"serviceConfig.review.exposed",
							"The service page becomes reachable from every network the device is on.",
						)
			}
		>
			{lost
				? t(
						"serviceConfig.review.lostCertText",
						"It becomes unencrypted (HTTP) on every network the device is on. Browsers warn people who open it.",
					)
				: t(
						"serviceConfig.review.exposedText",
						"Without a certificate it's unencrypted. Go back and pick one under Certificate, or set one up on the device's Certificates tab first.",
					)}
		</Banner>
	);
}

/** "Edit settings…": change values, review the diff, apply with a safe or quick update. Keeps the installed app version. */
export function EditSettingsSheet({
	editor,
	open,
	onOpenChange,
	onNote,
	focus,
	wizardNote,
}: Readonly<{
	editor: SettingsEditor;
	open: boolean;
	onOpenChange(open: boolean): void;
	/** The page shows what an apply did once the sheet is closed. */
	onNote(note: Note | null): void;
	/** Field to focus when the sheet opens ("cert" from the Endpoint tab). */
	focus?: string;
	wizardNote?: ReactNode;
}>) {
	const { t } = useTranslation("devices");
	const [base, setBase] = useState(editor.configuration);
	const [draft, setDraft] = useState<SettingsDraft>(EMPTY_DRAFT);
	const [step, setStep] = useState<"change" | "review">("change");
	const [showErrors, setShowErrors] = useState(false);
	const [wasOpen, setWasOpen] = useState(open);
	const run = useApplyRun(editor, () => onOpenChange(false), onNote);
	if (open !== wasOpen) {
		setWasOpen(open);
		if (open) {
			setBase(editor.configuration);
			setDraft(EMPTY_DRAFT);
			setStep("change");
			setShowErrors(false);
			run.reset();
		}
	}
	useEffect(() => {
		if (!open || !focus) return;
		const timer = setTimeout(
			() => document.getElementById(`svc-edit-${focus}`)?.focus(),
			60,
		);
		return () => clearTimeout(timer);
	}, [open, focus]);
	const fields = useMemo(
		() => settingFields(base.config, editor.context),
		[base, editor.context],
	);
	const result = useMemo(
		() => applyDraft(base.config, fields, draft, editor.facts),
		[base, fields, draft, editor.facts],
	);
	const stale =
		run.stale || editor.configuration.config_revision !== base.config_revision;
	const reload = async () => {
		const fresh = await editor.read.refresh();
		setBase(fresh ?? editor.configuration);
		setStep("change");
		setShowErrors(false);
		run.reset();
	};
	const review = () => {
		setShowErrors(true);
		if (!result.errors.length && !stale) setStep("review");
	};
	const reviewing = step === "review";
	return (
		<DvSheet
			open={open}
			onOpenChange={whileIdle(run, onOpenChange)}
			wide
			icon={reviewing ? Diff : Pencil}
			title={
				reviewing
					? t("serviceConfig.review.title", "Review changes to {{service}}", {
							service: editor.serviceId,
						})
					: t("serviceConfig.edit.title", "Edit settings of {{service}}", {
							service: editor.serviceId,
						})
			}
			sub={
				reviewing
					? t("serviceConfig.review.subtitle", "settings v{{from}} → v{{to}}", {
							from: base.config_revision,
							to: base.config_revision + 1,
						})
					: t(
							"serviceConfig.edit.subtitle",
							"Settings v{{version}} · {{device}} · keeps the installed app version",
							{
								version: base.config_revision,
								device: editor.read.deviceLabel,
							},
						)
			}
			onBack={reviewing ? () => setStep("change") : undefined}
			footNote={
				reviewing
					? t("serviceConfig.review.step", "Step 2 of 2 · Apply")
					: t("serviceConfig.edit.step", "Step 1 of 2 · Change")
			}
			foot={
				<>
					<DvButton
						onClick={() =>
							reviewing ? setStep("change") : onOpenChange(false)
						}
						disabled={run.busy}
					>
						{reviewing
							? t("serviceConfig.sheet.back", "Back")
							: t("serviceConfig.sheet.cancel", "Cancel")}
					</DvButton>
					{reviewing ? (
						<DvButton
							variant="primary"
							busy={run.busy}
							aria-disabled={stale || undefined}
							data-act="settings-apply"
							onClick={() => void run.run(base, result.config)}
						>
							{t("serviceConfig.sheet.apply", "Apply")}
						</DvButton>
					) : (
						<DvButton
							variant="primary"
							aria-disabled={stale || undefined}
							data-act="settings-review"
							onClick={review}
						>
							{t("serviceConfig.edit.review", "Review changes")}
						</DvButton>
					)}
				</>
			}
		>
			{stale ? (
				<StaleBanner
					editor={editor}
					base={base}
					keepsInput
					onReload={() => void reload()}
				/>
			) : null}
			{reviewing ? (
				<>
					<ConfigDiff
						before={base.config}
						after={result.config}
						names={editor.names}
						label={t(
							"serviceConfig.review.label",
							"Changes to settings v{{version}}",
							{ version: base.config_revision },
						)}
					/>
					<ExposureWarnings base={base.config} next={result.config} />
					<ApplySection editor={editor} run={run} config={result.config} />
				</>
			) : (
				<ChangeStep
					editor={editor}
					fields={fields}
					draft={draft}
					result={result}
					showErrors={showErrors}
					onDraft={setDraft}
					wizardNote={wizardNote}
				/>
			)}
		</DvSheet>
	);
}

/* Edit as JSON. */

function JsonCounter({ text }: Readonly<{ text: string }>) {
	const { t } = useTranslation("devices");
	let bytes: number;
	try {
		bytes = jsonBytes(JSON.parse(text));
	} catch {
		bytes = new TextEncoder().encode(text).length;
	}
	return (
		<span
			data-json-bytes={bytes}
			className={cx(
				"tabular-nums",
				bytes > CONFIG_MAX_BYTES ? TONE_TEXT.critical : undefined,
			)}
		>
			{t(
				"serviceConfig.json.bytes",
				"{{bytes, number}} of {{max, number}} bytes, as sent to the device",
				{ bytes, max: CONFIG_MAX_BYTES },
			)}
		</span>
	);
}

/** "Edit as JSON…": the whole settings object, preloaded; secrets appear as references, never values. */
export function EditJsonSheet({
	editor,
	open,
	onOpenChange,
	onNote,
}: Readonly<{
	editor: SettingsEditor;
	open: boolean;
	onOpenChange(open: boolean): void;
	onNote(note: Note | null): void;
}>) {
	const { t } = useTranslation("devices");
	const [base, setBase] = useState(editor.configuration);
	const [text, setText] = useState("");
	const [error, setError] = useState<string | null>(null);
	const [wasOpen, setWasOpen] = useState(open);
	const run = useApplyRun(editor, () => onOpenChange(false), onNote);
	if (open !== wasOpen) {
		setWasOpen(open);
		if (open) {
			setBase(editor.configuration);
			setText(JSON.stringify(editor.configuration.config, null, 2));
			setError(null);
			run.reset();
		}
	}
	const parsed = useMemo(
		() => parseJsonSettings(base.config, text),
		[base, text],
	);
	const stale =
		run.stale || editor.configuration.config_revision !== base.config_revision;
	const reload = async () => {
		const fresh = (await editor.read.refresh()) ?? editor.configuration;
		setBase(fresh);
		setText(JSON.stringify(fresh.config, null, 2));
		setError(null);
		run.reset();
	};
	const apply = () => {
		if ("error" in parsed) {
			setError(jsonErrorText(t, parsed.error));
			return;
		}
		setError(null);
		void run.run(base, parsed.config);
	};
	return (
		<DvSheet
			open={open}
			onOpenChange={whileIdle(run, onOpenChange)}
			wide
			icon={Braces}
			title={t("serviceConfig.json.title", "Edit {{service}} as JSON", {
				service: editor.serviceId,
			})}
			sub={t(
				"serviceConfig.json.subtitle",
				"Advanced · settings v{{version}} · secrets appear as references, never values",
				{ version: base.config_revision },
			)}
			foot={
				<>
					<DvButton onClick={() => onOpenChange(false)} disabled={run.busy}>
						{t("serviceConfig.sheet.cancel", "Cancel")}
					</DvButton>
					<DvButton
						variant="primary"
						busy={run.busy}
						aria-disabled={stale || undefined}
						data-act="json-apply"
						onClick={apply}
					>
						{t("serviceConfig.sheet.apply", "Apply")}
					</DvButton>
				</>
			}
		>
			{stale ? (
				<StaleBanner
					editor={editor}
					base={base}
					keepsInput={false}
					onReload={() => void reload()}
				/>
			) : null}
			<Field
				id="svc-json-text"
				label={t("serviceConfig.json.label", "Settings")}
				error={error ?? undefined}
				hint={<JsonCounter text={text} />}
			>
				<DvTextarea
					value={text}
					spellCheck={false}
					autoComplete="off"
					rows={16}
					className="max-h-[46vh] min-h-56 font-mono text-xs"
					onChange={(event) => {
						setText(event.target.value);
						setError(null);
					}}
				/>
			</Field>
			{"config" in parsed ? (
				<ConfigDiff
					before={base.config}
					after={parsed.config}
					names={editor.names}
					label={t(
						"serviceConfig.review.label",
						"Changes to settings v{{version}}",
						{ version: base.config_revision },
					)}
				/>
			) : null}
			<ApplySection
				editor={editor}
				run={run}
				config={"config" in parsed ? parsed.config : base.config}
			/>
			<p className="text-xs text-muted-foreground">
				{t(
					"serviceConfig.json.stateNote",
					"It never applies a stopped state on its own: requested Running or Stopped stays as it is.",
				)}
			</p>
		</DvSheet>
	);
}
