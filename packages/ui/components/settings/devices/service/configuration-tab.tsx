"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	Braces,
	CircleArrowUp,
	KeyRound,
	type LucideIcon,
	Pencil,
	Plus,
	Radio,
	Rocket,
	SlidersHorizontal,
} from "lucide-react";
import { Fragment, type ReactNode, useId, useMemo, useState } from "react";
import { isBotTokenKey } from "../../../../lib/device-management/bot-config";
import type { PlacementConfiguration } from "../../../../lib/device-management/deployment";
import type {
	AppServiceRow,
	AppView,
} from "../../../../lib/device-management/model/app-plan";
import type {
	AgentFeature,
	DevicesRoute,
	DevicesScope,
	GateNoticeKind,
	GateResult,
	PlacementEvent,
} from "../../../../lib/device-management/model/types";
import type { ActivityItem } from "../../../../lib/device-management/workspace/types";
import { humanFileSize } from "../../../../lib/utils";
import { appCopy } from "../copy/app-copy";
import {
	agentTooOldCopy,
	eventRunsCopy,
	eventTypeLabel,
} from "../copy/eligibility-copy";
import { gateCopy } from "../copy/gate-copy";
import { DriftChip, MODE_ICON, VersionCell } from "../primitives/app-chips";
import {
	type AreaTime,
	type DevicesT,
	useAreaTime,
} from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { LatestTag } from "../primitives/event-cell";
import { GateInline, GatedAction } from "../primitives/gate-notice";
import { IdRef } from "../primitives/id-ref";
import { KvGroup, KvRow } from "../primitives/key-value-list";
import { Meter } from "../primitives/meter";
import { StatusChip } from "../primitives/status-chip";
import { TONE_SURFACE, TONE_TEXT, cx } from "../primitives/tone";
import { useRouteLink } from "../routing/use-devices-route";
import type { ServiceTabProps } from "../screen-props";
import { useAppNames } from "../shell/attention-popover";
import {
	type GateTarget,
	useActivity,
	useAppView,
	useAttentionState,
	useFixAction,
	useGate,
} from "../workspace";
import {
	bufferingBudgets,
	exposureText,
	isolationText,
	pinsText,
	restartText,
} from "./config-diff";
import {
	CONFIG_MAX_BYTES,
	type PlacementConfig,
	type SettingField,
	hostingOf,
	jsonBytes,
	pinCounts,
	settingFields,
} from "./config-model";
import {
	ActionResults,
	ConfigStamp,
	ConfigUnavailable,
	FactList,
	LINK,
	Mono,
	type Note,
} from "./config-parts";
import { BufferTarget } from "./offline-tab";
import { RemoveServiceZone } from "./remove-service";
import { ChangeSecretSheet, type SecretChoice } from "./secret-fields";
import { type AppEventRow, addableEvents } from "./service-events";
import {
	EditJsonSheet,
	EditSettingsSheet,
	type SettingsEditor,
	fieldLabel,
	typeWord,
	useSettingsEditor,
} from "./settings-sheets";
import {
	type ServiceConfigRead,
	useSecretWrite,
	useServiceConfig,
} from "./use-service-config";

const triple = (value: readonly number[]) => value.join(".");

/* What the app's metadata says about this service's version (APP §6.1, §7.5). */

interface PinChange {
	eventId: string;
	name: string;
	from: PlacementEvent;
	to: { eventVersion: string; boardVersion: string };
}

interface Upload {
	running: boolean;
	progress?: { done: number; total: number; unit: string };
	expiresAt?: number;
}

interface UpdateFacts {
	appName: string;
	mode: "online" | "offline";
	/** `undefined` while loading; `null` when this account can't read the app. */
	view: AppView | null | undefined;
	row: AppServiceRow | null;
	newestLabel: string | null;
	/** Versions behind, "pins" when only the pins say so, 0 on the newest, `null` when unknown. */
	behind: number | "pins" | null;
	pins: PinChange[];
	/** Served events that follow Latest whose flow has edits no version holds: an update takes them. */
	flowEdits: string[];
	/** Events that are new in the newest version and not served here. */
	fresh: string[];
	/** Events of the app that can run on this device and aren't served here. */
	addable: number;
	/** Nothing else can be added because the device's agent lacks this flag. */
	addNeeds?: AgentFeature;
	upload: Upload | null;
}

function pinChanges(view: AppView, events: readonly PlacementEvent[]) {
	const published = new Map(
		[...view.events.rows, ...view.events.ineligible].map((row) => [
			row.eventId,
			row,
		]),
	);
	const changes: PinChange[] = [];
	let known = true;
	for (const event of events) {
		const row = published.get(event.event_id);
		if (!row?.pin) {
			known = false;
			continue;
		}
		const to = {
			eventVersion: triple(row.pin.eventVersion),
			boardVersion: triple(row.pin.boardVersion),
		};
		if (
			to.eventVersion !== triple(event.event_version) ||
			to.boardVersion !== triple(event.board_version)
		)
			changes.push({
				eventId: event.event_id,
				name: row.name,
				from: event,
				to,
			});
	}
	return { changes, known };
}

type Pins = ReturnType<typeof pinChanges>;

/** Versions behind when the version list says so, "pins" when only the pins differ, 0 on the newest, `null` when unknown. */
function behindOf(row: AppServiceRow | null, online: boolean, pins: Pins) {
	if (row?.behind != null) return row.behind;
	if (pins.changes.length) return "pins" as const;
	// An offline copy's version is the copy itself: equal pins don't prove it is the newest copy.
	return online && pins.known ? 0 : null;
}

const OPEN_UPLOAD = new Set(["active", "paused", "waiting"]);

/** The app's upload to this device that hasn't finished, as the activity tray tracks it. */
function uploadOf(items: readonly ActivityItem[], projectId: string) {
	for (const item of items) {
		const { resume, progress } = item;
		if (resume?.type !== "transfer" || resume.projectId !== projectId) continue;
		if (!OPEN_UPLOAD.has(item.state)) continue;
		const upload: Upload = {
			running: item.state === "active",
			expiresAt: resume.expiresAt,
		};
		if (progress && progress !== "indeterminate") upload.progress = progress;
		return upload;
	}
	return null;
}

/** The app's own name; an app whose metadata has none reads as its id, which is never shown. */
const viewName = (view: AppView | undefined, projectId: string) =>
	view && view.app.name !== projectId ? view.app.name : undefined;

const nameOf = (entry: { name: string }) => entry.name;

type VersionFacts = Pick<
	UpdateFacts,
	| "view"
	| "row"
	| "newestLabel"
	| "behind"
	| "pins"
	| "flowEdits"
	| "fresh"
	| "addable"
	| "addNeeds"
