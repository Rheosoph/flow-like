import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { act, useMemo, useState } from "react";
import type {
	DeployDraft,
	DeployResult,
	PlanApp,
	PlanDevice,
	PlanEntry,
} from "../../../../../lib/device-management/model/deploy-plan";
import type { DeployStepId } from "../../../../../lib/device-management/model/types";
import { withDeviceEventSource } from "../../../../../lib/event-source";
import {
	allByRole,
	byRole,
	click,
	clickByText,
	installDom,
	queryByRole,
	settle,
	typeInto,
} from "../../testing/dom-harness";
import type { FakeAgent } from "../../testing/fake-device-api";
import type { FakeWorkspace } from "../../testing/fake-workspace";
import type { MountDevicesOptions } from "../../testing/mount-devices";
import type {
	DeployDeviceCheck,
	DeployPrepared,
	DeployStepProps,
} from "../step-props";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../../testing/mount-devices"
);
await preloadDevices();
const { createFakeWorkspace } = await import("../../testing/fake-workspace");
const { AccessCostStep } = await import("./access-cost-step");
const { CopyUploadStep } = await import("./copy-upload-step");
const { ReviewStep } = await import("./review-step");
const { RolloutStep } = await import("./rollout-step");
const { useActivityTray } = await import("../../shell/activity-tray");
const { WriteBufferingFields, DEFAULT_WRITE_BUFFERING, writeBufferingIssues } =
	await import("./write-buffering-fields");
const {
	pendingArtifactTransfers,
	prepareProjectArtifact,
	rememberArtifactTransfer,
} = await import("../../../../../lib/device-management/artifacts");
const { checkPlan, makePlan, resolvePlan } = await import(
	"../../../../../lib/device-management/model/deploy-plan"
);
const {
	APPS,
	CRM_CATALOG,
	CRM_PLAN_APP,
	NOW0,
	PLAN_DEVICES,
	VISITOR_CATALOG,
	VISITOR_PLAN_APP,
} = await import(
	"../../../../../lib/device-management/model/__fixtures__/apps"
);
const { SAMPLE_IDS } = await import(
	"../../../../../lib/device-management/model/__fixtures__/sample-fleet"
);

const EDGE = SAMPLE_IDS.edge;
const STUDIO = SAMPLE_IDS.studio;
const SHA = "e647322ba8a0deb7e20632b5a4d4d61a356f13ee81ca306c8a9f58bf47d7e14d";
const DEVICES: Record<string, PlanDevice> = {
	[EDGE]: { ...PLAN_DEVICES["edge-berlin-01"], id: EDGE },
	[STUDIO]: { ...PLAN_DEVICES["studio-mac-mini"], id: STUDIO },
};
/** Banned words (APP §7) and anything that looks like a wire code (R3). */
const MACHINE_WORDS =
	/\b(placements?|grants?|replicas?|standalone)\b|\b[a-z]+_[a-z_]+\b/i;

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

const text = (root: ParentNode = document.body) =>
	(root as HTMLElement).textContent?.replace(/\s+/g, " ") ?? "";

function prose(root: HTMLElement): string {
	const copy = root.cloneNode(true) as HTMLElement;
	for (const node of copy.querySelectorAll(
		"[data-idref], code, .font-mono, input",
	))
		node.remove();
	return text(copy);
}

const primaries = (root: ParentNode = document.body) =>
	root.querySelectorAll("[data-dv-primary]").length;

const block = (id: string) => {
	const found = document.getElementById(id);
	if (!found) throw new Error(`no block #${id}`);
	return found;
};

/* The frame's part: a draft in state, the plan and its check derived from the model. */

interface Sink {
	draft?: DeployDraft;
	steps: DeployStepId[];
	results: DeployResult[];
	updates: number;
	/** Device id → what it said about the copy, as the frame would receive it. */
	checks: Record<string, DeployDeviceCheck>;
	/** How often a step asked the frame to start over. */
	startOvers: number;
}

let deployments = 0;

function draftOf(
	app: PlanApp,
	route: Partial<PlanEntry["route"]> & { deviceIds: string[] },
	change: Partial<DeployDraft> = {},
	extra: Partial<PlanEntry> = {},
): DeployDraft {
	deployments += 1;
	return {
		...makePlan({
			scope: { kind: "app", appId: app.id },
			route: { appId: app.id, ...route },
			app,
			deploymentId: `dep-step-${deployments}`,
			now: NOW0,
			...extra,
		}),
		...change,
	};
}

interface StageProps {
	app: PlanApp;
	initial: DeployDraft;
	sink: Sink;
	start: DeployStepId;
	prepared?: DeployPrepared | null;
	platform?: "desktop" | "web";
	/** The frame's sentence for the plan's first problem. */
	blockingText?: string;
	footerContainer?: HTMLElement | null;
	embedded?: boolean;
	deployMore?(): void;
}

const STEPS: Partial<
	Record<DeployStepId, (props: Readonly<DeployStepProps>) => unknown>
> = {
	access_cost: AccessCostStep,
	copy_upload: CopyUploadStep,
	review: ReviewStep,
	rollout: RolloutStep,
};

function Stage(stage: StageProps) {
	const { app, initial, sink, start, prepared, platform } = stage;
	const [draft, setDraft] = useState(initial);
	const [step, setStep] = useState(start);
	const facts = useMemo(
		() => ({
			app,
			devices: DEVICES,
			platform: platform ?? ("desktop" as const),
			now: NOW0,
			isAppOwner: true,
		}),
		[app, platform],
	);
	const plan = useMemo(() => resolvePlan(draft, facts), [draft, facts]);
	const check = useMemo(() => checkPlan(plan, facts), [plan, facts]);
	sink.draft = draft;
	const Step = STEPS[step] as (props: Readonly<DeployStepProps>) => never;
	return (
		<div data-stage={step}>
			<Step
				scope={{ kind: "app", appId: app.id }}
				draft={draft}
				plan={plan}
				check={check}
				update={(patch) => {
					sink.updates += 1;
					setDraft((current) => ({ ...current, ...patch }));
				}}
				goTo={(next) => {
					sink.steps.push(next);
					if (STEPS[next]) setStep(next);
				}}
				onFinished={(result) => sink.results.push(result)}
				prepared={prepared}
				blockingText={stage.blockingText}
				footerContainer={stage.footerContainer}
				embedded={stage.embedded}
				deployMore={stage.deployMore}
				reportDeviceCheck={(deviceId, value) => {
					sink.checks[deviceId] = value;
				}}
				startOver={() => {
					sink.startOvers += 1;
				}}
				summaryBar={<span data-stage-summary="" />}
			/>
		</div>
	);
}

const OWNER = {
	role_id: "role_owner",
	role_name: "Owner",
	permissions: 1,
	is_owner: true,
	can_leave: false,
};
const MEMBER = {
	role_id: "role_member",
	role_name: "Member",
	permissions: 0b1_0000_0000,
	is_owner: false,
	can_leave: true,
};
/** The hub answers `is_owner: true` for an Admin too: only the Owner permission names the owner. */
const ADMIN = {
	role_id: "role_admin",
	role_name: "Admin",
	permissions: 0b10,
	is_owner: true,
	can_leave: true,
};

async function mountStage(
	stage: Omit<StageProps, "sink">,
	options: MountDevicesOptions & { role?: typeof OWNER } = {},
) {
	const sink: Sink = {
		steps: [],
		results: [],
		updates: 0,
		checks: {},
		startOvers: 0,
	};
	const { role = OWNER, ...mount } = options;
	const mounted = await mountDevices(<Stage {...stage} sink={sink} />, {
		host: "app",
		search: `id=${stage.app.id}&flow=deploy&mode=new&step=${stage.start}`,
		backend: {
			roleState: { getOwnRole: async () => role },
		} as never,
		...mount,
	});
	return { ...mounted, sink };
}

async function until(done: () => boolean, what: string, rounds = 600) {
	for (let round = 0; round < rounds; round++) {
		if (done()) return;
		await settle();
	}
	throw new Error(
		`Timed out waiting for ${what}. Page: ${text().slice(0, 600)}`,
	);
}

