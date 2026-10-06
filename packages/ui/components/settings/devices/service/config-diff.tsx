"use client";

import { useTranslation } from "@flow-like/locales";
import { ShieldCheck, Zap } from "lucide-react";
import type { ReactNode } from "react";
import {
	type ConfigDiffRow,
	type DiffField,
	diffPlacementConfig,
} from "../../../../lib/device-management/model/deploy-plan";
import { humanFileSize } from "../../../../lib/utils";
import type { DevicesT } from "../primitives/area-context";
import type { ConsequenceRows } from "../primitives/consequence-preview";
import { type DiffRow, DiffRows } from "../primitives/diff-rows";
import {
	ChoiceCards,
	type ChoiceOption,
	Field,
	InputWithUnit,
} from "../primitives/form-fields";
import { TONE_TEXT, cx } from "../primitives/tone";
import {
	type ApplyHow,
	type ApplyHowError,
	type ApplyMode,
	type Exposure,
	type ExtraDiffField,
	type ExtraDiffRow,
	type PlacementConfig,
	exposureOf,
	extraDiffRows,
} from "./config-model";
import { Mono } from "./config-parts";

/* The one settings diff (W1-PLAN `diffPlacementConfig` → W1-UI2 `DiffRows`) and how a change is applied. */

const record = (value: unknown): Record<string, unknown> =>
	value && typeof value === "object" && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: {};

const listOf = (value: unknown): unknown[] =>
	Array.isArray(value) ? value : [];

const short = (value: unknown) => String(value ?? "").slice(0, 8);

const trimmed = (value: number) => String(Number(value.toFixed(2)));

/** A numeric setting; an absent one reads as 0. */
const amount = (value: unknown) => Number(value ?? 0);

export function exposureText(t: DevicesT, exposure: Exposure): string {
	const texts: Record<Exposure, string> = {
		loopback: t("devices:serviceConfig.exposure.loopback", "Only this device"),
		all: t("devices:serviceConfig.exposure.all", "All networks"),
		one: t("devices:serviceConfig.exposure.one", "One address"),
	};
	return texts[exposure];
}

/** "Sandboxed · 2 cores · 2 GiB · 256 processes · 4 GiB disk". */
export function isolationText(t: DevicesT, resources: unknown): string {
	const limits = record(resources);
	if (limits.profile !== "linux_sandbox")
		return t(
			"devices:serviceConfig.isolation.trusted",
			"Not sandboxed · runs with the agent's access to the device",
		);
	return t(
		"devices:serviceConfig.isolation.sandboxed",
		"Sandboxed · {{cores}} cores · {{memory}} · {{processes, number}} processes · {{disk}} disk",
		{
			cores: trimmed(amount(limits.cpu_millis) / 1000),
			memory: humanFileSize(amount(limits.memory_bytes)),
			processes: amount(limits.max_processes),
			disk: humanFileSize(amount(limits.disk_bytes)),
		},
	);
}

/** How long buffered changes are kept: "7 days", "12 hours". */
export function ageText(t: DevicesT, seconds: number): string {
	if (seconds >= 86_400)
		return t("devices:serviceConfig.buffering.days", {
			count: Number(trimmed(seconds / 86_400)),
			defaultValue_one: "{{count, number}} day",
			defaultValue_other: "{{count, number}} days",
		});
	return t("devices:serviceConfig.buffering.hours", {
		count: Math.max(1, Math.round(seconds / 3_600)),
		defaultValue_one: "{{count, number}} hour",
		defaultValue_other: "{{count, number}} hours",
	});
}

/** "queue 256 MiB · table copy 2.0 GiB · 10,000 queued changes · max age 7 days". */
export function bufferingBudgets(t: DevicesT, writes: unknown): string {
	const value = record(writes);
	return t(
		"devices:serviceConfig.buffering.budgets",
		"queue {{queue}} · table copy {{mirror}} · {{changes, number}} queued changes · max age {{age}}",
		{
			queue: humanFileSize(amount(value.max_queue_bytes), false, 0),
			mirror: humanFileSize(amount(value.max_mirror_bytes)),
			changes: amount(value.max_operations),
			age: ageText(t, amount(value.max_age_seconds)),
		},
	);
}