>;

const UNKNOWN_VERSION: Omit<VersionFacts, "view"> = {
	row: null,
	newestLabel: null,
	behind: null,
	pins: [],
	flowEdits: [],
	fresh: [],
	addable: 0,
};

/** How this service's pins compare with what the app publishes now, and what else it could serve. */
function versionFacts(
	view: AppView,
	mode: "online" | "offline",
	events: readonly PlacementEvent[],
	deviceId: string,
	serviceId: string,
): VersionFacts {
	const row =
		view.services.find(
			(entry) => entry.deviceId === deviceId && entry.serviceId === serviceId,
		) ?? null;
	const served = new Set(events.map((event) => event.event_id));
	const pins = pinChanges(view, events);
	const newest = view.versions[0]?.label ?? null;
	const { open, addable, needs } = addableEvents(
		view.events.rows,
		deviceId,
		served,
	);
	const hasEdits = (entry: AppEventRow) =>
		served.has(entry.eventId) &&
		entry.cells[deviceId]?.drift?.state === "edits";
	const isNew = (entry: AppEventRow) => entry.newIn === newest;
	const flowEdits = [...view.events.rows, ...view.events.ineligible]
		.filter(hasEdits)
		.map(nameOf);
	return {
		view,
		row,
		newestLabel: newest,
		behind: behindOf(row, mode === "online", pins),
		pins: pins.changes,
		flowEdits,
		fresh: newest ? open.filter(isNew).map(nameOf) : [],
		addable,
		...(needs ? { addNeeds: needs } : {}),
	};
}

function useUpdateFacts(
	deviceId: string,
	serviceId: string,
	configuration: PlacementConfiguration,
): UpdateFacts {
	const { t } = useTranslation("devices");
	const projectId = configuration.project_id;
	const { view, loading } = useAppView(projectId);
	const appName = useAppNames();
	const uploads = useActivity({ deviceId, kind: "upload" });
	const { config } = configuration;
	return useMemo(() => {
		const base = {
			appName:
				viewName(view, projectId) ??
				appName(projectId) ??
				t("serviceConfig.app.unnamed", "this app"),
			mode:
				config.source === "online" ? ("online" as const) : ("offline" as const),
			upload: uploadOf(uploads.items, projectId),
		};
		return view
			? {
					...base,
					...versionFacts(view, base.mode, config.events, deviceId, serviceId),
				}
			: { ...base, ...UNKNOWN_VERSION, view: loading ? undefined : null };
	}, [
		t,
		view,
		loading,
		appName,
		uploads.items,
		config,
		projectId,
		deviceId,
		serviceId,
	]);
}

const isBehind = (facts: UpdateFacts) =>
	facts.behind === "pins" ||
	(typeof facts.behind === "number" && facts.behind > 0);

/** A button label inside a sentence: without its trailing ellipsis. */
const bare = (label: string) => label.replace(/…$/, "");

interface LineGate {
	kind: GateNoticeKind;
	text: string;
}

type ActionId = "edit" | "secret" | "update" | "add" | "json";

interface RowAction {
	id: ActionId;
	label: string;
	gate: LineGate | null;
}

interface Line {
	kind: GateNoticeKind;
	text: string;
	/** Actions the line explains. */
	ids: ActionId[];
}

/* The update note under the action row. */

/** "Extract invoice 1.4.0 → 1.5.0 (flow 2.1.0 → 2.2.0)". */
function PinText({ pin }: Readonly<{ pin: PinChange }>) {
	const { t } = useTranslation("devices");
	const components = {
		1: <span>{pin.name}</span>,
		2: <Mono>{triple(pin.from.event_version)}</Mono>,
		3: <Mono>{pin.to.eventVersion}</Mono>,
		4: <Mono>{triple(pin.from.board_version)}</Mono>,
		5: <Mono>{pin.to.boardVersion}</Mono>,
	};
	return pin.to.boardVersion === triple(pin.from.board_version) ? (
		<Trans
			t={t}
			i18nKey="serviceConfig.update.pin"
			defaults="<1/> <2/> → <3/>"
			components={components}
		/>
	) : (
		<Trans
			t={t}
			i18nKey="serviceConfig.update.pinFlow"
			defaults="<1/> <2/> → <3/> (flow <4/> → <5/>)"
			components={components}
		/>
	);
}

function PinList({ pins }: Readonly<{ pins: readonly PinChange[] }>) {
	return (
		<span data-pins="">
			{pins.map((pin, index) => (
				<Fragment key={pin.eventId}>
					{index ? ", " : null}
					<PinText pin={pin} />
				</Fragment>
			))}
		</span>
	);
}

/** What Update… does for this service, in one sentence per mode. */
function UpdateSentence({
	facts,
	label,
	serviceId,
	device,
}: Readonly<{
	facts: UpdateFacts;
	label: string;
	serviceId: string;
	device: string;
}>) {
	const { t } = useTranslation("devices");
	const components = {
		1: <b className="font-semibold">{bare(label)}</b>,
		2: <Mono>{serviceId}</Mono>,
		3: <PinList pins={facts.pins} />,
		4: <span>{facts.appName}</span>,
		5: <span>{device}</span>,
	};
	const pinned = facts.pins.length > 0;
	if (facts.mode === "online")
		return pinned ? (
			<Trans
				t={t}
				i18nKey="serviceConfig.update.repinsPins"
				defaults="<1/> re-pins <2/> to the event and flow versions published now: <3/>. Data stays in the cloud and isn't touched."
				components={components}
			/>
		) : (
			<Trans
				t={t}
				i18nKey="serviceConfig.update.repins"
				defaults="<1/> re-pins <2/> to the event and flow versions published now. Data stays in the cloud and isn't touched."
				components={components}
			/>
		);
	return pinned ? (
		<Trans
			t={t}
			i18nKey="serviceConfig.update.copyPins"
			defaults="<1/> sends a new copy of <4/> from this computer. It replaces the app on <5/>. The data on the device stays as it is; the tables in the new copy aren't used by <2/>. Events: <3/>."
			components={components}
		/>
	) : (
		<Trans
			t={t}
			i18nKey="serviceConfig.update.copy"
			defaults="<1/> sends a new copy of <4/> from this computer. It replaces the app on <5/>. The data on the device stays as it is; the tables in the new copy aren't used by <2/>."
			components={components}
		/>
	);
}