async function bundleOf(
	project: string,
	source: "online" | "offline",
	catalog?: typeof VISITOR_CATALOG,
): Promise<DeployPrepared> {
	const online = source === "online";
	const files = online
		? [`apps/${project}/online-source.json`]
		: [`apps/${project}/manifest.app`, `apps/${project}/data/visits.lance`];
	const artifact = await prepareProjectArtifact(
		project,
		files.map((path) => ({ path, file: new Blob([`{"file":"${path}"}`]) })),
		undefined,
		undefined,
		source,
	);
	return {
		artifact,
		...(online && catalog
			? {
					approved: {
						app: { id: project } as never,
						file: {
							path: `apps/${project}/online-metadata.json`,
							file: new Blob(["{}"]),
						},
						sha256: SHA,
						catalog,
					},
				}
			: {}),
		preparedAt: 0,
	};
}

/** An upload the device holds answers status and abort; one it doesn't know is a coded `failed`, as on the device. */
function heldUpload(agent: FakeAgent, request: Record<string, unknown>) {
	const held = agent.transfers.get(String(request.transfer_id));
	if (!held)
		return {
			state: "rejected" as const,
			result: {
				code: "failed",
				error: "The device does not hold this upload.",
				retryable: false,
			},
		};
	if (request.kind === "abort") held.state = "aborted";
	return { state: "completed" as const, result: { ...held } };
}

/** The device takes a bundle at once and reports a copy's events and settings; `begins` collects the upload ids it began. */
function serveArtifacts(
	agent: FakeAgent,
	begins: string[] = [],
	events: readonly unknown[] = CRM_CATALOG.events,
) {
	agent.handle("artifact", (command, { operationId }) => {
		const request = command.request as Record<string, unknown>;
		if (request.kind === "status" || request.kind === "abort")
			return heldUpload(agent, request);
		if (request.kind === "begin") {
			begins.push(operationId);
			const descriptor = request.descriptor as {
				project_id: string;
				manifest_sha256: string;
				manifest_size: number;
			};
			return {
				state: "completed",
				result: {
					transfer_id: operationId,
					descriptor,
					state: "committed",
					expires_at: agent.now() + 86_400,
					manifest_ready: true,
					file_index: null,
					offset: descriptor.manifest_size,
					complete: true,
					project_path: `/private/projects/${descriptor.project_id}/revisions/${descriptor.manifest_sha256}`,
				},
			};
		}
		if (request.kind === "usage")
			return {
				state: "completed",
				result: {
					device: null,
					project: {
						bytes: { used: 2_040_109_466, max: 17_179_869_184 },
						entries: { used: 40, max: null },
						revisions: { used: 1, max: null },
					},
					revisions: [],
				},
			};
		if (request.kind !== "describe")
			return {
				state: "rejected",
				result: { code: "failed", error: "unknown upload", retryable: true },
			};
		const eventId = request.event_id as string | null;
		const variables = CRM_CATALOG.variables as Record<string, unknown[]>;
		return {
			state: "completed",
			result: {
				project_id: request.project_id,
				revision: request.revision,
				event_id: eventId,
				items: eventId ? [...(variables[eventId] ?? [])] : events,
				next: null,
			},
		};
	});
}

const sent = (fake: FakeWorkspace, deviceId: string, type: string) =>
	fake.api.commands.filter(
		(command) => command[0] === deviceId && command[1] === type,
	);

const grants = (fake: FakeWorkspace) =>
	fake.api.sent("POST", /devices\/[^/]+\/resource-grants$/);

const visitorDraft = (deviceIds: string[], change: Partial<DeployDraft> = {}) =>
	draftOf(
		VISITOR_PLAN_APP,
		{ deviceIds, eventId: "evt_visitor_page" },
		{
			approval: {
				files: "read_only",
				ownerConsent: true,
				models: [],
				maxInstances: 1,
				expiresAt: NOW0 + 30 * 86_400,
			},
			targets: deviceIds.map((deviceId) => ({
				deviceId,
				choices: {},
				serveBoth: [],
				over: { trustAgent: true },
			})),
			...change,
		},
	);