export function restartText(t: DevicesT, restart: unknown): string {
	const policy = record(restart);
	return t(
		"devices:serviceConfig.restart.text",
		"first retry after {{initial, number}} s, then up to {{max, number}} s apart · up to {{restarts, number}} restarts",
		{
			initial: amount(policy.initial_backoff_secs),
			max: amount(policy.max_backoff_secs),
			restarts: amount(policy.max_restarts),
		},
	);
}

export function pinsText(
	t: DevicesT,
	models: number,
	packages: number,
): string {
	const parts = [
		models
			? t("devices:serviceConfig.pins.models", {
					count: models,
					defaultValue_one: "{{count, number}} model",
					defaultValue_other: "{{count, number}} models",
				})
			: null,
		packages
			? t("devices:serviceConfig.pins.packages", {
					count: packages,
					defaultValue_one: "{{count, number}} package",
					defaultValue_other: "{{count, number}} packages",
				})
			: null,
	].filter(Boolean);
	return parts.length
		? parts.join(" · ")
		: t("devices:serviceConfig.pins.none", "None");
}

const version = (value: unknown) => listOf(value).join(".");

export interface DiffNames {
	variable(id: string): string;
	event(id: string): string;
	certificate(id: string): string;
}

interface Format {
	t: DevicesT;
	names: DiffNames;
}

const plain = (value: unknown): ReactNode =>
	typeof value === "string" ? value : JSON.stringify(value);

const FIELD_LABEL: Record<
	Exclude<DiffField, "event" | "variable" | "secret">,
	(t: DevicesT) => string
> = {
	app_version: (t) => t("devices:serviceConfig.diff.appVersion", "App version"),
	definitions: (t) =>
		t("devices:serviceConfig.diff.definitions", "Approved definitions"),
	endpoint_host: (t) =>
		t("devices:serviceConfig.field.bind", "Who can reach it"),
	endpoint_port: (t) => t("devices:serviceConfig.field.port", "Port"),
	authentication: (t) =>
		t("devices:serviceConfig.diff.authentication", "Service access"),
	token: (t) => t("devices:serviceConfig.token.name", "Access token"),
	certificate: (t) => t("devices:serviceConfig.field.cert", "Certificate"),
	instances: (t) => t("devices:serviceConfig.field.max", "Max instances"),
	cloud_access: (t) =>
		t("devices:serviceConfig.diff.cloudAccess", "Cloud approval"),
	spending: (t) => t("devices:serviceConfig.diff.spending", "Spending limit"),
	write_buffering: (t) =>
		t("devices:serviceConfig.section.buffering", "Write buffering"),
	isolation: (t) => t("devices:serviceConfig.summary.isolation", "Isolation"),
	packages: (t) =>
		t("devices:serviceConfig.summary.pins", "Pinned models & packages"),
};

function labelOf({ t, names }: Format, row: ConfigDiffRow): string {
	if (row.field === "event") return names.event(row.key ?? "");
	if (row.field === "variable" || row.field === "secret")
		return names.variable(row.key ?? "");
	return FIELD_LABEL[row.field](t);
}

const VALUE: Partial<
	Record<DiffField, (format: Format, value: unknown) => ReactNode>
