import type {
	DeploymentEvent,
	DeploymentVariable,
	InstalledProject,
} from "../../deployment";
import type {
	AppDeviceInput,
	AppEventInput,
	AppInput,
	AppVersionInput,
	LocalServiceChange,
} from "../app-plan";
import type { PlanApp, PlanDevice } from "../deploy-plan";
import type {
	AppDevicePlacements,
	Freshness,
	PlacementEvent,
	ServiceView,
} from "../types";

/* APP §5 sample: seven apps, the sample fleet's services of them, E20 rows and local change records. */

export const NOW0 = 1790769600;

export function v(text: string): [number, number, number] {
	const [major, minor, patch] = text.split(".").map(Number);
	return [major, minor, patch];
}

interface EventSeed {
	type: string;
	ver: string;
	/** null = follows the latest flow edits. */
	board: string | null;
	active?: boolean;
	page?: boolean;
	canary?: boolean;
}

function evt(id: string, name: string, seed: EventSeed): AppEventInput {
	return {
		id,
		name,
		active: seed.active ?? true,
		event_type: seed.type,
		event_version: v(seed.ver),
		board_version: seed.board ? v(seed.board) : null,
		default_page_id: seed.page ? `page_${id}` : null,
		...(seed.canary ? { canary: { percent: 10 } } : {}),
	};
}

function version(
	hash: string,
	label: string,
	builtAt: number,
	by: string | null,
	pins: [string, string, string][],
): AppVersionInput {
	return {
		hash,
		label,
		builtAt,
		by,
		pins: pins.map(([eventId, ver, board]) => ({
			eventId,
			eventVersion: v(ver),
			boardVersion: v(board),
		})),
	};
}

export const ME = "usr_2Nf8KqLx";

export const HASH = {
	support24: "e46717e72dac11fcaaa3ac1094841e388df4bd0ef07e8045c75a418837ff860b",
	support23: "71c6216b4fd24fcdc7a0130015a710bdb9568eeb9010187ef2e18402a3dae929",
	invoice15: "7c2d1e90a1f4c3b2e5d6a7980c1b2a3d4e5f60718293a4b5c6d7e8f901a2b3c4",
	invoice14: "491e8acfcbe8a07759ae759cd5c46a6717d3fd67a18a56c68d1f70212e332f91",
	invoice13: "684d0cd5a6474c11ada5e65b05ceff3aaf1e7fc0aafc64e2685200818bf5144d",
	crm31: "7c2d1e90b4a35f8e6d1c0a9b8e7f6d5c4b3a29187e6f5d4c3b2a1908f7e6d5c4",
	crm30: "a589985403501c44bacdf9f1dea8e62c6cf0cef2d613f8c6d6687e3631190b61",
	scan12: "b41c7e0253d9a86f1e04c7b2d5a93f68e1c0b7d4a2f96e35c8b1d07a4e63f952",
	scan11: "a417baa4fe9fd55c794745dc48594cc3311531acdb4a1158f6faf3cac5bf995f",
	notes20: "ef87af1b693b495e6445e96db85fc4dc719a250e42e0de78432e312e05e29845",
	notes19: "2b17fa9604d3cd1e9ba786c47861395f8909f8a0e7350ecfa9cb2d4ee3a9bbea",
	visitor04: "bfbae1d8cfe7b880ea5c3d99a6d26b53aa48b08c63b4054aa5a37b46df968a1d",
	visitor03: "247853dd8ea1aa70ad31cca473a712cad70a0fabb22128e212ec64e31fa00176",
} as const;