function BehindLines({
	facts,
	label,
	serviceId,
	device,
}: Readonly<{
	facts: UpdateFacts;
	label: string;
	serviceId: string;
	device: string;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const version = facts.row?.version;
	const newest = facts.view?.versions[0];
	return (
		<>
			<p data-update-line="behind" className="text-ui">
				<UpdateSentence
					facts={facts}
					label={label}
					serviceId={serviceId}
					device={device}
				/>
			</p>
			{facts.flowEdits.map((event) => (
				<p key={event} data-update-line="flow-edits" className="text-ui">
					{t(
						"deploy.what.updateTakesEdits",
						"Updating also takes the current flow edits of {{event}}.",
						{ event },
					)}
				</p>
			))}
			{facts.fresh.length ? (
				<p data-update-line="fresh" className="text-ui">
					{t("serviceConfig.update.fresh", {
						count: facts.fresh.length,
						names: new Intl.ListFormat(time.locale).format(facts.fresh),
						version: facts.newestLabel ?? "",
						service: serviceId,
						defaultValue_one:
							"{{names}} is new in {{version}} and isn't served here. Updating keeps the events {{service}} serves; use Add an event to serve it too.",
						defaultValue_other:
							"{{names}} are new in {{version}} and aren't served here. Updating keeps the events {{service}} serves; use Add an event to serve them too.",
					})}
				</p>
			) : null}
			{version?.label && newest?.label && typeof facts.behind === "number" ? (
				<p data-update-line="versions" className="text-ui">
					{t("serviceConfig.update.versions", {
						count: facts.behind,
						version: version.label,
						date: version.builtAt ? time.at(version.builtAt) : "–",
						newest: newest.label,
						when: newest.builtAt ? time.at(newest.builtAt) : "–",
						defaultValue_one:
							"It runs {{version}} from {{date}}. Newest is {{newest}} from {{when}}.",
						defaultValue_other:
							"It runs {{count, number}} versions behind: {{version}} from {{date}}. Newest is {{newest}} from {{when}}.",
					})}
				</p>
			) : null}
		</>
	);
}

function uploadText(
	t: DevicesT,
	time: AreaTime,
	upload: Upload,
	version: string | null,
	device: string,
): string {
	const what = version
		? t(
				"devices:serviceConfig.update.uploadOf",
				"An upload of {{version}} to {{device}}",
				{
					version,
					device,
				},
			)
		: t(
				"devices:serviceConfig.update.uploadOfNew",
				"An upload of the new version to {{device}}",
				{
					device,
				},
			);
	const { progress } = upload;
	const at = !progress
		? ""
		: progress.unit === "bytes"
			? t("devices:serviceConfig.update.atBytes", " at {{done}} of {{total}}", {
					done: humanFileSize(progress.done),
					total: humanFileSize(progress.total),
				})
			: t(
					"devices:serviceConfig.update.atFiles",
					" at {{done, number}} of {{total, number}} files",
					{ done: progress.done, total: progress.total },
				);
	const until =
		!upload.running && upload.expiresAt
			? t(
					"devices:serviceConfig.update.resumable",
					", resumable until {{time}}",
					{
						time: time.at(upload.expiresAt),
					},
				)
			: "";
	return upload.running
		? t(
				"devices:serviceConfig.update.uploadRunning",
				"{{what}} is running{{at}}. Nothing changes on the device until it finishes and you apply it.",
				{ what, at },
			)
		: t(
				"devices:serviceConfig.update.uploadPaused",
				"{{what}} is paused{{at}}{{until}}. Nothing changes on the device until it finishes and you apply it.",
				{ what, at, until },
			);
}

interface UpdateNoteProps {
	facts: UpdateFacts;
	label: string;
	serviceId: string;
	device: string;
}

/** Where the service stands against the app's versions: unreadable, behind, newest or not known yet. */
function VersionLines({ facts, label, serviceId, device }: UpdateNoteProps) {
	const { t } = useTranslation("devices");
	if (facts.view === null)
		return (
			<p data-update-line="unreadable" className="text-ui">
				{t(
					"serviceConfig.update.unreadable",
					"This app's versions aren't readable with your role, so the wizard shows only this service's own versions.",
				)}
			</p>
		);
	if (isBehind(facts))
		return (
			<BehindLines
				facts={facts}
				label={label}
				serviceId={serviceId}
				device={device}
			/>
		);
	if (facts.behind === 0)
		return (
			<p data-update-line="newest" className="text-ui">
				{facts.newestLabel
					? t(
							"serviceConfig.update.newest",
							"Runs the newest version, {{version}}.",
							{ version: facts.newestLabel },
						)
					: t(
							"serviceConfig.update.newestPins",
							"Runs the event and flow versions published now.",
						)}{" "}
				{appCopy(t).publishNote(facts.mode)}
			</p>
		);
	return facts.view ? (
		<p data-update-line="unknown" className="text-ui">
			{t(
				"serviceConfig.update.unknown",
				"The app version on {{device}} isn't known yet. The wizard reads it before anything changes.",
				{ device },
			)}
		</p>
	) : null;
}

const NOTE_LOOK = {
	upload: TONE_SURFACE.warning,
	behind: TONE_SURFACE.info,
	plain: "border-border bg-surface-sunken",
};

interface UpdateNoteFullProps extends UpdateNoteProps {
	/** The wizard's copy step, while an upload runs. */
	resume: ReactNode;
}

function UpdateNote({ resume, ...props }: UpdateNoteFullProps) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { facts, device } = props;
	const Icon: LucideIcon = MODE_ICON[facts.mode];
	if (facts.view === undefined && !facts.upload) return null;
	const look = facts.upload ? "upload" : isBehind(facts) ? "behind" : "plain";
	return (
		<div
			data-update-note={look}
			className={cx(
				"flex max-w-[92ch] items-start gap-2.5 rounded-lg border px-3 py-2.5 text-ui",
				NOTE_LOOK[look],
			)}
		>
			<Icon aria-hidden className="mt-0.5 size-4 shrink-0 text-ink-2" />
			<div className="flex min-w-0 flex-col gap-1.5">
				<VersionLines {...props} />
				{facts.upload ? (
					<p data-update-line="upload" className="text-ui">
						{uploadText(t, time, facts.upload, facts.newestLabel, device)}{" "}
						{facts.upload.running ? resume : null}
					</p>
				) : null}
			</div>
		</div>
	);
}

/* The action row and its gate lines. */