> = {
	app_version: (_format, value) => <Mono>{short(value)}</Mono>,
	definitions: (_format, value) => <Mono>{short(value)}</Mono>,
	cloud_access: (_format, value) => <Mono>{short(value)}</Mono>,
	spending: (_format, value) => <Mono>{short(value)}</Mono>,
	event: ({ t }, value) =>
		t(
			"devices:serviceConfig.diff.eventPins",
			"event {{event}} · flow {{flow}}",
			{
				event: version(record(value).event_version),
				flow: version(record(value).board_version),
			},
		),
	variable: (_format, value) => <Mono>{plain(value)}</Mono>,
	endpoint_host: ({ t }, value) => (
		<>
			{exposureText(t, exposureOf(String(value)))}{" "}
			<Mono className="text-muted-foreground">({String(value)})</Mono>
		</>
	),
	endpoint_port: (_format, value) => <Mono>{String(value)}</Mono>,
	authentication: ({ t }, value) =>
		value === "none"
			? t("devices:serviceConfig.endpoint.tokenNone", "Not required")
			: t("devices:serviceConfig.diff.tokenRequired", "Token required"),
	certificate: ({ names }, value) => names.certificate(String(value)),
	instances: (_format, value) => <Mono>{String(value)}</Mono>,
	write_buffering: ({ t }, value) => bufferingBudgets(t, value),
	isolation: ({ t }, value) => isolationText(t, value),
	packages: ({ t }, value) =>
		pinsText(
			t,
			listOf(record(value).bit_pins).length,
			listOf(record(value).package_pins).length,
		),
};

/** What "nothing" reads as for a field that has a meaningful absence. */
function absent(t: DevicesT, field: DiffField): string | undefined {
	const texts: Partial<Record<DiffField, string>> = {
		certificate: t("devices:serviceConfig.cert.none", "None · plain HTTP"),
		write_buffering: t("devices:serviceConfig.buffering.off", "Off"),
		isolation: isolationText(t, null),
		cloud_access: t("devices:serviceConfig.diff.noApproval", "No approval"),
		spending: t("devices:serviceConfig.diff.noLimit", "No limit"),
	};
	return texts[field];
}

function secretRow(format: Format, row: ConfigDiffRow): DiffRow {
	const { t } = format;
	const stored = t(
		"devices:serviceConfig.diff.storedSecret",
		"stored secret · never shown",
	);
	return {
		kind: row.kind,
		label: labelOf(format, row),
		before: stored,
		after:
			row.kind === "changed"
				? t("devices:serviceConfig.diff.newSecret", "a new stored value")
				: stored,
	};
}

function tokenRow(format: Format, row: ConfigDiffRow): DiffRow {
	const { t } = format;
	return {
		kind: row.kind,
		label: labelOf(format, row),
		before: t("devices:serviceConfig.diff.tokenOld", "the current token"),
		after: t("devices:serviceConfig.diff.tokenNew", "a new token"),
	};
}

function valueRow(format: Format, row: ConfigDiffRow): DiffRow {
	const render = VALUE[row.field] ?? ((_format, value) => plain(value));
	const none = absent(format.t, row.field);
	const before = row.before === undefined ? none : render(format, row.before);
	const after = row.after === undefined ? none : render(format, row.after);
	// A field whose absence has a name reads as a change, not as something added or removed.
	return {
		kind: none ? "changed" : row.kind,
		label: labelOf(format, row),
		...(before === undefined ? {} : { before }),
		...(after === undefined ? {} : { after }),
	};
}

function modelRow(format: Format, row: ConfigDiffRow): DiffRow {
	if (row.field === "secret") return secretRow(format, row);
	if (row.field === "token") return tokenRow(format, row);
	return valueRow(format, row);
}

const EXTRA_LABEL: Record<ExtraDiffField, (t: DevicesT) => string> = {
	limit_parallel: (t) =>
		t("devices:serviceConfig.field.inflight", "Max parallel requests"),
	limit_timeout: (t) =>
		t("devices:serviceConfig.field.timeout", "Request timeout"),
	origins: (t) => t("devices:serviceConfig.diff.origins", "Allowed origins"),
	restart: (t) => t("devices:serviceConfig.summary.restart", "Restart policy"),
	other: (t) => t("devices:serviceConfig.diff.other", "Other settings"),
};

function extraValue(t: DevicesT, row: ExtraDiffRow, value: unknown): ReactNode {
	if (value === undefined) return undefined;
	if (row.field === "limit_timeout")
		return t("devices:serviceConfig.diff.seconds", "{{count, number}} s", {
			count: Number(value),
		});
	if (row.field === "restart") return restartText(t, value);
	if (row.field === "origins")
		return listOf(value).map(String).join(", ") || "–";
	return <Mono>{plain(value)}</Mono>;
}

