import type {
	AppMode,
	AppVisibility,
	PinDrift,
} from "../../../../lib/device-management/model/app-plan";
import type { DevicesT } from "../primitives/area-context";

/* APP §7.1 visibility, §7.2 online vs offline copy, §7.5 versions and drift. */

type PinUnknownWhy = Extract<PinDrift, { state: "unknown" }>["why"];

const flowEditsText = (t: DevicesT) =>
	t("devices:app.drift.flowEdits", "Newer flow edits are available");

function pinUnknownText(t: DevicesT, why: PinUnknownWhy): string {
	if (why === "hub")
		return t(
			"devices:app.drift.unknownHub",
			"Unknown: this hub can't say which flow version is current",
		);
	return why === "pin"
		? t(
				"devices:app.drift.unknownPin",
				"Unknown: the event's flow version can't be read",
			)
		: t(
				"devices:app.drift.unknownLoading",
				"Unknown: the flow's current version isn't read yet",
			);
}

export interface VisibilityText {
	label: string;
	tooltip: string;
}

const VISIBILITY = {
	Offline: (t) => ({
		label: t("devices:app.visibility.offline", "Local only"),
		tooltip: t(
			"devices:app.visibility.offlineExplain",
			"Only on this computer. Not synced to your account.",
		),
	}),
	Private: (t) => ({
		label: t("devices:app.visibility.private", "Private"),
		tooltip: t(
			"devices:app.visibility.privateExplain",
			"Synced for your account only.",
		),
	}),
	Prototype: (t) => ({
		label: t("devices:app.visibility.prototype", "Prototype"),
		tooltip: t(
			"devices:app.visibility.prototypeExplain",
			"Development phase. You can invite collaborators.",
		),
	}),
	PublicRequestAccess: (t) => ({
		label: t(
			"devices:app.visibility.publicRequestAccess",
			"Public · on request",
		),
		tooltip: t(
			"devices:app.visibility.publicRequestAccessExplain",
			"Visible. People can ask to join.",
		),
	}),
	Public: (t) => ({
		label: t("devices:app.visibility.public", "Public"),
		tooltip: t(
			"devices:app.visibility.publicExplain",
			"Everyone can join. Listed in the store.",
		),
	}),
} satisfies Record<AppVisibility, (t: DevicesT) => VisibilityText>;

export const EXPLAINER_ROWS = [
	"apps",
	"ships",
	"data",
	"internet",
	"updates",
	"cost",
	"needs",
] as const;
export type ExplainerRow = (typeof EXPLAINER_ROWS)[number];

export interface ModeText {
	chip: string;
	/** The How it runs strip sentence. */
	sentence: string;
	/** Explainer cells for this mode's column. */
	rows: Record<ExplainerRow, string>;
	/** "Why not the other way?" for an app in this mode. */
	whyNot: string;
}

const MODE = {
	online: (t, app) => ({
		chip: t("devices:app.mode.online", "Runs online"),
		sentence: t(
			"devices:app.mode.onlineSentence",
			"Devices run the version you deploy from the hub. Data stays in the cloud, so they need internet.",
		),
		rows: {
			apps: t(
				"devices:app.mode.onlineApps",
				"Online apps: Private, Prototype, Public.",
			),
			ships: t(
				"devices:app.mode.onlineShips",
				"The definitions you approve: events, flows and pages at the versions published now, plus model details and packages. Usually under 2 MiB.",
			),
			data: t(
				"devices:app.mode.onlineData",
				"In the cloud, the same tables and files the app uses everywhere. The device keeps a read cache.",
			),
			internet: t(
				"devices:app.mode.onlineInternet",
				"Needed. Services get cloud access 10 minutes at a time. With write buffering they keep accepting changes during an outage and send them later.",
			),
			updates: t(
				"devices:app.mode.onlineUpdates",
				"Re-pin to the versions published now. The data isn't touched.",
			),
			cost: t(
				"devices:app.mode.onlineCost",
				"The device's compute is yours. Model use is charged to whoever sets the spending limit; a limit is required whenever models are approved. Files written count against the approver's storage.",
			),
			needs: t(
				"devices:app.mode.onlineNeeds",
				"Cloud access for each service, approved by an Admin or Owner of the app (project files: the owner).",
			),
		},
		whyNot: t(
			"devices:app.mode.onlineWhyNot",
			"Why not an offline copy? {{app}}'s data lives on the hub, and devices can't take a copy of an online app to run without internet yet. Turn on write buffering to ride out short outages.",
			{ app },
		),
	}),
	offline: (t, app) => ({
		chip: t("devices:app.mode.offline", "Offline copy"),
		sentence: t(
			"devices:app.mode.offlineSentence",
			"Devices get a copy of this app and its data from this computer. They run it without internet.",
		),
		rows: {
			apps: t("devices:app.mode.offlineApps", "Local-only apps."),
			ships: t(
				"devices:app.mode.offlineShips",
				"A copy of the app: flows, pages, tables with their current rows, files, your files in this app, packages and downloaded models. As big as the app's data.",
			),
			data: t(
				"devices:app.mode.offlineData",
				"Only on the device. Each new service starts from the copy; nothing syncs back.",
			),
			internet: t(
				"devices:app.mode.offlineInternet",
				"Not needed. Only hosted models, if you approve them, use the internet.",
			),
			updates: t(
				"devices:app.mode.offlineUpdates",
				"A new copy from this computer replaces the app. The data on the device stays as it is.",
			),
			cost: t(
				"devices:app.mode.offlineCost",
				"The device's compute is yours. Model use only if you approve hosted models, and then only up to the limit you must set.",
			),
			needs: t(
				"devices:app.mode.offlineNeeds",
				"The desktop app on the computer that has this app.",
			),
		},
		whyNot: t(
			"devices:app.mode.offlineWhyNot",
			"Why not online? {{app}} exists only on this computer, so the hub can't serve it to devices. To run it online, create an online copy of the app first. It gets a new app ID; services of this local-only app keep running it.",
			{ app },
		),
	}),
} satisfies Record<AppMode, (t: DevicesT, app: string) => ModeText>;