export const APPS = {
	app_support_portal: {
		id: "app_support_portal",
		name: "Support Portal",
		visibility: "Offline",
		versions: [
			version(HASH.support24, "v2.4.0", 1790760600, null, [
				["evt_support_chat", "2.4.0", "5.2.0"],
				["evt_support_http", "1.0.4", "5.2.0"],
			]),
			version(HASH.support23, "v2.3.0", 1789819200, null, [
				["evt_support_chat", "2.3.0", "5.1.2"],
				["evt_support_http", "1.0.4", "5.1.2"],
			]),
		],
		events: [
			evt("evt_support_chat", "Support chat", {
				type: "simple_chat",
				ver: "2.4.0",
				board: "5.2.0",
			}),
			evt("evt_support_reply", "Quick reply", {
				type: "quick_action",
				ver: "1.0.0",
				board: "5.2.0",
			}),
			evt("evt_support_http", "Support API", {
				type: "http",
				ver: "1.0.4",
				board: "5.2.0",
			}),
			evt("evt_support_digest", "Escalation digest", {
				type: "cron",
				ver: "1.1.0",
				board: "5.2.0",
			}),
			evt("evt_support_mailbox", "Support inbox", {
				type: "email",
				ver: "0.3.0",
				board: null,
				active: false,
			}),
		],
	},
	app_invoice_ai: {
		id: "app_invoice_ai",
		name: "Invoice AI",
		visibility: "Prototype",
		versions: [
			version(HASH.invoice15, "v1.5.0", 1790766000, ME, [
				["evt_extract_http", "1.5.0", "2.2.0"],
				["evt_gpu_extract", "1.1.0", "1.4.0"],
				["evt_invoice_mcp", "1.0.2", "2.2.0"],
			]),
			version(HASH.invoice14, "v1.4.0", 1790691600, ME, [
				["evt_extract_http", "1.4.0", "2.1.0"],
				["evt_gpu_extract", "1.0.2", "1.3.0"],
			]),
			version(HASH.invoice13, "v1.3.0", 1789212000, "usr_9QmT3rVb", [
				["evt_extract_http", "1.3.0", "2.0.0"],
				["evt_gpu_extract", "1.0.2", "1.3.0"],
			]),
		],
		events: [
			evt("evt_invoice_review", "Review queue", {
				type: "page",
				ver: "0.9.0",
				board: null,
				page: true,
			}),
			evt("evt_extract_http", "Extract invoice", {
				type: "http",
				ver: "1.5.0",
				board: "2.2.0",
			}),
			evt("evt_gpu_extract", "Extract invoice (GPU)", {
				type: "http",
				ver: "1.1.0",
				board: "1.4.0",
			}),
			evt("evt_invoice_mcp", "Invoice tools (MCP)", {
				type: "mcp",
				ver: "1.0.2",
				board: "2.2.0",
			}),
			evt("evt_invoice_inbox", "Invoice mailbox", {
				type: "inbound_email",
				ver: "1.2.0",
				board: "2.2.0",
			}),
			evt("evt_invoice_reconcile", "Nightly reconciliation", {
				type: "cron",
				ver: "1.0.0",
				board: "1.3.0",
			}),
		],
	},
	app_crm_sync: {
		id: "app_crm_sync",
		name: "CRM Sync",
		visibility: "Offline",
		versions: [
			version(HASH.crm31, "v3.1.0", 1790768400, null, [
				["evt_crm_nightly", "1.2.0", "4.1.0"],
				["evt_crm_webhook", "1.0.0", "4.1.0"],
				["evt_crm_watch", "0.3.0", "4.1.0"],
			]),
			version(HASH.crm30, "v3.0.0", 1790006400, null, [
				["evt_crm_nightly", "1.1.0", "4.0.0"],
				["evt_crm_webhook", "1.0.0", "4.0.0"],
			]),
		],
		events: [
			evt("evt_crm_nightly", "Nightly CRM sync", {
				type: "daemon",
				ver: "1.2.0",
				board: "4.1.0",
			}),
			evt("evt_crm_webhook", "CRM webhook", {
				type: "http",
				ver: "1.0.0",
				board: "4.1.0",
			}),
			evt("evt_crm_watch", "Watch import folder", {
				type: "daemon",
				ver: "0.3.0",
				board: "4.1.0",
			}),
			evt("evt_crm_hourly", "Hourly sync", {
				type: "cron",
				ver: "1.0.0",
				board: "4.1.0",
			}),
			evt("evt_crm_rest", "Sync REST API", {
				type: "rest",
				ver: "1.1.0",
				board: "4.1.0",
				canary: true,
			}),
		],
	},
	app_warehouse_scan: {
		id: "app_warehouse_scan",
		name: "Warehouse Scanner",
		visibility: "Offline",
		versions: [
			version(HASH.scan12, "v1.2.0", 1790665200, null, [
				["evt_scan_ingest", "2.0.1", "1.4.0"],
				["evt_scan_station", "1.0.0", "1.0.0"],
			]),
			version(HASH.scan11, "v1.1.0", 1789387200, null, [
				["evt_scan_ingest", "2.0.0", "1.3.0"],
			]),
		],
		events: [
			evt("evt_scan_station", "Scan station", {
				type: "page",
				ver: "1.0.0",
				board: "1.0.0",
				page: true,
			}),
			evt("evt_scan_ingest", "Scan ingest", {
				type: "daemon",
				ver: "2.0.1",
				board: "1.4.0",
			}),
			evt("evt_scan_deeplink", "Open scan", {
				type: "deeplink",
				ver: "1.0.0",
				board: "1.0.0",
			}),
			evt("evt_shift_report", "Shift report", {
				type: "cron",
				ver: "1.0.0",
				board: "1.0.0",
			}),
		],
	},
	app_field_notes: {
		id: "app_field_notes",
		name: "Field Notes",
		visibility: "Private",
		versions: [
			version(HASH.notes20, "v2.0.0", 1790424000, ME, [
				["evt_notes_http", "1.2.0", "3.0.1"],
			]),
			version(HASH.notes19, "v1.9.0", 1788940800, ME, [
				["evt_notes_http", "1.1.0", "3.0.0"],
			]),
		],
		events: [
			evt("evt_notes_form", "New note", {
				type: "generic_form",
				ver: "1.0.0",
				board: "3.0.1",
			}),
			evt("evt_notes_http", "Notes page", {
				type: "http",
				ver: "1.2.0",
				board: "3.0.1",
			}),
		],
	},
	app_partner_reports: {
		id: "app_partner_reports",
		name: "Partner Reports",
		visibility: "PublicRequestAccess",
		versions: null,
		events: [
			evt("evt_render_report", "Render report", {
				type: "rest",
				ver: "1.3.0",
				board: "4.0.2",
			}),
			evt("evt_report_tools", "Report tools", {
				type: "mcp",
				ver: "1.0.0",
				board: "4.0.2",
			}),
			evt("evt_report_bot", "Partner bot", {
				type: "teams",
				ver: "1.0.0",
				board: "4.0.2",
			}),
		],
	},
	app_visitor_checkin: {
		id: "app_visitor_checkin",
		name: "Visitor Check-in",
		visibility: "Private",
		versions: [
			version(HASH.visitor04, "v0.4.0", 1790757000, ME, [
				["evt_visitor_page", "0.4.0", "0.4.0"],
				["evt_badge_printer", "0.2.1", "0.4.0"],
			]),
			version(HASH.visitor03, "v0.3.0", 1788339600, ME, [
				["evt_visitor_page", "0.3.0", "0.3.0"],
				["evt_badge_printer", "0.2.0", "0.3.0"],
			]),
		],
		events: [
			evt("evt_visitor_page", "Check-in page", {
				type: "page",
				ver: "0.4.0",
				board: "0.4.0",
				page: true,
			}),
			evt("evt_badge_printer", "Badge printer", {
				type: "daemon",
				ver: "0.2.1",
				board: "0.4.0",
			}),
			evt("evt_visitor_mail", "Welcome email", {
				type: "email",
				ver: "0.1.0",
				board: "0.4.0",
			}),
			evt("evt_visitor_report", "Daily visitor report", {
				type: "cron",
				ver: "0.1.0",
				board: "0.2.0",
			}),
		],
	},
} satisfies Record<string, AppInput>;