function extraRow(t: DevicesT, row: ExtraDiffRow): DiffRow {
	if (row.field === "other")
		return {
			kind: "changed",
			label: EXTRA_LABEL.other(t),
			before: t("devices:serviceConfig.diff.otherBefore", "as stored"),
			after: t("devices:serviceConfig.diff.otherAfter", "edited as JSON"),
		};
	return {
		kind: row.kind,
		label: EXTRA_LABEL[row.field](t),
		before: extraValue(t, row, row.before),
		after: extraValue(t, row, row.after),
	};
}

/** The rows a settings change produces; empty when nothing a device would notice differs. */
export function configDiffRows(
	t: DevicesT,
	before: PlacementConfig,
	after: PlacementConfig,
	names: DiffNames,
): DiffRow[] {
	const format = { t, names };
	return [
		...diffPlacementConfig(before, after).map((row) => modelRow(format, row)),
		...extraDiffRows(before, after).map((row) => extraRow(t, row)),
	];
}

export function ConfigDiff({
	before,
	after,
	names,
	label,
}: Readonly<{
	before: PlacementConfig;
	after: PlacementConfig;
	names: DiffNames;
	label: string;
}>) {
	const { t } = useTranslation("devices");
	return (
		<div
			data-config-diff=""
			className="overflow-hidden rounded-lg border border-border bg-card"
		>
			<DiffRows rows={configDiffRows(t, before, after, names)} label={label} />
		</div>
	);
}

/* How to apply. */

export interface ApplyDraft {
	mode: ApplyMode;
	stabilize: string;
	deadline: string;
}

export const applyDraftOf = (mode: ApplyMode): ApplyDraft => ({
	mode,
	stabilize: "10",
	deadline: "120",
});

export const howOf = (draft: ApplyDraft): ApplyHow => ({
	mode: draft.mode,
	stabilizeS: Number(draft.stabilize),
	deadlineS: Number(draft.deadline),
});

export function applyHowError(
	t: DevicesT,
	error: ApplyHowError,
	draft: ApplyDraft,
): string {
	const texts: Record<ApplyHowError, string> = {
		stabilize_range: t(
			"devices:serviceConfig.apply.stabilizeRange",
			"Must stay healthy for is {{value}} s. Use 2 to 60 s.",
			{ value: draft.stabilize },
		),
		deadline_range: t(
			"devices:serviceConfig.apply.deadlineRange",
			"Time limit to start is {{value}} s. Use 10 to 600 s.",
			{ value: draft.deadline },
		),
		deadline_short: t(
			"devices:serviceConfig.apply.deadlineShort",
			"Time limit to start ({{deadline}} s) must be more than 5 s longer than Must stay healthy for ({{stabilize}} s).",
			{ deadline: draft.deadline, stabilize: draft.stabilize },
		),
	};
	return texts[error];
}

/** SPEC §6.5 Quick update, shown in the sheet itself (sheets never stack). */
export function quickUpdateRows(
	t: DevicesT,
	input: { running: boolean; address?: string; safeUnavailable?: string },
): ConsequenceRows {
	return {
		what: input.running
			? t(
					"devices:serviceConfig.apply.quickWhat",
					"The service stops, then starts with the new settings.",
				)
			: t(
					"devices:serviceConfig.apply.quickWhatStopped",
					"The new settings are stored. The service stays stopped and uses them at its next start.",
				),
		who: !input.running
			? t(
					"devices:serviceConfig.apply.quickWhoStopped",
					"Nobody: the service isn't running.",
				)
			: input.address
				? t(
						"devices:serviceConfig.apply.quickWho",
						"{{address}} is down until it starts again. Requests in flight are cut off.",
						{ address: input.address },
					)
				: t(
						"devices:serviceConfig.apply.quickWhoBackground",
						"Its background events pause until it starts again.",
					),
		when: t("devices:serviceConfig.apply.quickWhen", "Immediately."),
		undo: {
			reversible: true,
			text: t(
				"devices:serviceConfig.apply.quickUndo",
				"Apply the previous settings the same way.",
			),
		},
		...(input.safeUnavailable
			? {
					first: t(
						"devices:serviceConfig.apply.quickFirst",
						"Safe update isn't available: {{reason}}",
						{ reason: input.safeUnavailable },
					),
				}
			: {}),
	};
}

