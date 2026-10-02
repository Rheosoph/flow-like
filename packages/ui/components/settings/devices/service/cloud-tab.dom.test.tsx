import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { ApiResponseError } from "../../../../lib/api-error";
import type { MountCloudOptions } from "../cloud/cloud-test-kit";
import {
	advance,
	byRole,
	click,
	inPortal,
	installDom,
	queryByRole,
	typeInto,
} from "../testing/dom-harness";
import type { FakeWorkspace } from "../testing/fake-workspace";

const dom = installDom();
const { cleanupDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { SAMPLE_APPS, SAMPLE_IDS, SAMPLE_PEOPLE } = await import(
	"../../../../lib/device-management/model/__fixtures__/sample-fleet"
);
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const {
	ADMIN_ROLE,
	MACHINE_WORDS,
	MEMBER_ROLE,
	OWNER_ROLE,
	cloudWrites,
	copyOf,
	mountCloud,
	primaries,
	textOf,
} = await import("../cloud/cloud-test-kit");
const { useDevicesRoute } = await import("../routing/use-devices-route");
const { ServiceCloudTab } = await import("./cloud-tab");

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

const { edge: EDGE, studio: STUDIO } = SAMPLE_IDS;
const EDGE_GRANT = "d99ba88b-717b-445e-a719-a2084df3aec0";
const EDGE_LIMIT = "b936e936-d443-4252-a8e6-9420d361c037";
const EDGE_ENDS = 1_793_275_200;

const APP_MODELS = {
	[SAMPLE_APPS.invoiceAi]: ["gpt-4.1-mini", "bge-m3", "mistral-small-3"],
	[SAMPLE_APPS.supportPortal]: ["gpt-4.1-mini"],
};

function Tab({
	deviceId,
	serviceId,
}: Readonly<{ deviceId: string; serviceId: string }>) {
	const { route, scope } = useDevicesRoute();
	return (
		<ServiceCloudTab
			route={route}
			scope={scope}
			deviceId={deviceId}
			serviceId={serviceId}
		/>
	);
}

interface OpenOptions extends MountCloudOptions {
	/** Changes to the fake hub before the tab reads it. */
	arrange?(fake: FakeWorkspace): void;
}

async function open(
	deviceId: string,
	serviceId: string,
	{ arrange, ...options }: OpenOptions = {},
) {
	const fake = await createFakeWorkspace(undefined, options);
	arrange?.(fake);
	const view = await mountCloud(
		<Tab deviceId={deviceId} serviceId={serviceId} />,
		{
			roles: {
				[SAMPLE_APPS.invoiceAi]: OWNER_ROLE,
				[SAMPLE_APPS.fieldNotes]: OWNER_ROLE,
			},
			appModels: APP_MODELS,
			...options,
			fake,
			search: `device=${deviceId}&service=${serviceId}&tab=cloud`,
		},
	);
	await view.settle();
	return view;
}

const act = (root: ParentNode, name: string) =>
	root.querySelector<HTMLElement>(`[data-act="${name}"]`) as HTMLElement;

const block = (root: ParentNode, id: string) =>
	root.querySelector<HTMLElement>(`#${id}`) as HTMLElement;

const noApprovals = (fake: FakeWorkspace) => {
	const entry = fake.hub.resourcesOf(EDGE);
	entry.grants.length = 0;
	entry.billing.length = 0;
	entry.instances.length = 0;
};

describe("Service › Cloud access", () => {
	test("shows the approval, what it allows, its spending limit and its leases", async () => {
		const view = await open(EDGE, "invoice-extractor");
		const { container } = view;
		expect(
			container.querySelector('[data-service-cloud="active"]'),
		).not.toBeNull();

		const approval = textOf(block(container, "svc-cloud-approval"));
		expect(approval).toContain("Approval");
		expect(approval).toContain("Active");
		expect(approval).toContain("App Invoice AI");
		expect(approval).toContain("BGE-M3 embeddings, GPT-4.1 mini");
		expect(approval).toContain("Read & write project files");
		expect(approval).toContain("at least this service's instances (1)");
		expect(approval).toContain("Service uses this approval");
		expect(approval).toContain("checked");
		expect(
			container.querySelector("[data-expiry]")?.getAttribute("data-expiry"),
		).toBe("effective");

		const spending = textOf(block(container, "svc-cloud-spending"));
		expect(spending).toContain("€7.41 used · €0.12 reserved · €25.00 limit");
		expect(spending).toContain("not recurring");
		expect(spending).toContain("1 request · last");
		expect(spending).toContain("Raising the limit means replacing it");

		const leases = block(container, "svc-cloud-leases");
		expect(leases.querySelectorAll("[data-lease]").length).toBe(1);
		expect(textOf(leases)).toContain("Service instance");
		expect(textOf(leases)).toContain("It isn't proof the instance is healthy.");

		expect(queryByRole("alert", undefined, container)).toBeNull();
		expect(copyOf(container)).not.toMatch(MACHINE_WORDS);
		expect(primaries(container)).toBeLessThanOrEqual(1);
		expect(cloudWrites(view.fake)).toEqual([]);
	});

	test("the app name links to App › Devices", async () => {
		const view = await open(EDGE, "invoice-extractor");
		const app = [...view.container.querySelectorAll("a")].find(
			(link) => link.textContent === "App Invoice AI",
		) as HTMLAnchorElement;
		expect(app.getAttribute("href")).toBe(
			`/library/config/devices?id=${SAMPLE_APPS.invoiceAi}`,
		);
	});

	test("Revoke approval… states what happens; Cancel sends nothing, confirming revokes it", async () => {
		const view = await open(EDGE, "invoice-extractor");
		const { container, fake } = view;
		await click(act(container, "revoke-approval"));
		let sheet = inPortal("alertdialog");
		expect(textOf(sheet)).toContain(
			"Revoke the cloud access of invoice-extractor?",
		);
		expect(textOf(sheet)).toContain(
			"invoice-extractor's instances lose cloud access within minutes.",
		);
		expect(textOf(sheet)).toContain(
			"Model calls and project-file access fail; buffered writes pause and are kept.",
		);
		expect(textOf(sheet)).toContain("stay valid for up to 10 minutes");
		expect(textOf(sheet)).toContain("Approve again");
		await click(byRole("button", "Cancel", sheet));
		await view.settle();
		expect(cloudWrites(fake)).toEqual([]);

		await click(act(container, "revoke-approval"));
		sheet = inPortal("alertdialog");
		await click(
			byRole("button", "Revoke the cloud access of invoice-extractor", sheet),
		);
		await view.settle();
		expect(cloudWrites(fake).map(([method, path]) => [method, path])).toEqual([
			["DELETE", `devices/${EDGE}/resource-grants/${EDGE_GRANT}`],
		]);
		expect(textOf(container)).toContain(
			"The cloud access of invoice-extractor was revoked at",
		);
		expect(
			container.querySelector('[data-service-cloud="revoked"]'),
		).not.toBeNull();
		expect(textOf(container)).toContain(
			"invoice-extractor is bound to a revoked approval.",
		);
		expect(act(container, "approve-open")).not.toBeNull();
		expect(act(container, "revoke-approval")).toBeNull();
	});

	test("someone else's spending limit can't be revoked: the reason names who pays, a click sends nothing", async () => {
		const view = await open(EDGE, "invoice-extractor", {
			arrange: (fake) => {
				const limit = fake.hub.resourcesOf(EDGE).billing[0];
				if (limit) limit.payer_id = SAMPLE_PEOPLE.mira;
			},
		});
		const { container, fake } = view;
		const revoke = act(container, "revoke-limit");
		expect(revoke.getAttribute("aria-disabled")).toBe("true");
		expect(textOf(block(container, "svc-cloud-spending"))).toContain(
			"Only Mira Novak, who pays, can revoke this limit.",
		);
		await click(revoke);
		expect(queryByRole("alertdialog", undefined, document.body)).toBeNull();
		expect(cloudWrites(fake)).toEqual([]);
	});

	test("Replace spending limit…: the consent is needed, then a review, then the old limit closes and the new one starts", async () => {
		const view = await open(EDGE, "invoice-extractor");
		const { container, fake } = view;
		await click(act(container, "limit-open"));
		await view.settle();
		const form = container.querySelector("[data-limit-form]") as HTMLElement;
		expect(textOf(form)).toContain(
			"The current limit (€7.41 of €25.00 used) closes",
		);
		expect(textOf(form)).toContain("Your PRO plan covers these models.");
		expect(act(form, "limit-go").getAttribute("aria-disabled")).toBe("true");
		expect(textOf(form)).toContain("Tick the box to accept the charges.");
		await click(act(form, "limit-go"));
		expect(queryByRole("alertdialog", undefined, document.body)).toBeNull();

		await typeInto(form.querySelector("input") as HTMLInputElement, "40");
		await click(byRole("checkbox", /I pay for model use/, form));
		expect(textOf(form)).toContain("€40.00 at most · paid by you");
		expect(act(form, "limit-go").getAttribute("aria-disabled")).toBeNull();
		await click(act(form, "limit-go"));

		const sheet = inPortal("alertdialog");
		expect(textOf(sheet)).toContain(
			"Replace the spending limit of invoice-extractor?",
		);
		expect(textOf(sheet)).toContain("charged to you, up to €40.00");
		expect(textOf(sheet)).toContain("Not recurring");
		expect(cloudWrites(fake)).toEqual([]);
		await click(byRole("button", "Continue", sheet));
		await click(
			byRole(
				"button",
				"Replace the spending limit of invoice-extractor",
				inPortal("alertdialog"),
			),
		);
		await view.settle();

		expect(cloudWrites(fake)).toEqual([
			["DELETE", `devices/${EDGE}/billing-grants/${EDGE_LIMIT}`, undefined],
			[
				"POST",
				`devices/${EDGE}/resource-grants/${EDGE_GRANT}/billing`,
				{ limit_micros: 40_000_000, expires_at: EDGE_ENDS },
			],
		]);
		const spending = textOf(block(container, "svc-cloud-spending"));
		expect(spending).toContain(
			"A spending limit of €40.00 replaced the old one at",
		);
		expect(spending).toContain("€0.00 used · €40.00 limit");
		expect(container.querySelector("[data-limit-form]")).toBeNull();
	});

	test("buffered changes paused by a changed approval are counted and linked to Write buffering", async () => {
		const view = await open(STUDIO, "field-notes");
		const { container } = view;
		expect(textOf(container)).toContain(
			"3 buffered changes are paused because cloud access changed.",
		);
		expect(act(container, "open-buffering").getAttribute("href")).toContain(
			"tab=offline",
		);
		const spending = block(container, "svc-cloud-spending");
		expect(textOf(spending)).toContain(
			"No spending limit: hosted models refuse field-notes's model calls until someone sets one.",
		);
		expect(
			act(spending, "limit-open").getAttribute("aria-disabled"),
		).toBeNull();
		expect(copyOf(container)).not.toMatch(MACHINE_WORDS);
		expect(primaries(container)).toBeLessThanOrEqual(1);
	});
});

describe("Service › Cloud access: no approval yet", () => {
	test("an offline copy reads Model access and never offers project files", async () => {
		const view = await open(EDGE, "support-bot");
		const { container, fake } = view;
		expect(
			container.querySelector('[data-service-cloud="none"]'),
		).not.toBeNull();
		const said = textOf(container);
		expect(said).toContain("Model access");
		expect(said).toContain("Runs with local resources only");
		expect(said).toContain("No model access is approved for support-bot.");
		expect(said).toContain("Local-only apps can't get access to cloud files.");
		expect(said).not.toContain("Cloud access");
		expect(primaries(container)).toBe(1);

		await click(act(container, "approve-open"));
		await view.settle();
		const form = container.querySelector("[data-approval-form]") as HTMLElement;
		expect(textOf(form)).not.toContain("Project files");
		expect(textOf(form)).toContain("Pick at least one model.");
		expect(act(form, "approve-go").getAttribute("aria-disabled")).toBe("true");
		expect(primaries(container)).toBe(1);
		await click(act(form, "approve-go"));
		expect(cloudWrites(fake)).toEqual([]);

		await click(byRole("checkbox", "GPT-4.1 mini", form));
		expect(form.querySelector("[data-form-spending]")).not.toBeNull();
		expect(textOf(form)).toContain("support-bot may call GPT-4.1 mini.");
		await click(byRole("checkbox", /I pay for model use/, form));
		expect(act(form, "approve-go").getAttribute("aria-disabled")).toBeNull();
		await click(act(form, "approve-go"));
		await view.settle();

		const writes = cloudWrites(fake);
		expect(
			writes.map(([method, path]) => [method, path.split("/")[2]]),
		).toEqual([
			["POST", "resource-grants"],
			["POST", "resource-grants"],
		]);
		const approvalBody = writes[0]?.[2] as Record<string, unknown>;
		expect(approvalBody.placement_id).toBe("support-bot");
		expect(approvalBody.app_id).toBeNull();
		expect(approvalBody.model_ids).toEqual(["gpt-4.1-mini"]);
		expect("online_access" in approvalBody).toBe(false);
		expect(approvalBody.max_instances).toBe(4);
		expect(writes[1]?.[1]).toMatch(/\/billing$/);
		expect((writes[1]?.[2] as Record<string, unknown>).limit_micros).toBe(
			10_000_000,
		);

		expect(textOf(container)).toContain("Approved at");
		expect(textOf(container)).toContain(
			"support-bot uses it once its settings name this approval.",
		);
		expect(
			container.querySelector('[data-service-cloud="active"]'),
		).not.toBeNull();
		expect(container.querySelector('[data-binding="unbound"]')).not.toBeNull();
	});

	test("an online service: the owner approves project files with the consent tick", async () => {
		const view = await open(EDGE, "invoice-extractor", {
			arrange: noApprovals,
		});
		const { container, fake } = view;
		expect(textOf(container)).toContain(
			"No cloud access is approved for invoice-extractor.",
		);
		await click(act(container, "approve-open"));
		await view.settle();
		const form = container.querySelector("[data-approval-form]") as HTMLElement;
		expect(textOf(form)).toContain("Project files");
		expect(textOf(form)).toContain(
			"I own Invoice AI and allow this service to read its files",
		);
		expect(textOf(form)).toContain(
			"Tick the box to allow access to the project files.",
		);
		expect(act(form, "approve-go").getAttribute("aria-disabled")).toBe("true");

		await click(byRole("checkbox", /I own Invoice AI/, form));
		expect(act(form, "approve-go").getAttribute("aria-disabled")).toBeNull();
		await click(act(form, "approve-go"));
		await view.settle();

		const writes = cloudWrites(fake);
		expect(writes.length).toBe(1);
		const body = writes[0]?.[2] as Record<string, unknown>;
		expect(body.online_access).toBe("read_only");
		expect(body.app_id).toBe(SAMPLE_APPS.invoiceAi);
		expect(body.model_ids).toEqual([]);
	});

	test("an admin who isn't the owner can approve models but not project files", async () => {
		const view = await open(EDGE, "invoice-extractor", {
			arrange: noApprovals,
			roles: { [SAMPLE_APPS.invoiceAi]: ADMIN_ROLE },
		});
		const { container } = view;
		expect(
			act(container, "approve-open").getAttribute("aria-disabled"),
		).toBeNull();
		await click(act(container, "approve-open"));
		await view.settle();
		const form = container.querySelector("[data-approval-form]") as HTMLElement;
		expect(textOf(form)).toContain(
			"Only the owner of Invoice AI can allow access to its project files.",
		);
		expect(textOf(form)).not.toContain("I own Invoice AI");
		const read = byRole("button", "Read", form) as HTMLButtonElement;
		expect(read.disabled || read.getAttribute("aria-disabled") === "true").toBe(
			true,
		);
		expect(byRole("button", "None", form).getAttribute("aria-pressed")).toBe(
			"true",
		);
		expect(act(form, "approve-go").getAttribute("aria-disabled")).toBe("true");

		await click(byRole("checkbox", "GPT-4.1 mini", form));
		await click(byRole("checkbox", /I pay for model use/, form));
		expect(act(form, "approve-go").getAttribute("aria-disabled")).toBeNull();
		await click(act(form, "approve-go"));
		await view.settle();
		const body = cloudWrites(view.fake)[0]?.[2] as Record<string, unknown>;
		expect("online_access" in body).toBe(false);
		expect(body.model_ids).toEqual(["gpt-4.1-mini"]);
	});

	test("without Admin or Owner on the app the control is disabled with the reason, and a click sends nothing", async () => {
		const view = await open(EDGE, "invoice-extractor", {
			arrange: noApprovals,
			roles: { [SAMPLE_APPS.invoiceAi]: MEMBER_ROLE },
		});
		const { container, fake } = view;
		const approve = act(container, "approve-open");
		expect(approve.getAttribute("aria-disabled")).toBe("true");
		expect(textOf(container)).toContain(
			"Needs Admin or Owner on the app with Execute boards.",
		);
		expect(primaries(container)).toBe(0);
		await click(approve);
		await view.settle();
		expect(container.querySelector("[data-approval-form]")).toBeNull();
		expect(cloudWrites(fake)).toEqual([]);
	});
});

describe("Service › Cloud access on an older hub or agent", () => {
	test("an older hub: the approved date with a note and totals only, no error, no retry storm", async () => {
		const view = await open(EDGE, "invoice-extractor", { hubVersion: "old" });
		const { container, fake } = view;
		expect(queryByRole("alert", undefined, container)).toBeNull();
		expect(
			container.querySelector("[data-expiry]")?.getAttribute("data-expiry"),
		).toBe("approved");
		expect(textOf(container)).toContain("this hub doesn't report that");
		expect(
			container
				.querySelector("[data-spend-instances]")
				?.getAttribute("data-spend-instances"),
		).toBe("interim");
		expect(textOf(container)).toContain("Totals only.");
		expect(textOf(container)).toContain(
			"€7.41 used · €0.12 reserved · €25.00 limit",
		);
		expect(
			fake.api.sent("GET", /billing-grants\/[^/]+\/usage$/).length,
		).toBeLessThanOrEqual(1);
		expect(
			fake.api.sent("GET", "devices/resource-summary").length,
		).toBeLessThanOrEqual(1);

		await click(act(container, "limit-open"));
		await view.settle();
		expect(textOf(container)).toContain(
			"Whether your plan covers each model is checked when the service first calls it.",
		);
		expect(fake.api.sent("GET", /\/eligibility$/).length).toBeLessThanOrEqual(
			1,
		);
		expect(queryByRole("alert", undefined, container)).toBeNull();
	});

	test("an older agent: the tab reads the settings with the command every agent knows", async () => {
		const fake = await createFakeWorkspace(undefined, { agentFeatures: {} });
		const before = fake.api.commands.length;
		const view = await mountCloud(
			<Tab deviceId={EDGE} serviceId="invoice-extractor" />,
			{
				fake,
				roles: { [SAMPLE_APPS.invoiceAi]: OWNER_ROLE },
				appModels: APP_MODELS,
				search: `device=${EDGE}&service=invoice-extractor&tab=cloud`,
			},
		);
		await view.settle();
		expect(queryByRole("alert", undefined, view.container)).toBeNull();
		expect(textOf(view.container)).toContain("Service uses this approval");
		const sent = new Set(
			fake.api.commands.slice(before).map(([, type]) => type),
		);
		for (const type of sent)
			expect(["placement_configuration", "inspect"]).toContain(type);
	});
});

describe("Service › Cloud access: states", () => {
	test("the hub fails before anything was read: an error with a retry, never 'local resources only'", async () => {
		const view = await open(EDGE, "invoice-extractor", {
			arrange: (fake) => {
				fake.api.fail(
					{ method: "GET", path: /resource-grants/ },
					new ApiResponseError({
						status: 403,
						code: "FORBIDDEN",
						message: "refused",
					}),
				);
			},
		});
		const { container } = view;
		expect(textOf(container)).toContain(
			"Couldn't read the cloud access of this device",
		);
		expect(textOf(container)).toContain(
			"The hub refused this request. Nothing is known about invoice-extractor's cloud access until this can be read.",
		);
		expect(textOf(container)).not.toContain("Runs with local resources only");
		expect(byRole("button", "Try again", container)).not.toBeNull();
	});

	test("a refresh that fails keeps what was read and says so in every block head", async () => {
		const view = await open(EDGE, "invoice-extractor");
		const { container, fake } = view;
		fake.api.fail({
			method: "GET",
			path: /resource-grants|billing-grants|instances/,
		});
		void fake.queryClient.refetchQueries();
		// A hub error is retried twice (after 1 s and 2 s) before it counts.
		await advance(3_400);
		await view.settle();
		for (const id of [
			"svc-cloud-approval",
			"svc-cloud-spending",
			"svc-cloud-leases",
		])
			expect(textOf(block(container, id))).toContain("couldn't refresh");
		expect(textOf(container)).toContain("BGE-M3 embeddings, GPT-4.1 mini");
		expect(textOf(container)).toContain(
			"€7.41 used · €0.12 reserved · €25.00 limit",
		);
		expect(textOf(container)).not.toContain(
			"Couldn't read the cloud access of this device",
		);
	}, 15_000);

	test("project storage is full: read & write is shown as read-only right now", async () => {
		const view = await open(EDGE, "invoice-extractor", {
			arrange: (fake) => {
				const grant = fake.hub.resourcesOf(EDGE).grants[0];
				if (grant) grant.online_write_blocked = "storage_full";
			},
		});
		const blocked = view.container.querySelector(
			"[data-write-blocked]",
		) as HTMLElement;
		expect(textOf(blocked)).toBe(
			"Project storage is full, so the service can only read right now. Free up storage to let it write again.",
		);
	});

	/** The device's access rules ran out ten minutes ago; the approval's own date is weeks away. */
	const endedWithRules = (fake: FakeWorkspace) => {
		const grant = fake.hub.resourcesOf(EDGE).grants[0];
		if (!grant) throw new Error("the sample has an approval on edge");
		grant.effective_expires_at = fake.hub.now() - 600;
		grant.effective_limit = "access_rules";
	};

	test("an approval that ended with the access rules says what resumes it and can still be revoked", async () => {
		const view = await open(EDGE, "invoice-extractor", {
			arrange: endedWithRules,
		});
		const { container } = view;
		expect(
			container.querySelector('[data-service-cloud="expired"]'),
		).not.toBeNull();
		const said = textOf(block(container, "svc-cloud-approval"));
		expect(said).toContain("The approval of invoice-extractor has ended.");
		expect(said).toContain(
			"It resumes when the access rules of edge-berlin-01 are renewed.",
		);
		expect(said).toContain(
			"The access rules of edge-berlin-01 expired or can't be checked.",
		);
		expect(act(container, "open-access").getAttribute("href")).toContain(
			"tab=access",
		);
		expect(
			act(container, "revoke-approval").getAttribute("aria-disabled"),
		).toBeNull();
	});

	test("replacing an approval the hub still holds revokes it first", async () => {
		const view = await open(EDGE, "invoice-extractor", {
			arrange: endedWithRules,
		});
		const { container, fake } = view;
		await click(act(container, "approve-open"));
		await view.settle();
		const form = container.querySelector("[data-approval-form]") as HTMLElement;
		expect(textOf(form)).toContain(
			"The new approval replaces the current one.",
		);
		await click(byRole("checkbox", /I own Invoice AI/, form));
		await click(byRole("checkbox", /I pay for model use/, form));
		await click(act(form, "approve-go"));
		const sheet = inPortal("alertdialog");
		expect(textOf(sheet)).toContain(
			"The current approval of invoice-extractor is revoked and a new one is created.",
		);
		expect(cloudWrites(fake)).toEqual([]);
		await click(
			byRole("button", "Replace the cloud access of invoice-extractor", sheet),
		);
		await view.settle();
		expect(
			cloudWrites(fake).map(([method, path]) => [
				method,
				path.replace(/[0-9a-f-]{36}/g, "id"),
			]),
		).toEqual([
			["DELETE", "devices/id/resource-grants/id"],
			["POST", "devices/id/resource-grants"],
			["POST", "devices/id/resource-grants/id/billing"],
		]);
		expect(cloudWrites(fake)[0]?.[1]).toBe(
			`devices/${EDGE}/resource-grants/${EDGE_GRANT}`,
		);
	});

	test("the hub refuses a revoke: the sentence says so and the approval stays", async () => {
		const view = await open(EDGE, "invoice-extractor");
		const { container, fake } = view;
		fake.api.fail({ method: "DELETE", path: /resource-grants/ });
		await click(act(container, "revoke-approval"));
		await click(
			byRole(
				"button",
				"Revoke the cloud access of invoice-extractor",
				inPortal("alertdialog"),
			),
		);
		await view.settle();
		expect(
			container.querySelector('[data-service-cloud="active"]'),
		).not.toBeNull();
		expect(textOf(container)).not.toContain("was revoked at");
		expect(queryByRole("alert", undefined, container)).not.toBeNull();
	});

	test("a completion that arrives after the tab is gone is dropped", async () => {
		const view = await open(EDGE, "invoice-extractor");
		const { fake } = view;
		const release = fake.api.hold({
			method: "DELETE",
			path: /resource-grants/,
		});
		await click(act(view.container, "revoke-approval"));
		await click(
			byRole(
				"button",
				"Revoke the cloud access of invoice-extractor",
				inPortal("alertdialog"),
			),
		);
		await view.unmount();
		release();
		await fake.settle();
		expect(document.body.textContent ?? "").not.toContain("was revoked at");
	});

	test("a second click while the approval is being created sends nothing more", async () => {
		const view = await open(EDGE, "support-bot");
		const { container, fake } = view;
		await click(act(container, "approve-open"));
		await view.settle();
		const form = container.querySelector("[data-approval-form]") as HTMLElement;
		await click(byRole("checkbox", "GPT-4.1 mini", form));
		await click(byRole("checkbox", /I pay for model use/, form));
		const release = fake.api.hold({
			method: "POST",
			path: /resource-grants$/,
		});
		await click(act(form, "approve-go"));
		await click(act(form, "approve-go"));
		expect(fake.api.sent("POST", /resource-grants$/).length).toBe(1);
		release();
		await view.settle();
		expect(fake.api.sent("POST", /resource-grants$/).length).toBe(1);
		expect(fake.api.sent("POST", /\/billing$/).length).toBe(1);
	});

	test("an answer that got lost: what the hub did create is read again and nothing is sent twice", async () => {
		const view = await open(EDGE, "support-bot", {
			arrange: (fake) => {
				fake.api.on("POST", "devices/:id/resource-grants", (request) => {
					const sent = request.body as Record<string, unknown>;
					fake.hub.resourcesOf(EDGE).grants.push({
						grant_id: "0c0c0c0c-0000-4000-8000-000000000001",
						device_id: EDGE,
						placement_id: String(sent.placement_id),
						deployment_id: String(sent.deployment_id),
						project_id: String(sent.project_id),
						app_id: null,
						delegating_user_id: fake.hub.me,
						authz_version: 1,
						model_ids: sent.model_ids as string[],
						online_access: null,
						max_instances: Number(sent.max_instances),
						expires_at: Number(sent.expires_at),
						status: "active",
					});
					throw new ApiResponseError({
						status: 504,
						code: "GATEWAY_TIMEOUT",
						message: "timed out",
					});
				});
			},
		});
		const { container, fake } = view;
		await click(act(container, "approve-open"));
		await view.settle();
		const form = container.querySelector("[data-approval-form]") as HTMLElement;
		await click(byRole("checkbox", "GPT-4.1 mini", form));
		await click(byRole("checkbox", /I pay for model use/, form));
		await click(act(form, "approve-go"));
		await view.settle();

		expect(fake.api.sent("POST", /resource-grants$/).length).toBe(1);
		expect(fake.api.sent("POST", /\/billing$/).length).toBe(0);
		expect(queryByRole("alert", undefined, container)).not.toBeNull();
		expect(
			container.querySelector('[data-service-cloud="active"]'),
		).not.toBeNull();
		expect(textOf(block(container, "svc-cloud-spending"))).toContain(
			"No spending limit: hosted models refuse support-bot's model calls until someone sets one.",
		);
		expect(act(container, "limit-open")).not.toBeNull();
	});

	test("update checks are told apart from service instances, also for a lease without a purpose", async () => {
		const view = await open(EDGE, "invoice-extractor", {
			arrange: (fake) => {
				const entry = fake.hub.resourcesOf(EDGE);
				const lease = entry.instances[0];
				if (!lease) throw new Error("the sample has a lease on edge");
				const { purpose: _purpose, ...legacy } = lease;
				entry.instances = [
					legacy as typeof lease,
					{
						...lease,
						instance_id: "9419ff95-0000-4000-8000-000000000001",
						purpose: "rollout_validation",
						lease_expires_at: fake.hub.now() - 5,
					},
				];
			},
		});
		const leases = block(view.container, "svc-cloud-leases");
		const rows = [...leases.querySelectorAll("[data-lease]")].map((row) =>
			textOf(row),
		);
		expect(rows.length).toBe(2);
		expect(rows[0]).toContain("Service instance");
		expect(rows[0]).not.toContain("Update check");
		expect(rows[0]).not.toContain("may have expired");
		expect(rows[1]).toContain("Update check");
		expect(rows[1]).not.toContain("Service instance");
		expect(rows[1]).toContain("may have expired");
		expect(textOf(leases)).toContain(
			"An update check doesn't count toward the instances allowed.",
		);
	});

	test("the advanced reference is what a service's settings must name, and Copy copies exactly that", async () => {
		const view = await open(EDGE, "invoice-extractor");
		const reference = view.container.querySelector(
			"[data-approval-reference]",
		) as HTMLElement;
		const shown = JSON.parse(
			reference.querySelector("pre")?.textContent ?? "null",
		);
		expect(shown).toEqual({
			resource_grant: {
				grant_id: EDGE_GRANT,
				authz_version: 1,
				billing_grant_id: EDGE_LIMIT,
				billing_authz_version: 1,
			},
		});
		await click(act(reference, "copy-binding"));
		expect(JSON.parse(dom.clipboard.at(-1) ?? "null")).toEqual(shown);
		expect(textOf(reference)).toContain("Copied");
	});

	test("what this tab reads itself is never written to the persisted cache", async () => {
		const view = await open(EDGE, "invoice-extractor");
		const own = view.fake.queryClient
			.getQueryCache()
			.getAll()
			.filter((query) =>
				["cloud-binding", "model-bit"].some((part) =>
					query.queryKey.includes(part),
				),
			);
		expect(own.length).toBeGreaterThan(1);
		for (const query of own) expect(query.meta?.persist).toBe(false);
	});
});