export type SampleAppId = keyof typeof APPS;

const LIVE: Freshness = { src: "live", age: "live", at: NOW0 - 2 };
const LAST_KNOWN: Freshness = {
	src: "snap",
	age: "lastknown",
	at: NOW0 - 3 * 3600,
};

function pins(rows: [string, string, string][]): PlacementEvent[] {
	return rows.map(([event_id, ver, board]) => ({
		event_id,
		event_version: v(ver),
		board_version: v(board),
	}));
}

type ServiceSeed = Pick<ServiceView, "deviceId" | "serviceId" | "projectId"> &
	Partial<ServiceView>;

export function svc(seed: ServiceSeed): ServiceView {
	return {
		deploymentId: `dep-${seed.serviceId}`,
		desired: "running",
		observed: "running",
		conv: "converged",
		settings: { applied: 1, latest: 1 },
		instances: { requested: 1, ready: 1, running: 1, max: 1 },
		freshness: LIVE,
		source: null,
		events: null,
		appVersion: null,
		...seed,
	};
}

export const SERVICES = {
	supportBot: svc({
		deviceId: "edge-berlin-01",
		serviceId: "support-bot",
		projectId: "app_support_portal",
		source: "offline",
		settings: { applied: 7, latest: 7 },
		instances: { requested: 2, ready: 2, running: 2, max: 4 },
		events: pins([
			["evt_support_chat", "2.3.0", "5.1.2"],
			["evt_support_http", "1.0.4", "5.1.2"],
		]),
		appVersion: { label: "v2.3.0", hash: HASH.support23 },
	}),
	invoiceExtractor: svc({
		deviceId: "edge-berlin-01",
		serviceId: "invoice-extractor",
		projectId: "app_invoice_ai",
		observed: "starting",
		conv: "update_in_progress",
		source: "online",
		settings: { applied: 11, latest: 12 },
		events: pins([["evt_extract_http", "1.4.0", "2.1.0"]]),
		appVersion: { label: "v1.4.0", hash: HASH.invoice14 },
		offlineWrites: { pending: 0, quarantined: false },
	}),
	nightlySync: svc({
		deviceId: "edge-berlin-01",
		serviceId: "nightly-sync",
		projectId: "app_crm_sync",
		desired: "stopped",
		observed: "stopped",
		conv: "stopped_by_user",
		source: "offline",
		settings: { applied: 3, latest: 3 },
		instances: { requested: 0, ready: 0, running: 0, max: 1 },
		events: pins([["evt_crm_nightly", "1.1.0", "4.0.0"]]),
		appVersion: { label: "v3.0.0", hash: HASH.crm30 },
	}),
	invoiceGpu: svc({
		deviceId: "lab-gpu-02",
		serviceId: "invoice-extractor-gpu",
		projectId: "app_invoice_ai",
		source: "online",
		settings: { applied: 4, latest: 4 },
		events: pins([["evt_gpu_extract", "1.0.2", "1.3.0"]]),
		appVersion: { hash: HASH.invoice13 },
	}),
	fieldNotes: svc({
		deviceId: "studio-mac-mini",
		serviceId: "field-notes",
		projectId: "app_field_notes",
		source: "online",
		settings: { applied: 9, latest: 9 },
		events: pins([["evt_notes_http", "1.2.0", "3.0.1"]]),
		appVersion: { label: "v2.0.0", hash: HASH.notes20 },
		offlineWrites: { pending: 14, quarantined: true },
	}),
	scannerIngest: svc({
		deviceId: "warehouse-pi",
		serviceId: "scanner-ingest",
		projectId: "app_warehouse_scan",
		observed: "restarting",
		conv: "crash_looping",
		freshness: LAST_KNOWN,
		source: "offline",
		settings: { applied: 5, latest: 5 },
		appVersion: { hash: HASH.scan11 },
	}),
} satisfies Record<string, ServiceView>;