function Timing({
	id,
	label,
	hint,
	value,
	onChange,
}: Readonly<{
	id: string;
	label: string;
	hint: string;
	value: string;
	onChange(value: string): void;
}>) {
	const { t } = useTranslation("devices");
	return (
		<Field id={id} label={label} hint={hint} className="w-44">
			<InputWithUnit
				numeric
				inputMode="numeric"
				value={value}
				onChange={(event) => onChange(event.target.value)}
				unit={t("serviceConfig.unit.seconds", "s")}
			/>
		</Field>
	);
}

/** Safe update (default when it can run) or Quick update, with the reason when one of them can't. */
export function ApplyChoices({
	draft,
	onChange,
	safeUnavailable,
	quickUnavailable,
	singleInstance,
	running,
	error,
}: Readonly<{
	draft: ApplyDraft;
	onChange(draft: ApplyDraft): void;
	/** One sentence; set when a safe update can't run. */
	safeUnavailable?: string;
	quickUnavailable?: string;
	singleInstance: boolean;
	running: boolean;
	error?: string | null;
}>) {
	const { t } = useTranslation("devices");
	const safeHint = safeUnavailable
		? t("serviceConfig.apply.unavailable", "Not available: {{reason}}", {
				reason: safeUnavailable,
			})
		: singleInstance
			? t(
					"serviceConfig.apply.safeHintSingle",
					"The device switches to the new settings and restores the current ones on its own if the new ones aren't healthy in time.",
				)
			: t(
					"serviceConfig.apply.safeHint",
					"The current version keeps running until the new one proves healthy.",
				);
	const quickHint = quickUnavailable
		? t("serviceConfig.apply.unavailable", "Not available: {{reason}}", {
				reason: quickUnavailable,
			})
		: running
			? t(
					"serviceConfig.apply.quickHint",
					"Stops the service, then starts the new settings. It's down until it starts again.",
				)
			: t(
					"serviceConfig.apply.quickHintStopped",
					"Stores the new settings. The service stays stopped and uses them at its next start.",
				);
	const timings = (
		<div className="flex flex-wrap gap-3">
			<Timing
				id="svc-apply-stable"
				label={t("serviceConfig.apply.stabilize", "Must stay healthy for")}
				hint={t("serviceConfig.apply.stabilizeHint", "2 to 60 s")}
				value={draft.stabilize}
				onChange={(stabilize) => onChange({ ...draft, stabilize })}
			/>
			<Timing
				id="svc-apply-deadline"
				label={t("serviceConfig.apply.deadline", "Time limit to start")}
				hint={t("serviceConfig.apply.deadlineHint", "10 to 600 s")}
				value={draft.deadline}
				onChange={(deadline) => onChange({ ...draft, deadline })}
			/>
		</div>
	);
	const options: ChoiceOption<ApplyMode>[] = [
		{
			value: "safe",
			title: t("serviceConfig.apply.safe", "Safe update"),
			hint: safeHint,
			icon: ShieldCheck,
			disabled: !!safeUnavailable,
			// Shown inside the card while Safe update is the choice.
			...(safeUnavailable ? {} : { detail: timings }),
		},
		{
			value: "quick",
			title: t("serviceConfig.apply.quick", "Quick update"),
			hint: quickHint,
			icon: Zap,
			disabled: !!quickUnavailable,
		},
	];
	return (
		<div data-apply-choices="" className="flex min-w-0 flex-col gap-3">
			<ChoiceCards
				id="svc-apply-mode"
				legend={t("serviceConfig.apply.legend", "How to apply")}
				value={draft.mode}
				onValueChange={(mode) => onChange({ ...draft, mode })}
				options={options}
			/>
			{error ? (
				<p
					role="alert"
					data-apply-error=""
					className={cx("text-xs", TONE_TEXT.critical)}
				>
					{error}
				</p>
			) : null}
		</div>
	);
}