const ROW_LABELS = {
	apps: (t) => t("devices:app.mode.rowApps", "Which apps"),
	ships: (t) => t("devices:app.mode.rowShips", "What goes to the device"),
	data: (t) => t("devices:app.mode.rowData", "Where data lives"),
	internet: (t) => t("devices:app.mode.rowInternet", "Internet"),
	updates: (t) => t("devices:app.mode.rowUpdates", "Updates"),
	cost: (t) => t("devices:app.mode.rowCost", "Cost"),
	needs: (t) => t("devices:app.mode.rowNeeds", "Needs"),
} satisfies Record<ExplainerRow, (t: DevicesT) => string>;

export interface VersionFootInput {
	version: string;
	hash: string;
	/** Formatted by the caller ("today 13:00"). */
	when: string;
	mode: AppMode;
	running: number;
	total: number;
	/** Services whose version can't be told (`AppView.newestRuns.unknown`): they may run the newest one. */
	unknown?: number;
}

export interface UpdateLineInput {
	mode: AppMode;
	app: string;
	service: string;
	/** Online: "Extract invoice 1.4.0 → 1.5.0 (flow 2.1.0 → 2.2.0)". */
	pins?: string;
	/** Formatted sizes and counts. */
	size: string;
	files?: number;
}

function versionFoot(t: DevicesT, input: VersionFootInput) {
	const params = {
		version: input.version,
		hash: input.hash,
		when: input.when,
		running: input.running,
		total: input.total,
	};
	if (input.running === 0 && input.unknown)
		return input.mode === "online"
			? t(
					"devices:app.drift.footUnsure",
					"Newest version {{version}} ({{hash}}), built {{when}}, isn't on any service whose version is known.",
					params,
				)
			: t(
					"devices:app.drift.footUnsureLocal",
					"Newest version {{version}} ({{hash}}), changed on this computer {{when}}, isn't on any service whose version is known.",
					params,
				);
	if (input.running === 0)
		return input.mode === "online"
			? t(
					"devices:app.drift.footNotRunning",
					"Newest version {{version}} ({{hash}}), built {{when}}, isn't running anywhere yet.",
					params,
				)
			: t(
					"devices:app.drift.footNotRunningLocal",
					"Newest version {{version}} ({{hash}}), changed on this computer {{when}}, isn't running anywhere yet.",
					params,
				);
	return input.mode === "online"
		? t(
				"devices:app.drift.footRuns",
				"Newest version {{version}} ({{hash}}), built {{when}}, runs on {{running, number}} of {{total, number}} services.",
				params,
			)
		: t(
				"devices:app.drift.footRunsLocal",
				"Newest version {{version}} ({{hash}}), changed on this computer {{when}}, runs on {{running, number}} of {{total, number}} services.",
				params,
			);
}

function updateLine(t: DevicesT, input: UpdateLineInput) {
	return input.mode === "online"
		? t(
				"devices:app.drift.updateOnline",
				"Re-pins {{service}} to the event and flow versions published now: {{pins}}. About {{size}} is sent. Data stays in the cloud and isn't touched.",
				{ service: input.service, pins: input.pins ?? "", size: input.size },
			)
		: t("devices:app.drift.updateOffline", {
				app: input.app,
				count: input.files ?? 0,
				size: input.size,
				service: input.service,
				defaultValue_one:
					"Sends a new copy of {{app}} from this computer: {{count, number}} file, {{size}}. Replaces the app on the device. The data on the device stays as it is; the tables in the new copy aren't used by {{service}}.",
				defaultValue_other:
					"Sends a new copy of {{app}} from this computer: {{count, number}} files, {{size}}. Replaces the app on the device. The data on the device stays as it is; the tables in the new copy aren't used by {{service}}.",
			});
}