const DEVICES: Record<string, AppDeviceInput> = {
	edge: {
		id: "edge-berlin-01",
		name: "edge-berlin-01",
		presence: { kind: "online", since: NOW0 - 20 },
		relationship: "owner",
		keyState: "unlocked",
		services: [
			SERVICES.supportBot,
			SERVICES.invoiceExtractor,
			SERVICES.nightlySync,
		],
		refusals: {
			evt_crm_watch:
				"edge-berlin-01 requires sandboxed services, and this flow reads files outside its folder.",
		},
	},
	studio: {
		id: "studio-mac-mini",
		name: "studio-mac-mini",
		presence: { kind: "online", since: NOW0 - 15 },
		relationship: "owner",
		keyState: "unlocked",
		services: [SERVICES.fieldNotes],
	},
	warehouse: {
		id: "warehouse-pi",
		name: "warehouse-pi",
		presence: { kind: "offline", since: NOW0 - 3 * 3600 },
		relationship: "owner",
		keyState: "unlocked",
		services: [SERVICES.scannerIngest],
	},
	cold: {
		id: "cold-storage-nas",
		name: "cold-storage-nas",
		presence: { kind: "never" },
		relationship: "owner",
		keyState: "locked",
		services: { state: "notloaded", reason: { code: "never_reported" } },
	},
	partner: {
		id: "partner-edge",
		name: "partner-edge",
		presence: { kind: "revoked", since: NOW0 - 86400 },
		relationship: "cloud_approval",
		services: { state: "noaccess" },
	},
};

function labDevice(unlocked: boolean): AppDeviceInput {
	return {
		id: "lab-gpu-02",
		name: "lab-gpu-02",
		presence: { kind: "online", since: NOW0 - 40 },
		relationship: "shared",
		keyState: unlocked ? "unlocked" : "locked",
		services: unlocked
			? [SERVICES.invoiceGpu]
			: { state: "locked", reason: { code: "unlock_required" } },
	};
}