describe("Access & cost (APP §3.10)", () => {
	test("app-cost: who can approve, one approval per device, the sum of the limits and write buffering", async () => {
		const initial = visitorDraft([EDGE, STUDIO], {
			approval: {
				files: "read_write",
				ownerConsent: true,
				models: ["mistral-small-3"],
				maxInstances: 1,
				expiresAt: NOW0 + 30 * 86_400,
			},
			spending: {
				limitMicros: 10_000_000,
				expiresAt: NOW0 + 30 * 86_400,
				consent: true,
			},
		});
		const { fake, sink, container } = await mountStage({
			app: VISITOR_PLAN_APP,
			initial,
			start: "access_cost",
		});
		await until(
			() =>
				block("dp-who").querySelectorAll('[data-state="pass"]').length === 3,
			"the approver checks",
		);
		expect(text(block("dp-who"))).toContain(
			"You own Visitor Check-in (needed for project files)",
		);
		expect(text(block("dp-who"))).toContain("edge-berlin-01 (yours)");
		expect(document.querySelector("[data-gate='role']")).toBeNull();
		expect(text(block("dp-cloud"))).toContain(
			"Creates 2 approvals, one for check-in-page on each device, with the same files, models and end date.",
		);
		expect(text(block("dp-spend"))).toContain(
			"€10.00 on each device · €20.00 at most in total · paid by you · doesn't renew · ends with the approval",
		);
		// One limit per device: the consent names all of them.
		expect(text(block("dp-spend"))).toContain(
			"I pay for model use by these services up to these limits.",
		);

		await clickByText("Different per device");
		expect(
			sink.draft?.targets.map((target) => target.over.spendingLimitMicros),
		).toEqual([10_000_000, 10_000_000]);
		const studioLimit = document.getElementById(`dp-limit-${STUDIO}`);
		if (!studioLimit) throw new Error("no per-device limit field");
		await typeInto(studioLimit, "25.00");
		await act(async () => {
			studioLimit.dispatchEvent(
				new window.FocusEvent("focusout", { bubbles: true }),
			);
		});
		expect(sink.draft?.targets[1]?.over.spendingLimitMicros).toBe(25_000_000);
		expect(text(block("dp-spend"))).toContain("€35.00 at most in total");

		await click(byRole("switch", /Keep accepting changes/));
		expect(sink.draft?.writes).toEqual(DEFAULT_WRITE_BUFFERING);
		expect(text(block("dp-writes"))).toContain("Needs 1 instance.");
		await clickByText("Add table");
		await typeInto(byRole("textbox", "Table 1 name"), "visits");
		await typeInto(byRole("textbox", "Table 1 key column"), "id");
		await clickByText("Add folder");
		await typeInto(byRole("textbox", "Folder 1 path"), "badges");
		expect(sink.draft?.writes).toEqual({
			...DEFAULT_WRITE_BUFFERING,
			tables: [
				{
					purpose: "storage",
					database: "db",
					table: "visits",
					primary_key: "id",
				},
			],
			files: [{ purpose: "files", prefix: "badges" }],
		});
		expect(
			writeBufferingIssues(sink.draft?.writes ?? DEFAULT_WRITE_BUFFERING),
		).toEqual([]);

		expect(primaries()).toBe(0);
		expect(grants(fake)).toEqual([]);
		expect(prose(container)).not.toMatch(MACHINE_WORDS);
	});

	test("no-approve: a Member sees why, gets a request to copy, and nothing can be changed or sent", async () => {
		const { fake, sink } = await mountStage(
			{
				app: VISITOR_PLAN_APP,
				initial: visitorDraft([EDGE]),
				start: "access_cost",
			},
			{ role: MEMBER },
		);
		await until(
			() => document.querySelector("[data-gate='role']") !== null,
			"the approver gate",
		);
		const gate = document.querySelector("[data-gate='role']") as HTMLElement;
		expect(text(gate)).toContain(
			"You can't approve cloud access for this app.",
		);
		expect(text(gate)).toContain("Your role on Visitor Check-in: Member");
		expect(
			queryByRole("button", "Copy a request for the owner"),
		).not.toBeNull();
		expect(text(block("dp-who"))).toContain("Your role: Member");
		expect(block("dp-who").querySelectorAll('[data-state="fail"]').length).toBe(
			2,
		);

		const allow = byRole("button", "Allow Read & write");
		expect(allow.hasAttribute("disabled")).toBe(true);
		const before = sink.updates;
		await click(allow);
		expect(sink.updates).toBe(before);
		expect(grants(fake)).toEqual([]);
	});

	test("an Admin who doesn't own the app may approve, but isn't told they own it and can't allow its files", async () => {
		await mountStage(
			{
				app: VISITOR_PLAN_APP,
				initial: visitorDraft([EDGE], {
					approval: {
						files: "read_only",
						ownerConsent: false,
						models: [],
						maxInstances: 1,
						expiresAt: NOW0 + 30 * 86_400,
					},
				}),
				start: "access_cost",
			},
			{ role: ADMIN },
		);
		await until(
			() => block("dp-who").querySelectorAll('[data-state="fail"]').length > 0,
			"the owner check",
		);
		const who = text(block("dp-who"));
		expect(who).toContain(
			"Only the owner of Visitor Check-in can allow project files",
		);
		expect(who).not.toContain("You own Visitor Check-in");
		expect(block("dp-who").querySelectorAll('[data-state="pass"]').length).toBe(
			2,
		);
		// An Admin with Execute boards can approve: the whole-step gate stays away.
		expect(document.querySelector("[data-gate='role']")).toBeNull();

		const cloud = block("dp-cloud");
		expect(text(cloud)).toContain(
			"Only the owner of Visitor Check-in can allow access to its project files.",
		);
		expect(text(cloud)).not.toContain("I own Visitor Check-in");
		expect(text(cloud)).not.toContain("Tick the box");
		const read = allByRole("button", "Read").find((option) =>
			cloud.contains(option),
		);
		expect(read?.hasAttribute("disabled")).toBe(true);
		// Write buffering needs the files the Admin can't allow: its shortcut is off too.
		expect(
			byRole("button", "Allow Read & write").hasAttribute("disabled"),
		).toBe(true);
	});

	test("updates keep their approval: one line each, from the hub's list or the interim on an older hub", async () => {
		const update = () =>
			draftOf(
				APPS.app_field_notes,
				{ deviceIds: [STUDIO], serviceId: "field-notes" },
				{},
				{ updateEvents: ["evt_notes_http"] },
			);
		const current = await mountStage({
			app: APPS.app_field_notes,
			initial: update(),
			start: "access_cost",
		});
		await until(
			() => /keeps its cloud access \(until/.test(text()),
			"the kept line with its end date",
		);
		expect(text(block("dp-kept"))).toContain(
			"field-notes on studio-mac-mini keeps its cloud access (until",
		);
		expect(document.getElementById("dp-who")).toBeNull();
		expect(document.getElementById("dp-cloud")).toBeNull();
		await current.unmount();

		const older = await mountStage(
			{ app: APPS.app_field_notes, initial: update(), start: "access_cost" },
			{ hubVersion: "old" },
		);
		await until(
			() => document.getElementById("dp-kept") !== null,
			"the kept block",
		);
		await settle();
		expect(text(block("dp-kept"))).toContain(
			"field-notes on studio-mac-mini keeps its cloud access and spending limit.",
		);
		expect(queryByRole("alert")).toBeNull();
		expect(
			older.fake.api.sent("GET", "apps/app_field_notes/device-placements")
				.length,
		).toBeLessThanOrEqual(1);
	});

	test("an approval that already ended is said to have ended, not kept", async () => {
		const served = await createFakeWorkspace();
		const listed = served.hub.appPlacements("app_field_notes");
		served.api.on("GET", "apps/:app/device-placements", () => ({
			...listed,
			placements: listed.placements.map((placement) => ({
				...placement,
				grant: {
					...placement.grant,
					effective_expires_at: listed.server_time - 3_600,
				},
			})),
		}));
		await mountStage(
			{
				app: APPS.app_field_notes,
				initial: draftOf(
					APPS.app_field_notes,
					{ deviceIds: [STUDIO], serviceId: "field-notes" },
					{},
					{ updateEvents: ["evt_notes_http"] },
				),
				start: "access_cost",
			},
			{ fake: served },
		);
		await until(
			() => /its cloud access ended/.test(text()),
			"the ended approval",
		);
		expect(text(block("dp-kept"))).toContain(
			"field-notes on studio-mac-mini: its cloud access ended",
		);
		expect(text(block("dp-kept"))).toContain("the update doesn't renew it.");
		expect(text(block("dp-kept"))).not.toContain("keeps its cloud access");
	});
});

describe("write buffering fields", () => {
	test("names what keeps the selection from being sent", () => {
		expect(writeBufferingIssues(DEFAULT_WRITE_BUFFERING)).toEqual([
			"nothing_selected",
		]);
		expect(
			writeBufferingIssues({
				...DEFAULT_WRITE_BUFFERING,
				tables: [
					{ purpose: "storage", database: "db", table: "a b", primary_key: "" },
					{
						purpose: "storage",
						database: "db",
						table: "a b",
						primary_key: "id",
					},
				],
				files: [{ purpose: "storage", prefix: "db/visits" }],
			}),
		).toEqual(["table_name", "key_column", "table_twice", "folder"]);
	});

	test("off sends null; the instance note names the drop to one instance", async () => {
		const changes: unknown[] = [];
		await mountDevices(
			<WriteBufferingFields
				value={DEFAULT_WRITE_BUFFERING}
				maxInstances={3}
				onChange={(value) => changes.push(value)}
			/>,
		);
		expect(text()).toContain("Max instances drops from 3 to 1.");
		await click(byRole("switch", /Keep accepting changes/));
		expect(changes).toEqual([null]);
	});
});

async function crmStage(options: MountDevicesOptions = {}, prepared = true) {
	const bundle = prepared ? await bundleOf("app_crm_sync", "offline") : null;
	const mounted = await mountStage(
		{
			app: CRM_PLAN_APP,
			initial: draftOf(CRM_PLAN_APP, {
				deviceIds: [EDGE, STUDIO],
				eventId: "evt_crm_webhook",
			}),
			start: "copy_upload",
			prepared: bundle,
			platform: options.platform,
		},
		options,
	);
	return { ...mounted, bundle };
}

describe("Copy & upload (APP §3.11)", () => {
	test("offline-copy: what each device holds, Upload now through the tray, and the device's event check", async () => {
		const served = await createFakeWorkspace();
		serveArtifacts(served.agent(EDGE));
		serveArtifacts(served.agent(STUDIO));
		const { fake, sink } = await crmStage({ fake: served });
		await until(
			() =>
				(text(block("dp-uploads")).match(/used by CRM Sync on/g) ?? [])
					.length === 2,
			"the storage line of both devices",
		);
		const uploads = block("dp-uploads");
		expect(text(uploads)).toContain("2 files");
		expect(text(uploads)).toContain("Nothing of this version");
		// R5: what the devices hold was read over the live connection.
		const stamp = uploads.querySelector("header [data-stamp]");
		expect(stamp?.getAttribute("data-src")).toBe("live");
		expect(text(stamp as HTMLElement)).toMatch(/read /);
		expect(
			allByRole("row").filter((row) => /Not started/.test(text(row))),
		).toHaveLength(2);
		// Every device that reports its storage is named, not only the first.
		expect(text(uploads)).toContain(
			"1.9 GiB of 16.0 GiB used by CRM Sync on edge-berlin-01. 1.9 GiB of 16.0 GiB used by CRM Sync on studio-mac-mini.",
		);
		expect(primaries()).toBe(0);
		expect(sink.checks).toEqual({});

		await clickByText("Upload now");
		await until(
			() => (text(uploads).match(/checked the events/g) ?? []).length === 2,
			"both uploads to finish",
		);
		expect(text(uploads)).toContain(
			"edge-berlin-01 checked the events: CRM webhook can run",
		);
		expect(text(uploads)).toContain("Has all 2 files of this version");
		// Each device's check goes back to the wizard: Where and Settings read it.
		expect(Object.keys(sink.checks).sort()).toEqual([EDGE, STUDIO].sort());
		expect(sink.checks[EDGE]?.refusals).toEqual({});
		expect(sink.checks[EDGE]?.variables).toEqual({
			evt_crm_webhook: CRM_CATALOG.variables.evt_crm_webhook,
		} as never);
		const items = fake.workspace.activity
			.list()
			.filter((item) => item.kind === "upload" && item.state === "done");
		expect(items.map((item) => item.target.deviceId).sort()).toEqual(
			[EDGE, STUDIO].sort(),
		);
		expect(byRole("button", "Upload now").getAttribute("aria-disabled")).toBe(
			"true",
		);
		expect(text(uploads)).toContain("Nothing is left to send.");
	});

	test("an event the device refuses is named with the device's reason and handed back to the wizard", async () => {
		const reason = "the device requires sandboxed services";
		const refusing = CRM_CATALOG.events.map((event) =>
			event.id === "evt_crm_webhook"
				? { ...event, eligible: false, ineligible_reason: reason }
				: event,
		);
		const served = await createFakeWorkspace();
		serveArtifacts(served.agent(EDGE), [], refusing);
		serveArtifacts(served.agent(STUDIO));
		const { sink } = await crmStage({ fake: served });
		await until(
			() => /used by CRM Sync on/.test(text(block("dp-uploads"))),
			"the storage line",
		);
		await clickByText("Upload now");
		await until(
			() => Object.keys(sink.checks).length === 2,
			"both devices to answer",
		);
		expect(text(block("dp-uploads"))).toContain(
			`CRM webhook can't run here: ${reason}`,
		);
		expect(sink.checks[EDGE]?.refusals).toEqual({ evt_crm_webhook: reason });
		expect(sink.checks[EDGE]?.variables).toEqual({});
		expect(sink.checks[STUDIO]?.refusals).toEqual({});
		expect(queryByRole("alert")).toBeNull();
	});

	test("an older agent reports no storage: the room is unknown and no storage read is sent", async () => {
		const { fake } = await crmStage({ agentFeatures: {} });
		await settle();
		await settle();
		expect(text(block("dp-uploads"))).toContain(
			"Room for this copy on the device: unknown.",
		);
		// It can't list its versions, so the page doesn't claim it holds none.
		expect(text(block("dp-uploads"))).not.toContain("Nothing of this version");
		expect(text(block("dp-uploads"))).toContain(
			"Checked when the upload starts",
		);
		const reads = fake.api.commands.filter(
			([, type, command]) =>
				type === "artifact" &&
				(command.request as { kind?: string }).kind === "usage",
		);
		expect(reads).toEqual([]);
		expect(queryByRole("alert")).toBeNull();
	});

	test("an upload begun earlier is paused and resumable, and Abort asks first", async () => {
		const bundle = await bundleOf("app_crm_sync", "offline");
		const { descriptor } = bundle.artifact;
		const stage = await mountStage({
			app: CRM_PLAN_APP,
			initial: draftOf(CRM_PLAN_APP, {
				deviceIds: [EDGE],
				eventId: "evt_crm_webhook",
			}),
			start: "copy_upload",
			prepared: bundle,
		});
		const { fake } = stage;
		const transferId = "0a0b0c0d-0e0f-4101-8203-040506070809";
		fake.agent(EDGE).transfers.set(transferId, {
			transfer_id: transferId,
			descriptor,
			state: "receiving",
			expires_at: NOW0 + 3_600,
			manifest_ready: true,
			file_index: 1,
			offset: 0,
			complete: false,
			project_path: null,
		});
		rememberArtifactTransfer(
			EDGE,
			{
				transfer_id: transferId,
				project_id: descriptor.project_id,
				manifest_sha256: descriptor.manifest_sha256,
				confirmed: true,
			},
			fake.scope,
		);
		await act(async () => {
			await fake.queryClient.invalidateQueries();
		});
		await until(
			() => /Paused/.test(text(block("dp-uploads"))),
			"the paused upload",
		);
		expect(text(block("dp-uploads"))).toContain(
			"Has 1 of 2 files of this version · resumes",
		);
		expect(text(block("dp-uploads"))).toContain("Resumable until");
		expect(queryByRole("button", "Resume")).not.toBeNull();

		const aborts = () =>
			fake.api.commands.filter(
				([, type, command]) =>
					type === "artifact" &&
					(command.request as { kind?: string }).kind === "abort",
			).length;
		await clickByText("Abort…");
		expect(aborts()).toBe(0);
		expect(text()).toContain("Discards the files of this upload");

		await click(byRole("button", "Abort the upload to edge-berlin-01"));
		await until(
			() => !/Paused/.test(text(block("dp-uploads"))),
			"the abort to finish",
		);
		expect(aborts()).toBe(1);
		expect(fake.agent(EDGE).transfers.get(transferId)?.state).toBe("aborted");
		expect(
			pendingArtifactTransfers(EDGE, descriptor.project_id, fake.scope),
		).toEqual([]);
		expect(queryByRole("alert")).toBeNull();
		expect(
			byRole("button", "Upload now").getAttribute("aria-disabled"),
		).toBeNull();
	});

	/** A lost session reconnects after 1 s of real time; here that first retry fires at once. */
	const reconnectAtOnce = (run: () => void, ms: number) => {
		const timer = setTimeout(run, ms <= 1_000 ? 0 : ms);
		return () => clearTimeout(timer);
	};

	/** An upload asked for while the session reconnects is refused with "Connecting to …", so wait for it. */
	const sessionBack = (fake: FakeWorkspace) =>
		until(
			() => fake.workspace.live.state(EDGE).kind === "live",
			"the session to come back",
		);

	async function edgeCopyStage(begins: string[]) {
		const served = await createFakeWorkspace(undefined, {
			workspace: { live: { schedule: reconnectAtOnce } },
		});
		serveArtifacts(served.agent(EDGE), begins);
		const bundle = await bundleOf("app_crm_sync", "offline");
		const stage = await mountStage(
			{
				app: CRM_PLAN_APP,
				initial: draftOf(CRM_PLAN_APP, {
					deviceIds: [EDGE],
					eventId: "evt_crm_webhook",
				}),
				start: "copy_upload",
				prepared: bundle,
			},
			{ fake: served },
		);
		await until(
			() => /used by CRM Sync on/.test(text(block("dp-uploads"))),
			"the storage line",
		);
		const { project_id: projectId } = bundle.artifact.descriptor;
		const remembered = () =>
			pendingArtifactTransfers(EDGE, projectId, stage.fake.scope);
		return { ...stage, bundle, remembered };
	}

	test("a begin that never reached the device stays remembered and is begun again under the same upload", async () => {
		const begins: string[] = [];
		const { fake, remembered } = await edgeCopyStage(begins);
		fake.agent(EDGE).dropNext("artifact");
		await clickByText("Upload now");
		await until(
			() => /Failed/.test(text(block("dp-uploads"))),
			"the lost begin to fail",
		);
		// The device never got it: nothing is paused there, and the step isn't blocked.
		expect(begins).toEqual([]);
		const [lost] = remembered();
		expect(lost?.confirmed).toBe(false);
		expect(text(block("dp-uploads"))).not.toContain("Paused");
		expect(
			byRole("button", "Upload now").getAttribute("aria-disabled"),
		).toBeNull();

		await sessionBack(fake);
		await clickByText("Upload now");
		await until(
			() => /checked the events/.test(text(block("dp-uploads"))),
			"the upload to finish",
		);
		expect(begins).toEqual([lost?.transfer_id ?? ""]);
		expect(remembered()).toEqual([]);
	});

	test("a begin whose reply was lost is read from the device: nothing is sent twice", async () => {
		const begins: string[] = [];
		const { fake, bundle, remembered } = await edgeCopyStage(begins);
		fake.agent(EDGE).dropNext("artifact");
		await clickByText("Upload now");
		await until(
			() => /Failed/.test(text(block("dp-uploads"))),
			"the lost reply to fail",
		);
		const [lost] = remembered();
		const { descriptor } = bundle.artifact;
		// The device did take it; only its answer was lost.
		fake.agent(EDGE).transfers.set(lost?.transfer_id ?? "", {
			transfer_id: lost?.transfer_id ?? "",
			descriptor,
			state: "committed",
			expires_at: NOW0 + 3_600,
			manifest_ready: true,
			file_index: null,
			offset: descriptor.manifest_size,
			complete: true,
			project_path: `/private/projects/${descriptor.project_id}/revisions/${descriptor.manifest_sha256}`,
		});
		await sessionBack(fake);
		await act(async () => {
			await fake.queryClient.invalidateQueries();
		});
		await until(
			() => /Already has this version/.test(text(block("dp-uploads"))),
			"the device's own answer",
		);
		expect(text(block("dp-uploads"))).toContain("Nothing to send");
		expect(byRole("button", "Upload now").getAttribute("aria-disabled")).toBe(
			"true",
		);
		expect(begins).toEqual([]);
	});

	test("local-web: a local-only app on web shows the platform gate and nothing else", async () => {
		await crmStage({ platform: "web" }, false);
		const gate = document.querySelector(
			"[data-gate='platform']",
		) as HTMLElement;
		expect(text(gate)).toContain(
			"Local-only apps deploy from the desktop app.",
		);
		expect(document.getElementById("dp-uploads")).toBeNull();
		expect(queryByRole("button", "Upload now")).toBeNull();
	});

	test("model access is optional and says what local-only apps can't get", async () => {
		const { sink } = await crmStage();
		const models = block("dp-models");
		expect(text(models)).toContain(
			"Local-only apps can't get access to cloud files; only hosted models need the internet.",
		);
		expect(document.getElementById("dp-spend")).toBeNull();
		await click(byRole("switch", "Let the services use hosted models"));
		expect(text(models)).toContain("Creates 2 approvals");
		expect(sink.draft?.approval.files).toBe("none");
	});
});

describe("Review (APP §3.12)", () => {
	test("review-multi: the plan per device, how it's applied, consequences, size, ids and one primary", async () => {
		const bundle = await bundleOf(
			"app_visitor_checkin",
			"online",
			VISITOR_CATALOG,
		);
		const { fake, sink, container } = await mountStage({
			app: VISITOR_PLAN_APP,
			initial: visitorDraft([EDGE, STUDIO]),
			start: "review",
			prepared: bundle,
		});
		const plan = block("dp-plan");
		expect(text(plan)).toContain("2 devices");
		const rows = allByRole("row", undefined, plan).slice(1);
		expect(rows).toHaveLength(2);
		expect(text(rows[0] as HTMLElement)).toContain("New service");
		expect(text(rows[0] as HTMLElement)).toContain("check-in-page");
		expect(text(rows[0] as HTMLElement)).toContain("Check-in page");
		expect(text(rows[0] as HTMLElement)).toContain("127.0.0.1:8080");
		expect(text(rows[0] as HTMLElement)).toContain("Starts after deploy");

		await clickByText("All at once");
		expect(sink.draft?.order).toBe("all");
		await click(byRole("checkbox", /Stop if a device fails/));
		expect(sink.draft?.stopOnFail).toBe(false);
		await click(byRole("switch", "Start after deploy"));
		expect(sink.draft?.start).toBe(false);
		expect(text(plan)).toContain("Stays stopped");

		const before = block("dp-conseq");
		expect(text(before)).toContain(
			"Creates check-in-page on edge-berlin-01 and studio-mac-mini, serving Check-in page.",
		);
		expect(text(before)).toContain("Each gets cloud access until");
		expect(text(before)).toContain("All devices at once.");
		expect(text(block("dp-size"))).toMatch(
			/[\d,]+ of 12,000 bytes · largest: /,
		);

		expect(primaries()).toBe(1);
		const deploy = byRole("button", "Deploy to 2 devices");
		expect(deploy.getAttribute("aria-disabled")).toBeNull();
		expect(prose(container)).not.toMatch(MACHINE_WORDS);
		expect(fake.workspace.activity.runs()).toEqual([]);
	});

	test("a blocked plan keeps the primary visible, names the step, links to it and sends nothing", async () => {
		const bundle = await bundleOf(
			"app_visitor_checkin",
			"online",
			VISITOR_CATALOG,
		);
		const { fake, sink } = await mountStage({
			app: VISITOR_PLAN_APP,
			initial: visitorDraft([EDGE], {
				approval: {
					files: "read_only",
					ownerConsent: false,
					models: [],
					maxInstances: 1,
					expiresAt: NOW0 + 30 * 86_400,
				},
			}),
			start: "review",
			prepared: bundle,
		});
		const deploy = byRole("button", "Deploy Check-in page to edge-berlin-01");
		expect(deploy.getAttribute("aria-disabled")).toBe("true");
		expect(text()).toContain("Access & cost needs your attention first.");
		const commands = fake.api.commands.length;
		await click(deploy);
		expect(fake.workspace.activity.runs()).toEqual([]);
		expect(sink.steps).toEqual([]);
		expect(grants(fake)).toEqual([]);
		expect(sent(fake, EDGE, "apply")).toEqual([]);
		expect(fake.api.commands.length).toBe(commands);
		expect(primaries()).toBe(1);
		await clickByText("Go to Access & cost");
		expect(sink.steps).toEqual(["access_cost"]);
	});

	test("inside a dialog the blocked primary moves to the host footer as a disabled button with the reason and the way to the setting", async () => {
		const footer = document.createElement("footer");
		document.body.append(footer);
		try {
			const { fake, sink, container } = await mountStage({
				app: VISITOR_PLAN_APP,
				initial: visitorDraft([EDGE], {
					approval: {
						files: "read_only",
						ownerConsent: false,
						models: [],
						maxInstances: 1,
						expiresAt: NOW0 + 30 * 86_400,
					},
				}),
				start: "review",
				prepared: await bundleOf(
					"app_visitor_checkin",
					"online",
					VISITOR_CATALOG,
				),
				footerContainer: footer,
			});
			const label = "Deploy Check-in page to edge-berlin-01";
			expect(container.querySelector("[data-deploy-action]")).toBeNull();
			const deploy = byRole("button", label, footer) as HTMLButtonElement;
			expect(deploy.disabled).toBe(true);
			expect(deploy.className).toContain("h-11");
			expect(text(footer)).toContain(
				"Access & cost needs your attention first.",
			);
			await click(byRole("button", "Open required settings", footer));
			expect(sink.steps).toEqual(["access_cost"]);
			await click(deploy);
			expect(fake.workspace.activity.runs()).toEqual([]);
		} finally {
			footer.remove();
		}
	});

	test("inside a dialog the unblocked primary starts the run from the footer", async () => {
		const footer = document.createElement("footer");
		document.body.append(footer);
		try {
			const { fake, sink } = await mountStage({
				app: VISITOR_PLAN_APP,
				initial: visitorDraft([EDGE]),
				start: "review",
				prepared: await bundleOf(
					"app_visitor_checkin",
					"online",
					VISITOR_CATALOG,
				),
				footerContainer: footer,
			});
			serveArtifacts(fake.agent(EDGE));
			const deploy = byRole(
				"button",
				"Deploy Check-in page to edge-berlin-01",
				footer,
			) as HTMLButtonElement;
			expect(deploy.disabled).toBe(false);
			await click(deploy);
			expect(sink.steps).toEqual(["rollout"]);
			expect(footer.querySelector("[data-deploy-action]")).toBeNull();
		} finally {
			footer.remove();
		}
	});

	test("the frame's sentence for the first problem is the reason next to the blocked primary", async () => {
		const reason =
			"Confirm that you own Visitor Check-in and allow these services to read its files.";
		await mountStage({
			app: VISITOR_PLAN_APP,
			initial: visitorDraft([EDGE], {
				approval: {
					files: "read_only",
					ownerConsent: false,
					models: [],
					maxInstances: 1,
					expiresAt: NOW0 + 30 * 86_400,
				},
			}),
			start: "review",
			prepared: await bundleOf(
				"app_visitor_checkin",
				"online",
				VISITOR_CATALOG,
			),
			blockingText: reason,
		});
		const deploy = byRole("button", "Deploy Check-in page to edge-berlin-01");
		expect(deploy.getAttribute("aria-disabled")).toBe("true");
		const described = document.getElementById(
			deploy.getAttribute("aria-describedby") ?? "",
		);
		expect(text(described as HTMLElement)).toContain(reason);
		expect(text()).not.toContain("needs your attention first");
		expect(queryByRole("button", "Go to Access & cost")).not.toBeNull();
	});

	test("update-one: what changes against the service's settings, Safe or Quick, and the two timings", async () => {
		const bundle = await bundleOf("app_field_notes", "online", {
			events: [
				{
					id: "evt_notes_http",
					name: "Notes page",
					event_type: "http",
					event_version: [1, 2, 0],
					board_version: [3, 0, 1],
					hosted: true,
					readiness_kind: "listener",
					rollout_supported: true,
					eligible: true,
				},
			],
			variables: { evt_notes_http: [] },
		} as never);
		const { sink } = await mountStage({
			app: APPS.app_field_notes,
			initial: draftOf(
				APPS.app_field_notes,
				{ deviceIds: [STUDIO], serviceId: "field-notes" },
				{},
				{ updateEvents: ["evt_notes_http"] },
			),
			start: "review",
			prepared: bundle,
		});
		await until(
			() => /settings v9 → v10/.test(text(block("dp-diff"))),
			"the diff against the live settings",
		);
		const diff = block("dp-diff");
		expect(text(diff)).toContain("App version");
		expect(diff.querySelectorAll("[data-k='changed']").length).toBeGreaterThan(
			0,
		);
		const safe = byRole("radio", /Safe update/);
		expect(
			safe.getAttribute("aria-checked") ??
				String((safe as HTMLInputElement).checked),
		).toBe("true");
		expect(text(block("dp-conseq"))).toContain(
			"field-notes on studio-mac-mini switches to the newest version with a safe update.",
		);
		expect(
			byRole("button", "Update field-notes").getAttribute("aria-disabled"),
		).toBeNull();

		await typeInto(byRole("textbox", /Must stay healthy for/), "1");
		expect(text(block("dp-apply"))).toContain("Enter 2 to 60 seconds.");
		expect(
			byRole("button", "Update field-notes").getAttribute("aria-disabled"),
		).toBe("true");
		await typeInto(byRole("textbox", /Must stay healthy for/), "5");

		await click(byRole("radio", /Quick update/));
		expect(sink.draft?.strategy).toBe("quick");
		expect(text(block("dp-conseq"))).toContain(
			"field-notes on studio-mac-mini stops, then starts the newest version.",
		);
		expect(primaries()).toBe(1);
	});

	test("an agent that names no update modes offers only a quick update, with the reason", async () => {
		const bundle = await bundleOf("app_field_notes", "online", {
			events: [
				{
					id: "evt_notes_http",
					name: "Notes page",
					event_type: "http",
					event_version: [1, 2, 0],
					board_version: [3, 0, 1],
					hosted: true,
					eligible: true,
				},
			],
			variables: { evt_notes_http: [] },
		} as never);
		const fakeFirst = await mountStage({
			app: APPS.app_field_notes,
			initial: draftOf(
				APPS.app_field_notes,
				{ deviceIds: [STUDIO], serviceId: "field-notes" },
				{},
				{ updateEvents: ["evt_notes_http"] },
			),
			start: "access_cost",
			prepared: bundle,
		});
		const agent = fakeFirst.fake.agent(STUDIO);
		const current = await fakeFirst.fake.workspace.live.call(STUDIO)({
			type: "placement_configuration",
			placement_id: "field-notes",
		});
		const { rollout_sources: _modes, ...older } = current.result;
		agent.handle("placement_configuration", () => ({
			state: "completed",
			result: older,
		}));
		await act(async () => fakeFirst.sink.steps.push("review"));
		await fakeFirst.rerender(
			<Stage
				app={APPS.app_field_notes}
				initial={fakeFirst.sink.draft as DeployDraft}
				sink={fakeFirst.sink}
				start="review"
				prepared={bundle}
				key="review"
			/>,
		);
		await until(
			() =>
				document.getElementById("dp-strategy") !== null ||
				/Quick update/.test(text()),
			"the strategy choice",
		);
		await until(() => /Safe update isn't available/.test(text()), "the reason");
		expect(text(block("dp-apply"))).toContain(
			"Safe update isn't available: the device agent can't check this kind of app before switching.",
		);
		expect(sent(fakeFirst.fake, STUDIO, "stage_rollout")).toEqual([]);
	});

	const reportDraft = (change: Partial<DeployDraft> = {}) =>
		draftOf(
			VISITOR_PLAN_APP,
			{ deviceIds: [STUDIO], eventId: "evt_visitor_report" },
			{
				approval: {
					files: "read_write",
					ownerConsent: true,
					models: [],
					maxInstances: 1,
					expiresAt: NOW0 + 30 * 86_400,
				},
				...change,
			},
		);

	test("a schedule that moves to a device: when it runs there, that the hub stops running it, and the way back", async () => {
		const bundle = await bundleOf(
			"app_visitor_checkin",
			"online",
			VISITOR_CATALOG,
		);
		await mountStage({
			app: VISITOR_PLAN_APP,
			initial: reportDraft(),
			start: "review",
			prepared: bundle,
		});
		const before = text(block("dp-conseq"));
		expect(before).toContain(
			"Daily visitor report runs on studio-mac-mini at 18:00 every day (Europe/Berlin). First run 18:00 GMT+2 · in 4 hr. · 16:00 your time. Missed runs are not made up.",
		);
		expect(before).toContain(
			"The hub stops running Daily visitor report when studio-mac-mini starts it.",
		);
		expect(before).toContain(
			"Remove it from the service, or choose Run it on the hub again in Events: the hub runs it again a few minutes later.",
		);
		// Read-write access and no spending limit: neither of the two warnings applies.
		expect(before).not.toContain("read-only");
		expect(before).not.toContain("Each run can use hosted models");
		expect(before).not.toContain("falls into the switch");
	});

	test("a schedule with read-only cloud access or a spending limit says what that means for its runs", async () => {
		const bundle = await bundleOf(
			"app_visitor_checkin",
			"online",
			VISITOR_CATALOG,
		);
		const base = reportDraft();
		await mountStage({
			app: VISITOR_PLAN_APP,
			initial: {
				...base,
				approval: { ...base.approval, files: "read_only" },
				spending: {
					limitMicros: 25_000_000,
					expiresAt: NOW0 + 30 * 86_400,
					consent: true,
				},
			},
			start: "review",
			prepared: bundle,
		});
		const before = text(block("dp-conseq"));
		expect(before).toContain(
			"daily-visitor-report's cloud access is read-only. Runs that change the app's data will fail.",
		);
		expect(before).toContain(
			"Each run can use hosted models within daily-visitor-report's spending limit. When the limit is used up, this service's other events lose hosted models too.",
		);
	});

	test("a device-created schedule does not claim the hub stops or resumes it", async () => {
		const app = {
			...VISITOR_PLAN_APP,
			events: VISITOR_PLAN_APP.events.map((event) =>
				event.id === "evt_visitor_report"
					? {
							...event,
							...withDeviceEventSource({ config: [...(event.config ?? [])] }),
						}
					: event,
			),
		};
		await mountStage({
			app,
			initial: reportDraft(),
			start: "review",
			prepared: await bundleOf(
				"app_visitor_checkin",
				"online",
				VISITOR_CATALOG,
			),
		});
		const before = text(block("dp-conseq"));
		expect(before).toContain("Daily visitor report runs on studio-mac-mini");
		expect(before).not.toContain("The hub stops running");
		expect(before).not.toContain("the hub runs it again");
		expect(before).toContain(
			"Remove it from the service to stop this schedule. Its event stays in the app.",
		);
	});

	test("a flow version the preparation created is said before anything is uploaded, with what can't be undone", async () => {
		const bundle = await bundleOf(
			"app_visitor_checkin",
			"online",
			VISITOR_CATALOG,
		);
		await mountStage({
			app: VISITOR_PLAN_APP,
			initial: visitorDraft([EDGE]),
			start: "review",
			prepared: {
				...bundle,
				flows: [
					{ boardId: "flow_main", version: [0, 4, 1], created: true },
					{ boardId: "flow_report", version: [0, 2, 0], created: false },
				],
				latest: { evt_visitor_page: [0, 4, 1] },
			},
		});
		const before = text(block("dp-conseq"));
		expect(before).toContain(
			"Created flow version 0.4.1 of Main flow from the current edits. The event keeps following Latest.",
		);
		expect(before).toContain(
			"A created flow version stays in the flow's history.",
		);
		// A flow that already had a version equal to it created nothing.
		expect(before).not.toContain("Report flow");
	});

	describe("Endpoints, forms, one-time schedules and bots (R2 §6.4)", () => {
		const SHOP = APPS.app_shop_assistant;
		const shopDraft = (events: string[]) =>
			draftOf(
				SHOP,
				{ deviceIds: [EDGE] },
				{
					scope: "events",
					events,
					approval: {
						files: "read_only",
						ownerConsent: true,
						models: [],
						maxInstances: 1,
						expiresAt: NOW0 + 30 * 86_400,
					},
					targets: [
						{
							deviceId: EDGE,
							choices: {},
							serveBoth: [],
							over: { trustAgent: true },
						},
					],
				},
			);

		test.each(["Private", "Offline"] as const)(
			"a device-created endpoint does not claim its %s source keeps answering",
			async (visibility) => {
				const app = {
					...SHOP,
					visibility,
					events: SHOP.events.map((event) =>
						event.id === "evt_shop_orders"
							? {
									...event,
									...withDeviceEventSource({
										config: [...(event.config ?? [])],
									}),
								}
							: event,
					),
				};
				await mountStage({
					app,
					initial: shopDraft(["evt_shop_orders"]),
					start: "review",
				});
				const before = text(block("dp-conseq"));
				expect(before).toContain(
					"Orders answers GET http://127.0.0.1:8080/orders.",
				);
				expect(before).not.toContain("The hub keeps answering");
				expect(before).not.toContain("This computer keeps answering");
			},
		);

		test("an Endpoint, a form and a bot: where it answers, who may call or run it, and how the bot is undone", async () => {
			await mountStage({
				app: SHOP,
				initial: shopDraft([
					"evt_shop_orders",
					"evt_shop_return",
					"evt_shop_telegram",
				]),
				start: "review",
			});
			const before = text(block("dp-conseq"));
			expect(before).toContain(
				"Orders answers GET http://127.0.0.1:8080/orders.",
			);
			expect(before).toContain(
				"Callers need shop-assistant's access token. The token set in Events is not used on a device.",
			);
			expect(before).toContain(
				"A failed run answers 502, and a run is stopped at shop-assistant's time limit (300 s).",
			);
			expect(before).toContain(
				"The hub keeps answering Orders at its own address.",
			);
			expect(before).toContain(
				"Return request can be run from Devices by people who may start shop-assistant.",
			);
			expect(before).toContain(
				"Anyone with shop-assistant's access token can also run it from the service page.",
			);
			expect(before).toContain(
				"Shop helper answers from edge-berlin-01 while shop-assistant runs.",
			);
			expect(before).toContain(
				"edge-berlin-01 removes the bot's Telegram webhook when it first connects.",
			);
			expect(before).toContain(
				"Remove Shop helper from the service, or take it back in Events. Nothing else runs it afterwards until you start it somewhere.",
			);
			// One served event: nothing else shares its token.
			expect(before).not.toContain("can call all of its endpoints");
			expect(prose(block("dp-conseq"))).not.toMatch(MACHINE_WORDS);
		});

		test("a Discord bot's gap and a one-time schedule's single run are said before deploying", async () => {
			await mountStage({
				app: SHOP,
				initial: shopDraft(["evt_shop_discord", "evt_shop_prices"]),
				start: "review",
			});
			const before = text(block("dp-conseq"));
			expect(before).toContain(
				"Discord messages sent while shop-assistant restarts or updates are not answered.",
			);
			expect(before).toContain(
				"Price update runs once on edge-berlin-01 at 2026-10-15 09:00 (Europe/Berlin). If edge-berlin-01 isn't running then, or within 15 minutes after, it doesn't run at all.",
			);
			expect(before).toContain("After that nothing runs it again.");
			// A one-time schedule has no repeating times to describe.
			expect(before).not.toContain("Missed runs are not made up");
		});
	});
});

async function deployStage(
	deviceIds: string[],
	prepare?: (fake: FakeWorkspace) => void,
	extra: Partial<StageProps> = {},
) {
	const bundle = await bundleOf(
		"app_visitor_checkin",
		"online",
		VISITOR_CATALOG,
	);
	const stage = await mountStage({
		app: VISITOR_PLAN_APP,
		initial: visitorDraft(deviceIds),
		start: "review",
		prepared: bundle,
		...extra,
	});
	for (const deviceId of deviceIds) serveArtifacts(stage.fake.agent(deviceId));
	prepare?.(stage.fake);
	const label =
		deviceIds.length > 1
			? `Deploy to ${deviceIds.length} devices`
			: "Deploy Check-in page to edge-berlin-01";
	await click(byRole("button", label));
	expect(stage.sink.steps).toEqual(["rollout"]);
	return stage;
}

const fleet = () =>
	document.querySelector("[data-fleet-rollout]") as HTMLElement;

describe("Rollout (APP §3.13, §7.8)", () => {
	test("opening Rollout without a deploy starts nothing", async () => {
		const { fake, sink } = await mountStage({
			app: VISITOR_PLAN_APP,
			initial: visitorDraft([EDGE]),
			start: "rollout",
		});
		expect(text()).toContain("Nothing has been deployed yet");
		expect(fake.workspace.activity.runs()).toEqual([]);
		expect(sent(fake, EDGE, "apply")).toEqual([]);
		expect(primaries()).toBe(0);
		await clickByText("Go to Review");
		expect(sink.steps).toEqual(["review"]);
	});

	test("multi-running: the shared phase, the active device with its phase, the next one in line", async () => {
		let release = () => {};
		const { sink } = await deployStage([EDGE, STUDIO], (fake) => {
			release = fake.agent(EDGE).hold("set_secret");
		});
		await until(
			() => /Saving 1 secret\b/.test(text(fleet())),
			"the secrets phase",
		);
		const board = fleet();
		expect(text(board)).toContain("Deploy Check-in page to 2 devices");
		expect(text(board)).toContain(
			"Visitor Check-in · runs online · one device at a time",
		);
		expect(text(board)).toContain("Approving definitions");
		expect(text(board)).toContain("In line after edge-berlin-01");
		expect(text(board)).toContain("tracked on this computer");
		expect(text()).toContain("Deploying Check-in page on 2 devices.");
		expect(text()).toContain("0 of 2 done.");
		// The foot names the run like its tray item and opens the tray (R9).
		expect(text()).toContain("Deploy Check-in page to 2 devices · 0 of 2 done");
		expect(useActivityTray.getState().open).toBe(false);
		await clickByText("Follow in activity");
		expect(useActivityTray.getState().open).toBe(true);
		useActivityTray.getState().setOpen(false);
		expect(sink.results).toEqual([]);
		expect(primaries()).toBe(0);
		release();
		await until(() => sink.results.length === 1, "the run to finish");
	});

	test("multi-done: the result, where it runs, tokens shown once and the way on", async () => {
		const { fake, sink, navigations, container } = await deployStage([
			EDGE,
			STUDIO,
		]);
		await until(() => sink.results.length === 1, "the run to finish");
		expect(sink.results[0]?.outcome).toBe("all");
		const board = fleet();
		expect(text(board)).toMatch(/Deployed to both devices at \d/);
		expect(text(board)).toMatch(/Deployed at .* · check-in-page running/);
		expect(text()).toContain("Done on both devices.");
		expect(text()).toContain(
			"check-in-page runs on edge-berlin-01 and studio-mac-mini.",
		);
		const tokens = block("dp-tokens");
		expect(
			allByRole("button", undefined, tokens).map((b) =>
				b.getAttribute("aria-label"),
			),
		).toEqual([
			"Copy the access token of check-in-page on edge-berlin-01",
			"Copy the access token of check-in-page on studio-mac-mini",
		]);
		expect(text(tokens)).toContain("Access tokens · shown once");
		expect(primaries()).toBe(1);
		const open = byRole("link", "Open in Devices");
		expect(open.hasAttribute("data-dv-primary")).toBe(true);
		expect(open.getAttribute("href")).toContain("by=event");
		expect(open.getAttribute("href")).toContain("event=evt_visitor_page");
		expect(queryByRole("link", "Exit deploy")).not.toBeNull();
		await clickByText("Deploy to more devices…");
		expect(navigations.at(-1)?.href).toContain("step=where");
		expect(fake.workspace.activity.runs()).toHaveLength(1);
		expect(prose(container)).not.toMatch(MACHINE_WORDS);
		expect(sink.results).toHaveLength(1);
	});

	test("embedded in a dialog the result has no Exit deploy, and Deploy to more devices asks the frame instead of the route", async () => {
		let more = 0;
		const { navigations, sink } = await deployStage([EDGE, STUDIO], undefined, {
			embedded: true,
			deployMore: () => {
				more += 1;
			},
		});
		await until(() => sink.results.length === 1, "the run to finish");
		expect(queryByRole("link", "Exit deploy")).toBeNull();
		await clickByText("Deploy to more devices…");
		await clickByText("Deploy to more devices…");
		expect(more).toBe(2);
		expect(navigations.some((entry) => entry.href.includes("step=where"))).toBe(
			false,
		);
	});

	test("shown once: an access token leaves this window with the result that showed it", async () => {
		const stage = await deployStage([EDGE]);
		const { sink } = stage;
		await until(() => sink.results.length === 1, "the run to finish");
		const shown = block("dp-tokens").querySelector("code")?.textContent ?? "";
		expect(shown).toMatch(/^[\x21-\x7e]{10}…$/);
		expect(
			queryByRole(
				"button",
				"Copy the access token of check-in-page on edge-berlin-01",
			),
		).not.toBeNull();

		// The result goes away (another step, another screen) and comes back.
		await stage.rerender(<div />);
		await settle();
		await stage.rerender(
			<Stage
				app={VISITOR_PLAN_APP}
				initial={sink.draft as DeployDraft}
				sink={sink}
				start="rollout"
			/>,
		);
		await until(
			() => document.getElementById("dp-tokens") !== null,
			"the result again",
		);
		const tokens = block("dp-tokens");
		expect(text(tokens)).toContain("edge-berlin-01 › check-in-page");
		expect(text(tokens)).toContain(
			"Shown once · not kept after you left this page",
		);
		expect(tokens.querySelector("code")).toBeNull();
		expect(text()).not.toContain(shown.slice(0, 10));
		expect(allByRole("button", undefined, tokens)).toEqual([]);
		// It is the same finished run, not a new one.
		expect(text()).toContain("Done on edge-berlin-01.");
		expect(stage.fake.workspace.activity.runs()).toHaveLength(1);
	});

	test("multi-partial: the failed device says what stays on the hub and a retry finishes it", async () => {
		let restore = () => {};
		const { fake, sink } = await deployStage([EDGE, STUDIO], (fakes) => {
			restore = fakes
				.agent(STUDIO)
				.reject("apply", "invalid", "The device refused these settings.");
		});
		await until(() => sink.results.length === 1, "the run to finish");
		expect(sink.results[0]?.outcome).toBe("partial");
		const board = fleet();
		expect(text(board)).toContain("Failed on 1");
		expect(text(board)).toContain(
			"Failed while creating the service: the device refused these settings. Its access stays on the hub, so a retry reuses it. Nothing else changed on studio-mac-mini.",
		);
		expect(text(board)).toContain(
			"Deployed to 1 of 2 devices. studio-mac-mini failed while creating the service: the device refused these settings.",
		);
		expect(text()).toContain(
			"Retrying studio-mac-mini resumes from the step that failed and reuses its access on the hub.",
		);
		expect(text(board)).not.toContain("The device refused these settings.");
		expect(text()).toContain("Rollout");
		expect(text()).toContain("Deployed to 1 of 2 devices.");
		expect(text()).toContain("1 of 2 failed.");
		expect(primaries()).toBe(1);
		const retry = byRole("button", "Retry studio-mac-mini");
		expect(retry.hasAttribute("data-dv-primary")).toBe(true);
		expect(block("dp-tokens").querySelectorAll("li")).toHaveLength(1);

		restore();
		await click(retry);
		await until(() => sink.results.length === 2, "the retry to finish");
		expect(sink.results[1]?.outcome).toBe("all");
		expect(
			fake.api.sent("POST", `devices/${STUDIO}/resource-grants`),
		).toHaveLength(1);
	});

	test("deploy-failed and deploy-done on one device: steps, diagnostics, retry and the service link", async () => {
		let restore = () => {};
		const { sink } = await deployStage([EDGE], (fake) => {
			restore = fake
				.agent(EDGE)
				.reject("apply", "limit", "Too many services on this device.");
		});
		await until(() => sink.results.length === 1, "the failure");
		expect(sink.results[0]?.outcome).toBe("none");
		expect(text()).toContain("Check-in page wasn't deployed.");
		expect(text()).toContain("Nothing was deployed.");
		// One device: its own block replaces the board.
		expect(fleet()).toBeNull();
		const steps = block("dp-run");
		expect(text(steps)).toContain("Deploy to edge-berlin-01");
		expect(text(steps)).toContain("Failed");
		expect(text(byRole("alert", undefined, steps))).toBe(
			"Failed while creating the service: a limit on the device was reached. Its access stays on the hub, so a retry reuses it. Nothing else changed on edge-berlin-01.",
		);
		expect(text(steps)).toContain("Approving definitions");
		expect(steps.querySelectorAll("[data-state='fail']")).toHaveLength(1);
		expect(text(steps)).toContain("tracked on this computer");
		expect(queryByRole("button", "Copy diagnostics")).not.toBeNull();
		expect(queryByRole("button", "Start over…")).not.toBeNull();
		expect(queryByRole("button", "Change and deploy again")).not.toBeNull();
		expect(queryByRole("button", "Skip")).toBeNull();
		expect(primaries()).toBe(1);
		const deployed = document.querySelector("[data-deployed]") as HTMLElement;
		expect(text(deployed)).toContain("What you deployed");
		expect(text(deployed)).toContain("check-in-page · new on edge-berlin-01");

		// The frame's "This deploy" line sits right above the run's foot; the status also has a form for narrow widths.
		const foot = document.querySelector("[data-wizard-foot]") as HTMLElement;
		expect(
			foot.previousElementSibling?.hasAttribute("data-stage-summary"),
		).toBe(true);
		const narrow = foot.querySelector("output[data-run-foot-status]");
		expect(text(narrow as HTMLElement)).toBe("Nothing was deployed.");
		// "Start over…" is the frame's: it asks first and resets; the step alone doesn't just jump to What.
		await click(byRole("button", "Start over…"));
		expect(sink.startOvers).toBe(1);
		expect(sink.steps).toEqual(["rollout"]);

		restore();
		await click(byRole("button", "Retry"));
		await until(() => sink.results.length === 2, "the retry to finish");
		expect(text()).toContain("Check-in page is deployed.");
		expect(
			steps.querySelectorAll("[data-state='pass']").length,
		).toBeGreaterThan(3);
		expect(text(byRole("status", undefined, steps))).toMatch(
			/^Deployed at .* · check-in-page running$/,
		);
		expect(text(steps)).toContain(
			"The service page shows each step in its update history.",
		);
		const open = byRole("link", /Open check-in-page/);
		expect(open.hasAttribute("data-dv-primary")).toBe(true);
		expect(open.getAttribute("href")).toContain("service=check-in-page");
		expect(primaries()).toBe(1);
	});

	test("deploy-running on one device: a locked device waits for the unlock, with the way to unlock", async () => {
		let release = () => {};
		const { fake, sink } = await deployStage([EDGE], (fakes) => {
			release = fakes.agent(EDGE).hold("set_secret");
		});
		await until(
			() => sent(fake, EDGE, "set_secret").length === 1,
			"the secret to leave",
		);
		expect(text()).toContain("Deploying Check-in page to edge-berlin-01.");
		await act(async () => fake.workspace.keys.lock(EDGE));
		release();
		await until(
			() =>
				/Waiting: unlock edge-berlin-01 to continue\./.test(
					text(block("dp-run")),
				),
			"the blocked device",
		);
		expect(queryByRole("button", "Unlock…")).not.toBeNull();
		expect(queryByRole("button", "Follow in activity")).not.toBeNull();
		expect(sink.results).toEqual([]);
		await act(async () => {
			await fake.unlock(EDGE, { connectLive: true });
		});
		await until(() => sink.results.length === 1, "the run to finish");
		expect(sink.results[0]?.outcome).toBe("all");
	});
});
