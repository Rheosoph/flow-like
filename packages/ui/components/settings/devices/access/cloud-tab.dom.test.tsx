import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { ApiResponseError } from "../../../../lib/api-error";
import {
	byRole,
	click,
	inPortal,
	installDom,
	queryByRole,
} from "../testing/dom-harness";
import type { FakeWorkspaceOptions } from "../testing/fake-workspace";

const dom = installDom();
const { cleanupDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { SAMPLE_APPS, SAMPLE_IDS } = await import(
	"../../../../lib/device-management/model/__fixtures__/sample-fleet"
);
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const { MACHINE_WORDS, cloudWrites, mountCloud, primaries, textOf } =
	await import("../cloud/cloud-test-kit");
const { useDevicesRoute } = await import("../routing/use-devices-route");
const { AccessCloudTab } = await import("./cloud-tab");

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

const { edge: EDGE, studio: STUDIO, partner: PARTNER } = SAMPLE_IDS;
const EDGE_GRANT = "d99ba88b-717b-445e-a719-a2084df3aec0";
const PARTNER_GRANT = "22c49df5-abd8-4df5-9508-0de6a80418ae";
const PARTNER_LIMIT = "cc3e21cf-49b2-4ed9-9eae-a337d7912be3";

function Routed() {
	const { route, scope } = useDevicesRoute();
	return (
		<>
			<p data-screen="">{route.screen}</p>
			<AccessCloudTab route={route} scope={scope} />
		</>
	);
}

async function open(options: FakeWorkspaceOptions = {}) {
	const fake = await createFakeWorkspace(undefined, options);
	const view = await mountCloud(<Routed />, {
		fake,
		search: "view=access&tab=cloud",
	});
	await view.settle();
	return view;
}

const row = (root: ParentNode, grantId: string) =>
	root.querySelector<HTMLElement>(
		`tr[data-approval="${grantId}"]`,
	) as HTMLElement;

const act = (root: ParentNode, name: string) =>
	root.querySelector<HTMLElement>(`[data-act="${name}"]`) as HTMLElement;

describe("Access › Cloud approvals & spending", () => {
	test("lists every approval with models, files, end, status, spending and who pays; one sum line", async () => {
		const view = await open();
		const { container } = view;
		expect(container.querySelectorAll("tr[data-approval]").length).toBe(3);

		const edge = row(container, EDGE_GRANT);
		const said = textOf(edge);
		expect(said).toContain("invoice-extractor");
		expect(said).toContain("edge-berlin-01");
		expect(said).toContain("App Invoice AI");
		expect(said).toContain("BGE-M3 embeddings, GPT-4.1 mini");
		expect(said).toContain("Read & write project files");
		expect(said).toContain("Active");
		expect(said).toContain("€7.41 used · €0.12 reserved · €25.00 limit");
		expect(edge.querySelector("[data-person]")?.textContent).toContain("You");
		expect(
			edge.querySelector("[data-expiry]")?.getAttribute("data-expiry"),
		).toBe("effective");

		const sum = container.querySelector("[data-spend-sum]") as HTMLElement;
		expect(textOf(sum)).toContain(
			"You pay for 2 spending limits · €19.91 used of €75.00 · €0.12 reserved.",
		);
		expect(textOf(sum)).toContain("Not a billing record");

		expect(textOf(container)).toContain("checked");
		expect(textOf(container)).not.toMatch(MACHINE_WORDS);
		expect(primaries(container)).toBeLessThanOrEqual(1);
		expect(cloudWrites(view.fake)).toEqual([]);
	});

	test("an app name links to App › Devices and a service to its Cloud access tab", async () => {
		const view = await open();
		const edge = row(view.container, EDGE_GRANT);
		const app = [...edge.querySelectorAll("a")].find(
			(link) => link.textContent === "App Invoice AI",
		) as HTMLAnchorElement;
		expect(app.getAttribute("href")).toBe(
			`/library/config/devices?id=${SAMPLE_APPS.invoiceAi}`,
		);
		const service = [...edge.querySelectorAll("a")].find(
			(link) => link.textContent === "invoice-extractor",
		) as HTMLAnchorElement;
		expect(service.getAttribute("href")).toContain(`device=${EDGE}`);
		expect(service.getAttribute("href")).toContain("tab=cloud");
	});

	test("a revoked device: still billed, the limit can be revoked, the approval only by its approver", async () => {
		const view = await open();
		const { container, fake } = view;
		const partner = row(container, PARTNER_GRANT);
		const said = textOf(partner);
		expect(said).toContain("Device revoked");
		expect(said).toContain("Approval still active · still billed to you");
		expect(said).toContain("€12.50 used · €50.00 limit");
		expect(said).toContain("You still pay for a revoked device.");
		expect(partner.querySelector('[data-act="open-service"]')).toBeNull();

		const revokeApproval = act(partner, "revoke-approval");
		expect(revokeApproval.getAttribute("aria-disabled")).toBe("true");
		expect(said).toContain("Only Partner Org Admin can revoke this approval.");
		await click(revokeApproval);
		expect(queryByRole("alertdialog", undefined, document.body)).toBeNull();
		expect(cloudWrites(fake)).toEqual([]);

		const revokeLimit = act(partner, "revoke-limit");
		expect(revokeLimit.getAttribute("aria-disabled")).toBeNull();
		await click(revokeLimit);
		const sheet = inPortal("alertdialog");
		expect(textOf(sheet)).toContain(
			"Revoke the spending limit of report-renderer?",
		);
		expect(textOf(sheet)).toContain(
			"No new model requests are charged to you.",
		);
		expect(textOf(sheet)).toContain("Add a new limit.");
		expect(cloudWrites(fake)).toEqual([]);
		await click(
			byRole("button", /Revoke the spending limit of report-renderer/, sheet),
		);
		await view.settle();
		expect(
			fake.api.sent("DELETE", /billing-grants/).map(([, path]) => path),
		).toEqual([`devices/${PARTNER}/billing-grants/${PARTNER_LIMIT}`]);
		const result = container.querySelector(
			`[data-approval-result="${PARTNER_GRANT}"]`,
		) as HTMLElement;
		expect(textOf(result)).toContain(
			"The spending limit on partner-edge was revoked at",
		);
		expect(textOf(result)).toContain("You won't be charged for new requests.");
		await view.settle();
		expect(
			textOf(container.querySelector("[data-spend-sum]") as HTMLElement),
		).toContain("You pay for 1 spending limit ·");
	});

	test("cancelling the confirmation sends nothing", async () => {
		const view = await open();
		await click(act(row(view.container, EDGE_GRANT), "revoke-approval"));
		const sheet = inPortal("alertdialog");
		expect(textOf(sheet)).toContain(
			"invoice-extractor's instances lose cloud access within minutes.",
		);
		await click(byRole("button", /Cancel/, sheet));
		await view.settle();
		expect(cloudWrites(view.fake)).toEqual([]);
	});

	test("Details shows who approved, the IDs and the leases", async () => {
		const view = await open();
		const edge = row(view.container, EDGE_GRANT);
		await click(act(edge, "approval-details"));
		await view.settle();
		const details = view.container.querySelector(
			`[data-approval-details="${EDGE_GRANT}"]`,
		) as HTMLElement;
		expect(textOf(details)).toContain("Approved by");
		expect(textOf(details)).toContain("Felix Schultz");
		expect(textOf(details)).toContain("Service instance");
		expect(textOf(details)).toContain("lease ends");
		expect(textOf(details)).not.toMatch(MACHINE_WORDS);
	});

	test("an approval that ends before its approved date says what limits it", async () => {
		const fake = await createFakeWorkspace();
		const grant = fake.hub.resourcesOf(EDGE).grants[0];
		if (!grant) throw new Error("the sample has an approval on edge");
		grant.effective_expires_at = grant.expires_at - 5 * 86_400;
		grant.effective_limit = "access_rules";
		const view = await mountCloud(<Routed />, {
			fake,
			search: "view=access&tab=cloud",
		});
		await view.settle();
		const said = textOf(row(view.container, EDGE_GRANT));
		expect(said).toContain("Before the approved date");
		expect(said).toContain(
			"The access rules of edge-berlin-01 expire then. Renewing them extends it.",
		);
	});

	test("an approval whose approver lost the device has ended, whatever its approved date says", async () => {
		const fake = await createFakeWorkspace();
		const grant = fake.hub.resourcesOf(EDGE).grants[0];
		if (!grant) throw new Error("the sample has an approval on edge");
		grant.effective_expires_at = fake.hub.now() - 3_600;
		grant.effective_limit = "sharing_grant";
		const view = await mountCloud(<Routed />, {
			fake,
			search: "view=access&tab=cloud",
		});
		await view.settle();
		const edge = row(view.container, EDGE_GRANT);
		expect(edge.getAttribute("data-state")).toBe("expired");
		expect(textOf(edge)).toContain("Ended");
		expect(textOf(edge)).toContain(
			"The approver's permission to deploy on edge-berlin-01 ended.",
		);
		// It would resume with the permission, so it can still be revoked for good.
		expect(
			act(edge, "revoke-approval").getAttribute("aria-disabled"),
		).toBeNull();
	});

	test("project storage is full: read & write shows as read-only right now", async () => {
		const fake = await createFakeWorkspace();
		const grant = fake.hub.resourcesOf(EDGE).grants[0];
		if (!grant) throw new Error("the sample has an approval on edge");
		grant.online_write_blocked = "storage_full";
		const view = await mountCloud(<Routed />, {
			fake,
			search: "view=access&tab=cloud",
		});
		await view.settle();
		expect(
			textOf(
				row(view.container, EDGE_GRANT).querySelector(
					"[data-write-blocked]",
				) as HTMLElement,
			),
		).toBe("Read-only right now: project storage is full");
	});

	test("details a viewer may not read say so instead of showing nothing", async () => {
		const fake = await createFakeWorkspace();
		fake.api.on("GET", "devices/:id/resource-grants", ({ params }) =>
			params.id === PARTNER
				? []
				: fake.hub
						.resourcesOf(params.id ?? "")
						.grants.map((grant) => fake.hub.grantView(grant)),
		);
		fake.api.on("GET", "devices/:id/billing-grants", ({ params }) =>
			params.id === PARTNER
				? []
				: fake.hub.resourcesOf(params.id ?? "").billing,
		);
		const view = await mountCloud(<Routed />, {
			fake,
			search: "view=access&tab=cloud",
		});
		await view.settle();
		const partner = row(view.container, PARTNER_GRANT);
		expect(textOf(partner)).toContain(
			"Only the approver and the device owner see this",
		);
		expect(textOf(partner)).toContain("€12.50 used");
		expect(
			act(partner, "revoke-limit").getAttribute("aria-disabled"),
		).toBeNull();
	});
});

describe("Access › Cloud approvals & spending on an older hub", () => {
	test("no fleet-wide list: devices are read one by one, with no error and no retry storm", async () => {
		const view = await open({ hubVersion: "old" });
		const { container, fake } = view;
		expect(queryByRole("alert", undefined, container)).toBeNull();
		const note = container.querySelector("[data-per-device]") as HTMLElement;
		expect(textOf(note)).toContain("This hub lists approvals device by device");
		expect(
			fake.api.sent("GET", "devices/resource-summary").length,
		).toBeLessThanOrEqual(1);

		await click(act(note, "read-more"));
		await view.settle();
		expect(row(container, EDGE_GRANT)).not.toBeNull();
		const edge = row(container, EDGE_GRANT);
		expect(
			edge.querySelector("[data-expiry]")?.getAttribute("data-expiry"),
		).toBe("approved");
		// Said once for the table, not in every row.
		expect(
			textOf(container.querySelector("[data-per-device]") as HTMLElement),
		).toContain(
			"Ends are the approved dates: an approval ends earlier if its approver's access to the device ends first, which this hub doesn't report.",
		);
		expect(textOf(edge)).not.toContain("this hub doesn't report");
		expect(textOf(container)).toContain("For the devices read so far");
		expect(
			fake.api.sent("GET", "devices/resource-summary").length,
		).toBeLessThanOrEqual(1);
		expect(queryByRole("alert", undefined, container)).toBeNull();
		expect(
			fake.api.sent("GET", new RegExp(`devices/${STUDIO}/resource-grants`))
				.length,
		).toBeGreaterThan(0);
	});
});

describe("Access › Cloud approvals & spending: states", () => {
	test("nothing approved: an empty state, never an error", async () => {
		const fake = await createFakeWorkspace();
		for (const id of [EDGE, STUDIO, PARTNER]) {
			const entry = fake.hub.resourcesOf(id);
			entry.grants.length = 0;
			entry.billing.length = 0;
			entry.instances.length = 0;
		}
		const view = await mountCloud(<Routed />, {
			fake,
			search: "view=access&tab=cloud",
		});
		await view.settle();
		expect(textOf(view.container)).toContain(
			"No cloud approvals or spending limits",
		);
		expect(view.container.querySelector('[data-kind="empty"]')).not.toBeNull();
	});

	test("the hub refuses the list before anything was read: an error with a retry, not an empty list", async () => {
		const fake = await createFakeWorkspace();
		const heal = fake.api.fail(
			{ method: "GET", path: "devices/resource-summary" },
			new ApiResponseError({
				status: 403,
				code: "FORBIDDEN",
				message: "refused",
			}),
		);
		const view = await mountCloud(<Routed />, {
			fake,
			search: "view=access&tab=cloud",
		});
		await view.settle();
		const { container } = view;
		expect(container.querySelector('[data-kind="empty"]')).toBeNull();
		expect(textOf(container)).toContain(
			"Couldn't read cloud approvals and spending limits",
		);
		expect(textOf(container)).toContain("The hub refused this request.");
		expect(textOf(container)).not.toContain(
			"No cloud approvals or spending limits",
		);

		heal();
		await click(byRole("button", "Try again", container));
		await view.settle();
		expect(container.querySelectorAll("tr[data-approval]").length).toBe(3);
	});

	test("in an app's settings only that app's approvals are listed", async () => {
		const fake = await createFakeWorkspace();
		const view = await mountCloud(<Routed />, {
			fake,
			host: "app",
			search: `id=${SAMPLE_APPS.invoiceAi}&view=access&tab=cloud`,
		});
		await view.settle();
		const rows = view.container.querySelectorAll("tr[data-approval]");
		expect(rows.length).toBe(1);
		expect(rows[0]?.getAttribute("data-approval")).toBe(EDGE_GRANT);
		expect(
			textOf(view.container.querySelector("[data-spend-sum]") as HTMLElement),
		).toContain("You pay for 1 spending limit · €7.41 used of €25.00");
	});

	test("long lists are capped at 20 rows with Show more", async () => {
		const fake = await createFakeWorkspace();
		const entry = fake.hub.resourcesOf(EDGE);
		const first = entry.grants[0];
		if (!first) throw new Error("the sample has an approval on edge");
		for (let index = 0; index < 24; index++)
			entry.grants.push({
				...first,
				grant_id: `00000000-0000-4000-8000-0000000000${String(index).padStart(2, "0")}`,
				placement_id: `extra-${index}`,
			});
		const view = await mountCloud(<Routed />, {
			fake,
			search: "view=access&tab=cloud",
		});
		await view.settle();
		const { container } = view;
		expect(container.querySelectorAll("tr[data-approval]").length).toBe(20);
		const more = act(container, "show-more");
		expect(textOf(more)).toBe("Show 7 more");
		await click(more);
		await view.settle();
		expect(container.querySelectorAll("tr[data-approval]").length).toBe(27);
	});

	test("an older agent changes nothing here: the list comes from the hub and no device is asked", async () => {
		const fake = await createFakeWorkspace(undefined, { agentFeatures: {} });
		const before = fake.api.commands.length;
		const view = await mountCloud(<Routed />, {
			fake,
			search: "view=access&tab=cloud",
		});
		await view.settle();
		expect(view.container.querySelectorAll("tr[data-approval]").length).toBe(3);
		expect(queryByRole("alert", undefined, view.container)).toBeNull();
		expect(fake.api.commands.length).toBe(before);
	});

	test("a completion that arrives after the tab is gone is dropped", async () => {
		const view = await open();
		const { fake } = view;
		const release = fake.api.hold({ method: "DELETE", path: /billing-grants/ });
		await click(act(row(view.container, PARTNER_GRANT), "revoke-limit"));
		await click(
			byRole(
				"button",
				/Revoke the spending limit of report-renderer/,
				inPortal("alertdialog"),
			),
		);
		await view.unmount();
		release();
		await fake.settle();
		expect(document.body.textContent ?? "").not.toContain("was revoked at");
	});
});