/** The sample fleet as App › Devices sees it; lab-gpu-02 is locked unless asked. */
export function sampleDevices(
	options: { labUnlocked?: boolean } = {},
): AppDeviceInput[] {
	return [
		DEVICES.edge,
		labDevice(options.labUnlocked ?? false),
		DEVICES.studio,
		DEVICES.warehouse,
		DEVICES.cold,
		DEVICES.partner,
	].map((device) => ({ ...device }));
}

function grantRow(
	deviceId: string,
	placementId: string,
	grant: Partial<AppDevicePlacements["placements"][number]["grant"]>,
	billing: AppDevicePlacements["placements"][number]["billing"],
): AppDevicePlacements["placements"][number] {
	return {
		device_id: deviceId,
		placement_id: placementId,
		deployment_id: `dep-${placementId}`,
		relationship: "owner",
		grant: {
			grant_id: `grant-${placementId}`,
			status: "active",
			expires_at: NOW0 + 29 * 86400,
			effective_expires_at: NOW0 + 29 * 86400,
			effective_limit: "approval",
			online_access: "read_write",
			model_ids: [],
			max_instances: 1,
			approved_by_user_id: ME,
			created_at: NOW0 - 86400,
			...grant,
		},
		billing,
		instances: { active: 1, newest_lease_expires_at: NOW0 + 600 },
	};
}

/** E20 per app: what the hub lets this viewer see. */
export const PLACEMENTS: Record<SampleAppId, AppDevicePlacements> = {
	app_support_portal: { server_time: NOW0, placements: [] },
	app_invoice_ai: {
		server_time: NOW0,
		placements: [
			grantRow(
				"edge-berlin-01",
				"invoice-extractor",
				{ model_ids: ["gpt-4.1-mini", "bge-m3"] },
				{
					billing_grant_id: "billing-invoice-extractor",
					limit_micros: 25_000_000,
					used_micros: 7_410_000,
					reserved_micros: 0,
					expires_at: NOW0 + 29 * 86400,
					payer_is_me: true,
				},
			),
		],
	},
	app_crm_sync: { server_time: NOW0, placements: [] },
	app_warehouse_scan: { server_time: NOW0, placements: [] },
	app_field_notes: {
		server_time: NOW0,
		placements: [
			grantRow(
				"studio-mac-mini",
				"field-notes",
				{ model_ids: ["gpt-4.1-mini"] },
				null,
			),
		],
	},
	app_partner_reports: {
		server_time: NOW0,
		placements: [
			grantRow(
				"partner-edge",
				"report-renderer",
				{ approved_by_user_id: "usr_4HbW8sPz", online_access: "read_only" },
				{
					billing_grant_id: "billing-report-renderer",
					limit_micros: 50_000_000,
					used_micros: 12_500_000,
					reserved_micros: 0,
					expires_at: NOW0 + 10 * 86400,
					payer_is_me: true,
				},
			),
		],
	},
	app_visitor_checkin: { server_time: NOW0, placements: [] },
};

/** APP §5.4: what this computer recorded when it deployed each service. */
export const CHANGES: Record<string, LocalServiceChange> = {
	"edge-berlin-01/support-bot": {
		at: 1789830600,
		kind: "update",
		hash: HASH.support23,
		settings: 7,
		by: ME,
		seededAt: 1787209200,
	},
	"edge-berlin-01/invoice-extractor": {
		at: 1790697600,
		kind: "update",
		hash: HASH.invoice14,
		settings: 11,
		by: ME,
	},
	"edge-berlin-01/nightly-sync": {
		at: 1790008200,
		kind: "update",
		hash: HASH.crm30,
		settings: 3,
		by: ME,
		seededAt: 1788339600,
	},
	"warehouse-pi/scanner-ingest": {
		at: 1789396200,
		kind: "update",
		hash: HASH.scan11,
		settings: 5,
		by: ME,
		seededAt: 1783497600,
	},
	"studio-mac-mini/field-notes": {
		at: 1790428200,
		kind: "update",
		hash: HASH.notes20,
		settings: 9,
		by: ME,
	},
};

/* Deploy plan facts (APP §3): Visitor Check-in (online, never deployed) and CRM Sync (local-only, forced split). */

function variable(
	id: string,
	name: string,
	dataType: string,
	secret = false,
): DeploymentVariable {
	return { id, name, data_type: dataType, value_type: "Normal", secret };
}

export const VARIABLES = {
	siteName: variable("var_site_name", "Site name", "String"),
	printer: variable("var_printer_url", "Badge printer address", "String"),
	hostToken: variable("var_host_token", "Host directory token", "String", true),
	batch: variable("var_batch_size", "Batch size", "Integer"),
	exportPath: variable("var_export_folder", "Export folder", "PathBuf"),
	exportText: variable("var_export_folder", "Export folder", "String"),
} as const;