/** A reason that disables every action stands alone; any other line names the actions it explains (R7). */
function gateLines(
	t: DevicesT,
	locale: string,
	actions: readonly RowAction[],
): Line[] {
	const lines: (Line & { labels: string[] })[] = [];
	for (const action of actions) {
		if (!action.gate) continue;
		const known = lines.find((line) => line.text === action.gate?.text);
		if (known) {
			known.ids.push(action.id);
			known.labels.push(bare(action.label));
		} else
			lines.push({
				...action.gate,
				ids: [action.id],
				labels: [bare(action.label)],
			});
	}
	return lines.map(({ labels, ...line }) =>
		line.ids.length === actions.length
			? line
			: {
					...line,
					text: t(
						"devices:serviceConfig.gate.named",
						"{{actions}}: {{reason}}",
						{
							actions: new Intl.ListFormat(locale).format(labels),
							reason: line.text,
						},
					),
				},
	);
}

function lineOf(
	t: DevicesT,
	time: AreaTime,
	result: GateResult,
): LineGate | null {
	return result.ok
		? null
		: { kind: result.kind, text: gateCopy(t, result, time).inline };
}

interface ActionModel {
	actions: RowAction[];
	lines: Line[];
	updateLabel: string;
	resuming: boolean;
	routes: {
		update: DevicesRoute;
		resume: DevicesRoute;
		add: DevicesRoute;
		/** A bot's new token: an update of the service, from its Settings step. */
		token: DevicesRoute;
		scope?: DevicesScope;
	};
}

/** While an update holds the settings, one sentence names everything that waits for it. */
function rolloutLine(
	t: DevicesT,
	time: AreaTime,
	editor: SettingsEditor,
	labels: readonly string[],
): LineGate {
	const { rollout } = editor.gates;
	if (rollout?.staged)
		return {
			kind: "busy",
			text: t(
				"devices:serviceConfig.gate.staged",
				"An update is staged. Activate or discard it on Status first.",
			),
		};
	const actions = new Intl.ListFormat(time.locale).format(labels.map(bare));
	return {
		kind: "busy",
		text: rollout?.deadlineAt
			? t(
					"devices:serviceConfig.gate.busyBy",
					"An update is in progress. {{actions}} work again after it finishes (by {{time}} at the latest).",
					{ actions, time: time.clock(rollout.deadlineAt) },
				)
			: t(
					"devices:serviceConfig.gate.busy",
					"An update is in progress. {{actions}} work again after it finishes.",
					{ actions },
				),
	};
}

function useActionModel(
	scope: DevicesScope,
	editor: SettingsEditor,
	facts: UpdateFacts,
	hasSecrets: boolean,
): ActionModel {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { deviceId, serviceId, gates, configuration } = editor;
	const projectId = configuration.project_id;
	const wizardTarget = useMemo<GateTarget>(
		() => ({
			...gates.target,
			extra: {
				...gates.target.extra,
				offlineLocalApp: facts.mode === "offline",
			},
		}),
		[gates.target, facts.mode],
	);
	const wizardGate = useGate("update_service", deviceId, wizardTarget);
	return useMemo(() => {
		const behind = isBehind(facts);
		const resuming = !!facts.upload && !facts.upload.running;
		const version = facts.newestLabel;
		const updateLabel = resuming
			? version
				? t("serviceConfig.action.resumeTo", "Resume update to {{version}}…", {
						version,
					})
				: t("serviceConfig.action.resume", "Resume update…")
			: behind && version
				? t("serviceConfig.action.updateTo", "Update to {{version}}…", {
						version,
					})
				: t("serviceConfig.action.update", "Update…");
		const labels: Record<ActionId, string> = {
			edit: t("serviceConfig.action.edit", "Edit settings…"),
			secret: t("serviceConfig.action.secret", "Change secret value…"),
			update: updateLabel,
			add: t("serviceConfig.action.add", "Add an event…"),
			json: t("serviceConfig.action.json", "Edit as JSON…"),
		};
		const shown: ActionId[] = hasSecrets
			? ["edit", "secret", "update", "add", "json"]
			: ["edit", "update", "add", "json"];
		const edit = editor.editGate;
		const wizard = lineOf(t, time, wizardGate);
		const noMore: LineGate | null =
			facts.view && facts.addable === 0
				? {
						kind: "unsupported",
						text: facts.addNeeds
							? agentTooOldCopy(t, editor.read.deviceLabel, facts.addNeeds).long
							: t(
									"serviceConfig.gate.noMoreEvents",
									"Every event of {{app}} that can run on a device is already served here.",
									{ app: facts.appName },
								),
					}
				: null;
		const own: Record<ActionId, LineGate | null> = {
			edit,
			secret: lineOf(t, time, gates.secret),
			update: wizard,
			add: wizard ?? noMore,
			json: edit ?? lineOf(t, time, gates.gates.advanced_config),
		};
		// A running or staged update holds everything, whatever else would gate it.
		const held = gates.rollout
			? rolloutLine(
					t,
					time,
					editor,
					shown.map((id) => labels[id]),
				)
			: null;
		const actions = shown.map(
			(id): RowAction => ({ id, label: labels[id], gate: held ?? own[id] }),
		);
		const base = {
			screen: "deploy" as const,
			deviceIds: [deviceId],
			serviceId,
			...(scope.kind === "app"
				? { mode: "update" as const }
				: { appId: projectId }),
		};
		const foreign = scope.kind === "app" && scope.appId !== projectId;
		return {
			actions,
			lines: held
				? [{ ...held, ids: shown }]
				: gateLines(t, time.locale, actions),
			updateLabel,
			resuming,
			routes: {
				update: base,
				resume: { ...base, step: "copy_upload" },
				add: { ...base, step: "what" },
				token: { ...base, step: "settings" },
				...(foreign ? { scope: { kind: "app", appId: projectId } } : {}),
			},
		};
	}, [
		t,
		time,
		scope,
		editor,
		facts,
		hasSecrets,
		wizardGate,
		gates,
		deviceId,
		serviceId,
		projectId,
	]);
}

function RowButton({
	action,
	icon,
	variant,
	describedBy,
	onClick,
	href,
}: Readonly<{
	action: RowAction;
	icon: LucideIcon;
	variant?: "primary" | "ghost";
	describedBy?: string;
	onClick?(): void;
	/** Link props of the deploy wizard; a gated action renders a disabled button instead. */
	href?: ReturnType<ReturnType<typeof useRouteLink>>;
}>) {
	const gated = !!action.gate;
	const common = {
		icon,
		"data-act": `config-${action.id}`,
		variant: gated && variant === "primary" ? undefined : variant,
	};
	if (href && !gated)
		return (
			<DvButton asChild {...common}>
				<a {...href}>{action.label}</a>
			</DvButton>
		);
	return (
		<DvButton
			{...common}
			aria-disabled={gated || undefined}
			aria-describedby={gated ? describedBy : undefined}
			data-gated={action.gate?.kind}
			onClick={onClick}
		>
			{action.label}
		</DvButton>
	);
}

