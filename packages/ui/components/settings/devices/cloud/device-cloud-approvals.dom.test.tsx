import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { ApiResponseError } from "../../../../lib/api-error";
import {
	byRole,
	click,
	inPortal,
	installDom,
	queryByRole,
} from "../testing/dom-harness";
import type { FakeWorkspace } from "../testing/fake-workspace";
import type { MountCloudOptions } from "./cloud-test-kit";

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
	await import("./cloud-test-kit");
const { useDevicesRoute } = await import("../routing/use-devices-route");
const { DeviceCloudApprovals } = await import("./device-cloud-approvals");

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

const { edge: EDGE, partner: PARTNER, warehouse: WAREHOUSE } = SAMPLE_IDS;
const EDGE_GRANT = "d99ba88b-717b-445e-a719-a2084df3aec0";
const PARTNER_LIMIT = "cc3e21cf-49b2-4ed9-9eae-a337d7912be3";

function Tab({ deviceId }: Readonly<{ deviceId: string }>) {
	const { route, scope } = useDevicesRoute();
	return (
		<DeviceCloudApprovals route={route} scope={scope} deviceId={deviceId} />
	);
}

interface OpenOptions extends MountCloudOptions {
	arrange?(fake: FakeWorkspace): void;
}

async function open(
	deviceId: string,
	{ arrange, ...options }: OpenOptions = {},
) {
	const fake = await createFakeWorkspace(undefined, options);
	arrange?.(fake);
	const view = await mountCloud(<Tab deviceId={deviceId} />, {
		...options,
		fake,
		search: `device=${deviceId}&tab=access`,
	});
	await view.settle();
	return view;
}

const act = (root: ParentNode, name: string) =>
	root.querySelector<HTMLElement>(`[data-act="${name}"]`) as HTMLElement;

describe("Device › Access › Cloud approvals", () => {
	test("one block per approval: what it allows, who pays, the leases and where to manage it", async () => {
		const view = await open(EDGE);
		const { container } = view;
		const blocks = container.querySelectorAll("[data-block]");
		expect(blocks.length).toBe(1);
		const said = textOf(container);
		expect(said).toContain("Cloud approval · invoice-extractor");
		expect(said).toContain("App Invoice AI");
		expect(said).toContain("Active");
		expect(said).toContain("BGE-M3 embeddings, GPT-4.1 mini");
		expect(said).toContain("Read & write project files");
		expect(said).toContain(
			"€7.41 used · €0.12 reserved · €25.00 limit · paid by you · ends",
		);
		expect(said).toContain("Service instance");
		expect(said).toContain("It isn't proof the instance is healthy.");
		expect(said).toContain("checked");
		expect(container.querySelectorAll("[data-lease]").length).toBe(1);

		expect(act(container, "open-service").getAttribute("href")).toContain(
			"tab=cloud",
		);
		const all = [...container.querySelectorAll("a")].find(
			(link) =>
				link.textContent === "Cloud approvals and spending on all devices",
		) as HTMLAnchorElement;
		expect(all.getAttribute("href")).toContain("view=access");
		expect(all.getAttribute("href")).toContain("tab=cloud");

		expect(said).not.toMatch(MACHINE_WORDS);
		expect(primaries(container)).toBe(0);
		expect(cloudWrites(view.fake)).toEqual([]);
	});

	test("Revoke approval… asks first and then revokes it", async () => {
		const view = await open(EDGE);
		const { container, fake } = view;
		await click(act(container, "revoke-approval"));
		const sheet = inPortal("alertdialog");
		expect(textOf(sheet)).toContain(
			"invoice-extractor's instances lose cloud access within minutes.",
		);
		expect(cloudWrites(fake)).toEqual([]);
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
		expect(textOf(container)).toContain("Revoked");
		expect(act(container, "revoke-approval")).toBeNull();
	});

	test("a revoked device: you still pay, the limit can be revoked, the approval only by its approver", async () => {
		const view = await open(PARTNER);
		const { container, fake } = view;
		const said = textOf(container);
		expect(said).toContain("You still pay for a revoked device.");
		expect(said).toContain(
			"partner-edge can't run anything any more, but this spending limit stays active until",
		);
		expect(said).toContain("€12.50 used · €50.00 limit · paid by you");
		expect(act(container, "open-service")).toBeNull();

		const revokeApproval = act(container, "revoke-approval");
		expect(revokeApproval.getAttribute("aria-disabled")).toBe("true");
		expect(said).toContain("Only Partner Org Admin can revoke this approval.");
		await click(revokeApproval);
		expect(queryByRole("alertdialog", undefined, document.body)).toBeNull();
		expect(cloudWrites(fake)).toEqual([]);

		const revokeLimit = act(container, "revoke-limit");
		expect(revokeLimit.getAttribute("aria-disabled")).toBeNull();
		// The usable control comes first; the gated one and its reason follow.
		expect(
			revokeLimit.compareDocumentPosition(revokeApproval) &
				Node.DOCUMENT_POSITION_FOLLOWING,
		).toBeTruthy();
		await click(revokeLimit);
		expect(textOf(inPortal("alertdialog"))).toContain(
			"Requests already running may still complete and bill.",
		);
		await click(
			byRole(
				"button",
				"Revoke the spending limit of report-renderer",
				inPortal("alertdialog"),
			),
		);
		await view.settle();
		expect(cloudWrites(fake).map(([method, path]) => [method, path])).toEqual([
			["DELETE", `devices/${PARTNER}/billing-grants/${PARTNER_LIMIT}`],
		]);
		expect(textOf(container)).toContain(
			"The spending limit on partner-edge was revoked at",
		);
		expect(textOf(container)).not.toContain(
			"You still pay for a revoked device.",
		);
	});

	test("a device without approvals says its services run locally", async () => {
		const view = await open(WAREHOUSE);
		const { container } = view;
		expect(
			textOf(container.querySelector("[data-cloud-empty]") as HTMLElement),
		).toBe(
			"No cloud approvals on warehouse-pi. Services here use local resources only.",
		);
		expect(queryByRole("alert", undefined, container)).toBeNull();
	});

	test("more than five approvals are capped behind Show more", async () => {
		const view = await open(EDGE, {
			arrange: (fake) => {
				const entry = fake.hub.resourcesOf(EDGE);
				const first = entry.grants[0];
				if (!first) throw new Error("the sample has an approval on edge");
				for (let index = 0; index < 6; index++)
					entry.grants.push({
						...first,
						grant_id: `00000000-0000-4000-8000-00000000000${index}`,
						placement_id: `extra-${index}`,
					});
			},
		});
		const { container } = view;
		expect(container.querySelectorAll("[data-block]").length).toBe(5);
		const more = act(container, "show-more");
		expect(textOf(more)).toBe("Show 2 more approvals");
		await click(more);
		expect(container.querySelectorAll("[data-block]").length).toBe(7);
		expect(act(container, "show-more")).toBeNull();
	});
});