export const VISITOR_PLAN_APP: PlanApp = {
	...APPS.app_visitor_checkin,
	variables: {
		evt_visitor_page: [VARIABLES.siteName, VARIABLES.hostToken],
		evt_badge_printer: [VARIABLES.siteName, VARIABLES.printer],
	},
};

export const CRM_PLAN_APP: PlanApp = {
	...APPS.app_crm_sync,
	variables: {
		evt_crm_nightly: [VARIABLES.batch, VARIABLES.exportPath],
		evt_crm_webhook: [VARIABLES.batch],
		evt_crm_watch: [VARIABLES.exportText],
	},
};

function catalogEvent(
	event: AppEventInput,
	hosted: boolean,
	readiness: DeploymentEvent["readiness_kind"],
): DeploymentEvent {
	return {
		id: event.id,
		name: event.name,
		event_type: event.event_type,
		event_version: v(String(event.event_version?.join("."))),
		board_version: v(String(event.board_version?.join("."))),
		hosted,
		readiness_kind: readiness,
		rollout_supported: true,
		eligible: true,
	};
}

const [visitorPage, badgePrinter] = APPS.app_visitor_checkin.events;

/** The approved bundle for Visitor Check-in v0.4.0, as the device verifies it. */
export const VISITOR_CATALOG = {
	events: [
		catalogEvent(badgePrinter, false, "explicit"),
		catalogEvent(visitorPage, true, "listener"),
	],
	variables: {
		evt_visitor_page: [VARIABLES.hostToken, VARIABLES.siteName],
		evt_badge_printer: [VARIABLES.printer, VARIABLES.siteName],
	},
};

export const VISITOR_INSTALLED: InstalledProject = {
	project_id: "app_visitor_checkin",
	project_path: "/private/online/projects/app_visitor_checkin",
	revision: HASH.visitor04,
	source: "online",
	online_metadata_sha256:
		"e647322ba8a0deb7e20632b5a4d4d61a356f13ee81ca306c8a9f58bf47d7e14d",
	online_catalog: VISITOR_CATALOG,
};

const [crmNightly, crmWebhook] = APPS.app_crm_sync.events;

export const CRM_CATALOG = {
	events: [
		catalogEvent(crmNightly, false, "unsupported"),
		catalogEvent(crmWebhook, true, "listener"),
	],
	variables: CRM_PLAN_APP.variables ?? {},
};

export const CRM_INSTALLED: InstalledProject = {
	project_id: "app_crm_sync",
	project_path: `/private/projects/app_crm_sync/revisions/${HASH.crm31}`,
	revision: HASH.crm31,
	source: "offline",
};

export const PLAN_DEVICES: Record<string, PlanDevice> = {
	"edge-berlin-01": {
		id: "edge-berlin-01",
		name: "edge-berlin-01",
		gate: null,
		locked: false,
		services: [
			{
				serviceId: "support-bot",
				projectId: "app_support_portal",
				events: ["evt_support_chat", "evt_support_http"],
			},
			{
				serviceId: "invoice-extractor",
				projectId: "app_invoice_ai",
				events: ["evt_extract_http"],
			},
			{
				serviceId: "nightly-sync",
				projectId: "app_crm_sync",
				events: ["evt_crm_nightly"],
				desired: "stopped",
			},
		],
		reservedIds: ["crm-sync-old"],
		refusals: {
			evt_crm_watch:
				"edge-berlin-01 requires sandboxed services, and this flow reads files outside its folder.",
		},
		portsInUse: [
			{ port: 80 },
			{ port: 8081, serviceId: "invoice-extractor" },
			{ port: 8443, serviceId: "support-bot" },
		],
		isolation: "required",
		memoryBytes: 16 * 1024 ** 3,
	},
	"studio-mac-mini": {
		id: "studio-mac-mini",
		name: "studio-mac-mini",
		gate: null,
		locked: false,
		services: [
			{
				serviceId: "field-notes",
				projectId: "app_field_notes",
				events: ["evt_notes_http"],
			},
		],
		portsInUse: [{ port: 8090, serviceId: "field-notes" }],
		isolation: "none",
	},
	"lab-gpu-02": {
		id: "lab-gpu-02",
		name: "lab-gpu-02",
		gate: null,
		locked: true,
		services: null,
	},
};