function ConnectLive({ read }: Readonly<{ read: ServiceConfigRead }>) {
	const { t } = useTranslation("devices");
	const run = useFixAction();
	const fix = read.gate?.fix;
	if (fix?.kind !== "connect") return null;
	return (
		<DvButton variant="ghost" icon={Radio} onClick={() => run(fix)}>
			{t("serviceConfig.action.connect", "Connect live")}
		</DvButton>
	);
}

type OpenSheet = "edit" | "secret" | "json" | null;

function ConfigActions({
	model,
	editor,
	facts,
	secrets,
	note,
	onNote,
}: Readonly<{
	model: ActionModel;
	editor: SettingsEditor;
	facts: UpdateFacts;
	secrets: readonly SecretChoice[];
	note: Note | null;
	onNote(note: Note | null): void;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const link = useRouteLink();
	const lineId = useId();
	const { deviceId, serviceId, read, configuration } = editor;
	const writer = useSecretWrite(
		deviceId,
		serviceId,
		read.service,
		read.deviceLabel,
	);
	const [sheet, setSheet] = useState<OpenSheet>(null);
	const options = model.routes.scope
		? { scope: model.routes.scope }
		: undefined;
	const byId = new Map(model.actions.map((action) => [action.id, action]));
	const lineFor = (id: ActionId) => {
		const index = model.lines.findIndex((line) => line.ids.includes(id));
		return index < 0 ? undefined : `${lineId}-${index}`;
	};
	const open = (id: Exclude<OpenSheet, null>) => () => {
		if (!byId.get(id)?.gate) setSheet(id);
	};
	const update = byId.get("update");
	const add = byId.get("add");
	const edit = byId.get("edit");
	const secret = byId.get("secret");
	const json = byId.get("json");
	const wizardNote = (
		<p className="flex items-start gap-1.5 text-xs text-muted-foreground">
			<Rocket aria-hidden className="mt-0.5 size-3 shrink-0" />
			<span>
				{t(
					"serviceConfig.edit.wizardNote",
					"To change the app version, use {{update}}; to serve another event, use {{add}}. Both open the deploy wizard with this service's current values.",
					{ update: bare(model.updateLabel), add: bare(add?.label ?? "") },
				)}
			</span>
		</p>
	);
	return (
		<div data-config-actions="" className="flex min-w-0 flex-col gap-2.5">
			<div className="flex flex-wrap items-center gap-2">
				{edit ? (
					<RowButton
						action={edit}
						icon={Pencil}
						variant="primary"
						describedBy={lineFor("edit")}
						onClick={open("edit")}
					/>
				) : null}
				{secret ? (
					<RowButton
						action={secret}
						icon={KeyRound}
						describedBy={lineFor("secret")}
						onClick={open("secret")}
					/>
				) : null}
				{update ? (
					<RowButton
						action={update}
						icon={Rocket}
						describedBy={lineFor("update")}
						href={link(
							model.resuming ? model.routes.resume : model.routes.update,
							options,
						)}
					/>
				) : null}
				{add ? (
					<RowButton
						action={add}
						icon={Plus}
						describedBy={lineFor("add")}
						href={link(model.routes.add, options)}
					/>
				) : null}
				{json ? (
					<RowButton
						action={json}
						icon={Braces}
						variant="ghost"
						describedBy={lineFor("json")}
						onClick={open("json")}
					/>
				) : null}
				<ConnectLive read={read} />
			</div>
			{edit && !edit.gate ? (
				<p data-config-hint="" className="text-xs text-muted-foreground">
					{secret && !secret.gate
						? t(
								"serviceConfig.action.hintSecrets",
								"Change secret value works while the service runs. Edit settings goes through a safe or quick update and keeps the installed app version.",
							)
						: t(
								"serviceConfig.action.hint",
								"Edit settings goes through a safe or quick update and keeps the installed app version.",
							)}
				</p>
			) : null}
			{model.lines.map((line, index) => (
				<GateInline
					key={line.text}
					kind={line.kind}
					id={`${lineId}-${index}`}
					className="max-w-[92ch]"
				>
					{line.text}
				</GateInline>
			))}
			<UpdateNote
				facts={facts}
				label={model.updateLabel}
				serviceId={serviceId}
				device={read.deviceLabel}
				resume={
					<a className={LINK} {...link(model.routes.resume, options)}>
						{t("serviceConfig.update.openDeploy", "Open in deploy")}
					</a>
				}
			/>
			<ActionResults resultKey={writer.resultKey} />
			<ActionResults
				resultKey={editor.apply.resultKey}
				note={note}
				onDismiss={() => onNote(null)}
			/>
			<EditSettingsSheet
				editor={editor}
				open={sheet === "edit"}
				onOpenChange={(next) => setSheet(next ? "edit" : null)}
				onNote={onNote}
				wizardNote={wizardNote}
			/>
			<EditJsonSheet
				editor={editor}
				open={sheet === "json"}
				onOpenChange={(next) => setSheet(next ? "json" : null)}
				onNote={onNote}
			/>
			{secrets.length ? (
				<ChangeSecretSheet
					open={sheet === "secret"}
					onOpenChange={(next) => setSheet(next ? "secret" : null)}
					serviceId={serviceId}
					deviceLabel={read.deviceLabel}
					secrets={secrets}
					revision={configuration.config_revision}
					writer={writer}
					onSent={(label) =>
						onNote({
							tone: "info",
							text: t(
								"serviceConfig.secret.sent",
								"New value for {{name}} sent at {{time}}. Activity shows when the device saved it.",
								{ name: label, time: time.clock(time.nowS) },
							),
						})
					}
				/>
			) : null}
		</div>
	);
}

/* The summary. */

const Hint = ({ children }: Readonly<{ children: ReactNode }>) => (
	<span className="mt-0.5 block text-xs text-muted-foreground">{children}</span>
);

const Muted = ({ children }: Readonly<{ children: ReactNode }>) => (
	<span className="text-muted-foreground">{children}</span>
);

/** "Web request · event 1.4.0 · flow 2.1.0". */
function EventPins({
	event,
	type,
}: Readonly<{ event: PlacementEvent; type?: string }>) {
	const { t } = useTranslation("devices");
	const components = {
		1: <Mono>{triple(event.event_version)}</Mono>,
		2: <Mono>{triple(event.board_version)}</Mono>,
	};
	return type ? (
		<Trans
			t={t}
			i18nKey="serviceConfig.summary.eventTyped"
			defaults="{{type}} · event <1/> · flow <2/>"
			values={{ type }}
			components={components}
		/>
	) : (
		<Trans
			t={t}
			i18nKey="serviceConfig.summary.eventPlain"
			defaults="event <1/> · flow <2/>"
			components={components}
		/>
	);
}

/** "→ 1.5.0 · flow 2.2.0 in v1.5.0": what the app publishes now for an event served at older pins. */
function NewerPins({
	pin,
	version,
}: Readonly<{ pin: PinChange; version: string | null }>) {
	const { t } = useTranslation("devices");
	const components = {
		1: <Mono>{pin.to.eventVersion}</Mono>,
		2: <Mono>{pin.to.boardVersion}</Mono>,
	};
	const flow = pin.to.boardVersion !== triple(pin.from.board_version);
	if (version)
		return flow ? (
			<Trans
				t={t}
				i18nKey="serviceConfig.summary.newerFlowIn"
				defaults="→ <1/> · flow <2/> in {{version}}"
				values={{ version }}
				components={components}
			/>
		) : (
			<Trans
				t={t}
				i18nKey="serviceConfig.summary.newerIn"
				defaults="→ <1/> in {{version}}"
				values={{ version }}
				components={components}
			/>
		);
	return flow ? (
		<Trans
			t={t}
			i18nKey="serviceConfig.summary.newerFlow"
			defaults="→ <1/> · flow <2/> published now"
			components={components}
		/>
	) : (
		<Trans
			t={t}
			i18nKey="serviceConfig.summary.newer"
			defaults="→ <1/> published now"
			components={components}
		/>
	);
}

function EventRows({
	config,
	facts,
	editor,
}: Readonly<{
	config: PlacementConfig;
	facts: UpdateFacts;
	editor: SettingsEditor;
}>) {
	const { t } = useTranslation("devices");
	const rows = facts.view
		? [...facts.view.events.rows, ...facts.view.events.ineligible]
		: [];
	return (
		<>
			<KvGroup>{t("serviceConfig.summary.events", "Events")}</KvGroup>
			{config.events.map((event, index) => {
				const row = rows.find((entry) => entry.eventId === event.event_id);
				const pin = facts.pins.find(
					(entry) => entry.eventId === event.event_id,
				);
				const label = facts.view
					? editor.names.event(event.event_id)
					: t("serviceConfig.summary.eventNumber", "Event {{number}}", {
							number: index + 1,
						});
				// An event that follows Latest is never "newest" while its flow has edits no version holds, or while its state isn't known.
				const note = appCopy(t).servedNote(row?.cells[editor.deviceId]?.drift);
				return (
					<KvRow key={event.event_id} label={label}>
						<span data-event={event.event_id}>
							<EventPins
								event={event}
								type={
									row
										? eventTypeLabel(t, row.eventType, row.hasPage)
										: undefined
								}
							/>{" "}
							{pin ? (
								<span data-pin="behind" className={TONE_TEXT.info}>
									<NewerPins pin={pin} version={facts.newestLabel} />
								</span>
							) : note ? (
								<span
									data-pin={note.tone === "info" ? "edits" : "unknown"}
									className={
										note.tone === "info"
											? TONE_TEXT.info
											: "text-muted-foreground"
									}
								>
									{note.text}
								</span>
							) : row?.pin ? (
								<span data-pin="newest" className={TONE_TEXT.good}>
									{t("serviceConfig.summary.newest", "newest")}
								</span>
							) : null}
							{row?.eligibility.followsLatest ? (
								<>
									{" "}
									<LatestTag />
								</>
							) : null}
						</span>
						{row ? <Hint>{eventRunsCopy(t, row.eligibility)}</Hint> : null}
					</KvRow>
				);
			})}
		</>
	);
}

/**
 * "Set new…" for a bot's token: an update of the service, from the wizard's
 * Settings step, so the running version keeps its token until the switch.
 */
function TokenSetNew({
	model,
	serviceId,
	explain,
}: Readonly<{
	model: ActionModel;
	serviceId: string;
	/** Says what a new token does; shown under the last token only. */
	explain: boolean;
}>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const update = model.actions.find((action) => action.id === "update");
	const gate = update?.gate
		? { kind: update.gate.kind, reason: update.gate.text }
		: null;
	const label = t("serviceConfig.botToken.setNew", "Set new…");
	const options = model.routes.scope
		? { scope: model.routes.scope }
		: undefined;
	return (
		<span className="mt-1 flex flex-col items-start gap-1">
			{gate ? (
				<GatedAction gate={gate}>
					<DvButton size="xs" icon={KeyRound} data-act="config-token">
						{label}
					</DvButton>
				</GatedAction>
			) : (
				<DvButton asChild size="xs" icon={KeyRound}>
					<a data-act="config-token" {...link(model.routes.token, options)}>
						{label}
					</a>
				</DvButton>
			)}
			{explain ? (
				<Hint>
					{t(
						"serviceConfig.botToken.hint",
						"A new token goes to {{service}} in an update. The bot keeps the stored one until the update switches over, and nobody can read a token back.",
						{ service: serviceId },
					)}
				</Hint>
			) : null}
		</span>
	);
}

function VariableRows({
	fields,
	editor,
	model,
}: Readonly<{
	fields: readonly SettingField[];
	editor: SettingsEditor;
	model: ActionModel;
}>) {
	const { t } = useTranslation("devices");
	const stored = fields.filter(
		(field) => field.section === "variables" && field.variable?.stored,
	);
	const isToken = (field: SettingField) =>
		isBotTokenKey(field.variable?.id ?? "");
	const lastToken = stored.filter(isToken).at(-1);
	return (
		<>
			<KvGroup>{t("serviceConfig.section.variables", "Variables")}</KvGroup>
			{stored.length ? (
				stored.map((field) => (
					<KvRow key={field.id} label={fieldLabel(t, editor.names, field)}>
						{field.kind === "secret" ? (
							<>
								<Muted>
									{t(
										"serviceConfig.summary.storedSecret",
										"stored secret · can't be read back",
									)}
								</Muted>
								{isToken(field) ? (
									<TokenSetNew
										model={model}
										serviceId={editor.serviceId}
										explain={field === lastToken}
									/>
								) : null}
							</>
						) : (
							<>
								<Mono>{field.value}</Mono>{" "}
								<span className="text-xs text-muted-foreground">
									({typeWord(t, field)})
								</span>
							</>
						)}
						{field.variable?.issue ? (
							<span
								data-issue={field.variable.issue}
								className={cx("mt-0.5 block text-xs", TONE_TEXT.warning)}
							>
								{field.variable.issue === "unused"
									? t(
											"serviceConfig.summary.unused",
											"No event of this service uses it any more. Edit settings removes it.",
										)
									: t(
											"serviceConfig.summary.incompatible",
											"The stored value doesn't fit its type. Edit settings replaces or removes it.",
										)}
							</span>
						) : null}
					</KvRow>
				))
			) : (
				<KvRow label="–">
					<Muted>
						{t("serviceConfig.summary.noVariables", "No variables")}
					</Muted>
				</KvRow>
			)}
		</>
	);
}

function EndpointRows({
	config,
	editor,
}: Readonly<{ config: PlacementConfig; editor: SettingsEditor }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { input } = useAttentionState();
	const hosting = hostingOf(config);
	const certificateId = config.tls_certificate_id;
	const notAfter = certificateId
		? (input.live[editor.deviceId]?.certificates?.certificates.find(
				(entry) => entry.certificate_id === certificateId,
			)?.not_after ??
			input.certInventory[editor.deviceId]?.certificates.find(
				(entry) => entry.certificate_id === certificateId,
			)?.not_after)
		: undefined;
	return (
		<>
			<KvGroup>{t("serviceConfig.section.endpoint", "Web endpoint")}</KvGroup>
			<KvRow label={t("serviceConfig.summary.address", "Address")}>
				{hosting ? (
					<>
						<Mono>{`${hosting.host}:${hosting.port}`}</Mono>
						{" · "}
						{exposureText(t, hosting.exposure).toLowerCase()}
					</>
				) : (
					<Muted>
						{t(
							"serviceConfig.summary.noEndpoint",
							"No web endpoint · its events don't answer page or chat requests",
						)}
					</Muted>
				)}
			</KvRow>
			{hosting ? (
				<>
					<KvRow label={t("serviceConfig.field.cert", "Certificate")}>
						{certificateId
							? `${editor.names.certificate(certificateId)}${
									notAfter === undefined
										? ""
										: ` · ${t("serviceConfig.endpoint.expires", "expires {{date}}", { date: time.at(notAfter) })}`
								}`
							: t("serviceConfig.cert.none", "None · plain HTTP")}
					</KvRow>
					<KvRow label={t("serviceConfig.endpoint.limits", "Limits")}>
						{t(
							"serviceConfig.summary.limits",
							"{{parallel, number}} parallel requests · {{timeout, number}} s timeout · allowed origins: {{origins}}",
							{
								parallel: hosting.maxInFlight,
								timeout: hosting.timeoutS,
								origins: hosting.origins.length
									? hosting.origins.join(", ")
									: t(
											"serviceConfig.summary.originsNone",
											"the service page only",
										),
							},
						)}
					</KvRow>
					<KvRow label={t("serviceConfig.token.name", "Access token")}>
						{t("serviceConfig.endpoint.tokenSet", "Set")}
					</KvRow>
				</>
			) : null}
		</>
	);
}

function RunRows({ config }: Readonly<{ config: PlacementConfig }>) {
	const { t } = useTranslation("devices");
	const pins = pinCounts(config);
	const restart = (config as Record<string, unknown>).restart;
	return (
		<>
			<KvGroup>{t("serviceConfig.summary.runs", "Runs")}</KvGroup>
			<KvRow label={t("serviceConfig.field.max", "Max instances")}>
				<span className="tabular-nums">{config.max_replicas}</span>
			</KvRow>
			<KvRow label={t("serviceConfig.summary.isolation", "Isolation")}>
				{isolationText(t, config.resources)}
			</KvRow>
			<KvRow label={t("serviceConfig.summary.restart", "Restart policy")}>
				{restart ? (
					restartText(t, restart)
				) : (
					<Muted>
						{t("serviceConfig.summary.restartDefault", "The device's default")}
					</Muted>
				)}
			</KvRow>
			<KvRow
				label={t("serviceConfig.summary.pins", "Pinned models & packages")}
			>
				{pinsText(t, pins.models, pins.packages)}
			</KvRow>
		</>
	);
}

function BufferingRows({ config }: Readonly<{ config: PlacementConfig }>) {
	const { t } = useTranslation("devices");
	const writes = config.offline_writes;
	const targets = writes
		? [
				...writes.tables.map((table) => (
					<BufferTarget
						key={`t:${table.purpose}:${table.table}`}
						kind="table"
						name={table.table}
						purpose={table.purpose}
						inline
					/>
				)),
				...writes.files.map((file) => (
					<BufferTarget
						key={`f:${file.purpose}:${file.prefix}`}
						kind="folder"
						name={file.prefix}
						purpose={file.purpose}
						inline
					/>
				)),
			]
		: [];
	return (
		<>
			<KvGroup>
				{t("serviceConfig.section.buffering", "Write buffering")}
			</KvGroup>
			<KvRow label={t("serviceConfig.section.buffering", "Write buffering")}>
				{writes ? (
					<>
						{targets.map((target, index) => (
							<Fragment key={target.key}>
								{index ? "; " : null}
								{target}
							</Fragment>
						))}
						<Hint>
							{t(
								"serviceConfig.summary.bufferingHint",
								"The service keeps accepting changes when the internet drops and sends them when it's back.",
							)}
						</Hint>
					</>
				) : (
					<Muted>
						{config.source === "offline"
							? t(
									"serviceConfig.summary.bufferingOffCopy",
									"Off · an offline copy keeps its data on the device, so nothing waits for the cloud",
								)
							: t("serviceConfig.buffering.off", "Off")}
					</Muted>
				)}
			</KvRow>
			{writes ? (
				<KvRow label={t("serviceConfig.summary.budgets", "Budgets")}>
					{bufferingBudgets(t, writes)}
				</KvRow>
			) : null}
		</>
	);
}

function HowRows({
	config,
	facts,
	device,
}: Readonly<{ config: PlacementConfig; facts: UpdateFacts; device: string }>) {
	const { t } = useTranslation("devices");
	const version = facts.row?.version;
	const behind = typeof facts.behind === "number" ? facts.behind : null;
	return (
		<>
			<KvGroup>{t("serviceConfig.summary.how", "How it runs")}</KvGroup>
			<KvRow label={t("serviceConfig.summary.how", "How it runs")}>
				<span data-how-runs={facts.mode}>
					{facts.mode === "online"
						? t(
								"serviceConfig.summary.online",
								"Runs online · data stays in the cloud",
							)
						: t(
								"serviceConfig.summary.offline",
								"Offline copy · data lives only on {{device}}",
								{ device },
							)}
				</span>
				<Hint>{appCopy(t).mode(facts.mode, facts.appName).sentence}</Hint>
			</KvRow>
			<KvRow label={t("serviceConfig.summary.appVersion", "App version")}>
				{version?.label ? (
					<VersionCell
						label={version.label}
						{...(version.hash ? { hash: version.hash } : {})}
						behind={behind}
					/>
				) : (
					<span
						data-app-version="hash"
						className="inline-flex flex-wrap items-center gap-x-1.5 gap-y-1"
					>
						<IdRef
							id={config.revision}
							copyLabel={t(
								"serviceConfig.summary.copyVersion",
								"Copy app version hash",
							)}
						/>
						{facts.behind === "pins" ? (
							<StatusChip tone="info" icon={CircleArrowUp} data-drift="behind">
								{t("serviceConfig.summary.behind", "Behind")}
							</StatusChip>
						) : (
							<DriftChip behind={behind} />
						)}
					</span>
				)}
			</KvRow>
			<KvRow
				label={t(
					"serviceConfig.summary.folderOnDevice",
					"Folder on the device",
				)}
			>
				<span title={config.project_path} className="block truncate font-mono">
					{config.project_path}
				</span>
			</KvRow>
			{config.online_metadata_sha256 ? (
				<KvRow
					label={t("serviceConfig.diff.definitions", "Approved definitions")}
				>
					<IdRef
						id={config.online_metadata_sha256}
						copyLabel={t(
							"serviceConfig.summary.copyDefinitions",
							"Copy definitions hash",
						)}
					/>
				</KvRow>
			) : null}
		</>
	);
}

/** BG19: one message to the device carries the whole settings, so their size is a hard limit until chunked apply exists. */
function SizeMeter({ config }: Readonly<{ config: PlacementConfig }>) {
	const { t } = useTranslation("devices");
	const bytes = jsonBytes(config);
	const text = t(
		"serviceConfig.summary.size",
		"{{bytes, number}} of {{max, number}} bytes",
		{ bytes, max: CONFIG_MAX_BYTES },
	);
	return (
		<div
			data-config-size={bytes}
			className="flex flex-wrap items-center gap-x-3 gap-y-1.5"
		>
			<Meter
				className="w-44"
				label={text}
				segments={[
					{
						value: (bytes / CONFIG_MAX_BYTES) * 100,
						tone: bytes > CONFIG_MAX_BYTES * 0.9 ? "warning" : "neutral",
					},
				]}
			/>
			<span className="text-xs tabular-nums">{text}</span>
			<span className="text-xs text-muted-foreground">
				{t(
					"serviceConfig.summary.sizeHint",
					"Settings over {{max, number}} bytes can't be applied.",
					{ max: CONFIG_MAX_BYTES },
				)}
			</span>
		</div>
	);
}

function ConfigSummary({
	editor,
	facts,
	fields,
	model,
}: Readonly<{
	editor: SettingsEditor;
	facts: UpdateFacts;
	fields: readonly SettingField[];
	model: ActionModel;
}>) {
	const { config } = editor.configuration;
	return (
		<div data-config-summary="" className="@container/cfg min-w-0">
			<div className="grid min-w-0 gap-x-10 gap-y-1 @min-[860px]/cfg:grid-cols-2">
				<FactList>
					<EventRows config={config} facts={facts} editor={editor} />
					<VariableRows fields={fields} editor={editor} model={model} />
					<EndpointRows config={config} editor={editor} />
				</FactList>
				<FactList>
					<RunRows config={config} />
					<BufferingRows config={config} />
					<HowRows
						config={config}
						facts={facts}
						device={editor.read.deviceLabel}
					/>
				</FactList>
			</div>
			<hr className="my-3 border-hairline" />
			<SizeMeter config={config} />
		</div>
	);
}

function SettingsBlock({
	scope,
	editor,
}: Readonly<{ scope: DevicesScope; editor: SettingsEditor }>) {
	const { t } = useTranslation("devices");
	const { read, configuration } = editor;
	const facts = useUpdateFacts(
		editor.deviceId,
		editor.serviceId,
		configuration,
	);
	const [note, setNote] = useState<Note | null>(null);
	const fields = useMemo(
		() => settingFields(configuration.config, editor.context),
		[configuration, editor.context],
	);
	// A bot's token changes in an update ("Set new…"), never in place.
	const secrets = useMemo<SecretChoice[]>(
		() =>
			Object.entries(configuration.config.secret_overrides ?? {}).flatMap(
				([id, reference]) => {
					if (isBotTokenKey(id)) return [];
					const definition = editor.definitions.definitions?.find(
						(entry) => entry.id === id,
					);
					return [
						{
							id,
							reference,
							label: editor.names.variable(id),
							...(definition ? { definition } : {}),
						},
					];
				},
			),
		[configuration, editor.definitions.definitions, editor.names],
	);
	const model = useActionModel(scope, editor, facts, secrets.length > 0);
	const applied = read.service?.settings.applied;
	return (
		<Block
			id="svc-settings"
			icon={SlidersHorizontal}
			title={t("serviceConfig.title", "Settings v{{version}}", {
				version: configuration.config_revision,
			})}
			summary={
				applied != null && applied !== configuration.config_revision
					? t("serviceConfig.running", "running v{{version}}", {
							version: applied,
						})
					: undefined
			}
			stamp={<ConfigStamp read={read} />}
		>
			<ConfigActions
				model={model}
				editor={editor}
				facts={facts}
				secrets={secrets}
				note={note}
				onNote={setNote}
			/>
			<hr className="border-hairline" />
			<ConfigSummary
				editor={editor}
				facts={facts}
				fields={fields}
				model={model}
			/>
		</Block>
	);
}

/** SPEC §5.3 Configuration: what the service is set to, how to change it, and removing it. */
export function ServiceConfigurationTab({
	scope,
	deviceId,
	serviceId,
}: Readonly<ServiceTabProps>) {
	const { t } = useTranslation("devices");
	const read = useServiceConfig(deviceId, serviceId);
	const editor = useSettingsEditor(deviceId, serviceId, read);
	return (
		<div data-service-configuration="" className="flex min-w-0 flex-col gap-4">
			{editor ? (
				<SettingsBlock scope={scope} editor={editor} />
			) : (
				<Block
					id="svc-settings"
					icon={SlidersHorizontal}
					title={t("serviceConfig.titlePlain", "Settings")}
				>
					<ConfigUnavailable read={read} serviceId={serviceId} />
				</Block>
			)}
			<RemoveServiceZone
				deviceId={deviceId}
				serviceId={serviceId}
				read={read}
			/>
		</div>
	);
}