describe("Device › Access › Cloud approvals: hub states", () => {
	test("an older hub shows the approved date with a note and no error", async () => {
		const view = await open(EDGE, { hubVersion: "old" });
		const { container, fake } = view;
		expect(queryByRole("alert", undefined, container)).toBeNull();
		expect(
			container.querySelector("[data-expiry]")?.getAttribute("data-expiry"),
		).toBe("approved");
		expect(textOf(container)).toContain("this hub doesn't report that");
		expect(textOf(container)).toContain("BGE-M3 embeddings, GPT-4.1 mini");
		expect(
			fake.api.sent("GET", "devices/resource-summary").length,
		).toBeLessThanOrEqual(1);
	});

	test("the hub refuses the read: an error with a retry, never 'no approvals'", async () => {
		const view = await open(EDGE, {
			arrange: (fake) => {
				fake.api.fail(
					{ method: "GET", path: /resource-grants|resource-summary/ },
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
			"Couldn't read this device's cloud approvals",
		);
		expect(textOf(container)).toContain("The hub refused this request.");
		expect(container.querySelector("[data-cloud-empty]")).toBeNull();
		expect(byRole("button", "Try again", container)).not.toBeNull();
	});
});

describe("Device › Access › Cloud approvals for a local-only app", () => {
	test("reads Model access and shows no project-files row", async () => {
		const view = await open(EDGE, {
			arrange: (fake) => {
				const entry = fake.hub.resourcesOf(EDGE);
				entry.billing.length = 0;
				entry.instances.length = 0;
				entry.grants.length = 0;
				entry.grants.push({
					grant_id: "0b0b0b0b-0000-4000-8000-000000000001",
					device_id: EDGE,
					placement_id: "support-bot",
					deployment_id: "0b0b0b0b-0000-4000-8000-000000000002",
					project_id: SAMPLE_APPS.supportPortal,
					app_id: null,
					delegating_user_id: fake.hub.me,
					authz_version: 1,
					model_ids: ["gpt-4.1-mini"],
					online_access: null,
					max_instances: 4,
					expires_at: fake.hub.now() + 30 * 86_400,
					status: "active",
				});
			},
		});
		const said = textOf(view.container);
		expect(said).toContain("Model access · support-bot");
		expect(said).toContain("App Support Portal");
		expect(said).not.toContain("Project files");
		expect(said).toContain(
			"No spending limit: hosted models refuse its model calls",
		);
		expect(textOf(act(view.container, "open-service"))).toBe(
			"Open service's model access",
		);
	});
});