/** Copy for the app's mode, visibility and version drift; every string through `t`. */
export function appCopy(t: DevicesT) {
	return {
		visibility: (visibility: AppVisibility): VisibilityText =>
			VISIBILITY[visibility](t),
		mode: (mode: AppMode, app: string): ModeText => MODE[mode](t, app),
		explainerRowLabel: (row: ExplainerRow): string => ROW_LABELS[row](t),
		explainerTitle: (): string =>
			t("devices:app.mode.explainerTitle", "Online or offline copy"),
		explainerSub: (mode: AppMode, app: string): string =>
			mode === "online"
				? t(
						"devices:app.mode.explainerSubOnline",
						"Decided by the app: {{app}} is an online app",
						{ app },
					)
				: t(
						"devices:app.mode.explainerSubOffline",
						"Decided by the app: {{app}} is a local-only app",
						{ app },
					),
		thisApp: (): string => t("devices:app.mode.thisApp", "This app"),
		explainButton: (): string =>
			t("devices:app.mode.explain", "Online or offline?"),
		/** null = unknown, 0 = newest, n = behind. */
		drift: (behind: number | null): string =>
			behind === null
				? t("devices:app.drift.unknown", "Unknown")
				: behind === 0
					? t("devices:app.drift.newest", "Newest")
					: t("devices:app.drift.behind", "{{count, number}} behind", {
							count: behind,
						}),
		driftTitle: (input: {
			version: string;
			date: string;
			newest: string;
			newestDate: string;
		}): string =>
			t(
				"devices:app.drift.title",
				"Runs {{version}} from {{date}}. Newest is {{newest}} from {{newestDate}}.",
				input,
			),
		/** A served event that follows Latest: its flow has edits no published version holds. */
		flowEdits: (): string => flowEditsText(t),
		/** The tag on a served event whose record follows Latest: a device runs the flow as it was at its last deploy or update. */
		followsLatest: (): string =>
			t("devices:app.drift.followsLatest", "Follows Latest"),
		followsLatestTitle: (): string =>
			t(
				"devices:app.drift.followsLatestTitle",
				"A device runs the flow as it was when the service was last deployed or updated.",
			),
		/** Why a served pin can't be compared; never shown as "Newest". */
		pinUnknown: (why: PinUnknownWhy): string => pinUnknownText(t, why),
		/** What stands in for "newest" on a served event that can't be called so: flow edits no version holds, or a flow state that isn't known. */
		servedNote: (
			drift: PinDrift | undefined,
		): { text: string; tone: "info" | "unknown" } | null =>
			drift?.state === "edits"
				? { text: flowEditsText(t), tone: "info" }
				: drift?.state === "unknown"
					? { text: pinUnknownText(t, drift.why), tone: "unknown" }
					: null,
		/** An event that follows Latest, where the app has no flow version to name: "event 0.9.0 · flow as it is now". */
		pinLatest: (event: string): string =>
			t("devices:app.drift.pinLatest", "event {{event}} · flow as it is now", {
				event,
			}),
		staged: (version: string): string =>
			t("devices:app.drift.staged", "{{version}} staged", { version }),
		pinNewest: (version: string): string =>
			t("devices:app.drift.pinNewest", "{{version}} · newest", { version }),
		pinTitle: (input: {
			event: string;
			version: string;
			release: string;
		}): string =>
			t(
				"devices:app.drift.pinTitle",
				"{{event}} {{version}} is part of {{release}}",
				input,
			),
		versionFoot: (input: VersionFootInput): string => versionFoot(t, input),
		/** Named instead of a hash while the newest version is flow edits that no version holds yet. */
		currentEdits: (): string =>
			t(
				"devices:app.versions.currentEditsShort",
				"current edits, no version yet",
			),
		publishNote: (mode: AppMode): string =>
			mode === "online"
				? t(
						"devices:app.drift.publishOnline",
						"Publishing on the hub doesn't change a running device. Each service keeps its version until you update it.",
					)
				: t(
						"devices:app.drift.publishOffline",
						"Changes on this computer reach a device only when you update it. The data on the device is never replaced.",
					),
		olderVersion: (): string =>
			t(
				"devices:app.drift.older",
				"Can't be prepared again. Devices that have it keep it until you update them.",
			),
		updateLine: (input: UpdateLineInput): string => updateLine(t, input),
	};
}

export type AppCopy = ReturnType<typeof appCopy>;
