import { afterAll, afterEach, describe, expect, test } from "bun:test";
import {
	SAMPLE_IDS,
	SAMPLE_PEOPLE,
} from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import type { ManagementPolicy } from "../../../../lib/device-management/types";
import {
	byRole,
	click,
	dropFiles,
	inPortal,
	installDom,
} from "../testing/dom-harness";
import type { FakeWorkspace } from "../testing/fake-workspace";
import type { MountedDevices } from "../testing/mount-devices";

const dom = installDom();
const { cleanupDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { act } = await import("react");
const { MODEL_HOST_FEATURES, PRE_MODEL_FEATURES } = await import(
	"../../../../lib/device-management/model/__fixtures__/sample-models"
);
const { KNOWN_CAPABILITIES } = await import(
	"../../../../lib/device-management/model/permissions"
);
const { mountAccess, requestFile } = await import("./access-test-kit");
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const { AccessScreen } = await import("./access-screen");
const { useDevicesRoute } = await import("../routing/use-devices-route");
const { useOverlayStore } = await import("../workspace/overlay-store");

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
	useOverlayStore.getState().close();
	globalThis.localStorage?.clear();
});
afterAll(dom.restore);

const { edge } = SAMPLE_IDS;
const ANNA = "usr_anna";
const MODEL_CAPS = ["model_use", "model_manage"] as const;

function Routed() {
	const { route, scope } = useDevicesRoute();
	return <AccessScreen route={route} scope={scope} />;
}

const mount = async (features = MODEL_HOST_FEATURES, legacyHub = false) => {
	const fake = await createFakeWorkspace(undefined, {
		agentFeatures: { [edge]: features },
	});
	if (legacyHub) fake.hub.supportedCapabilities = undefined;
	return mountAccess(<Routed />, {
		fake,
		search: "view=access",
		people: {
			[SAMPLE_PEOPLE.mira]: "Mira Novak",
			[SAMPLE_PEOPLE.jonas]: "Jonas Weber",
			[ANNA]: "Anna Petrova",
		},
		agentFeatures: { [edge]: features },
	});
};

const next = (sheet: HTMLElement) => byRole("button", "Continue", sheet);
const sheet = () => inPortal("dialog");
const row = (capability: string) =>
	sheet().querySelector(
		`[data-permission="${capability}"]`,
	) as HTMLElement | null;
const ticked = (capability: string) =>
	row(capability)
		?.querySelector("[role=checkbox]")
		?.getAttribute("aria-checked");
const summary = () => sheet().querySelector("[data-summary]")?.textContent;
const pressed = (label: string) =>
	byRole("button", label, sheet()).getAttribute("aria-pressed");

const policyOf = (fake: FakeWorkspace) =>
	fake.hub.policies.get(edge)?.policy as ManagementPolicy;

/** Add people › edge › Anna's request file › Permissions. */
async function toPermissions(mounted: MountedDevices) {
	await click(byRole("button", "Add people…", mounted.container));
	await click(
		sheet().querySelector(
			`[data-device-choice="${edge}"] [role=checkbox]`,
		) as HTMLElement,
	);
	await click(next(sheet()));
	await dropFiles(
		sheet().querySelector("label[for=access-wizard-files]") as HTMLElement,
		[requestFile(edge, ANNA, "anna-key")],
	);
	await mounted.settle();
	await click(next(sheet()));
}

describe("model permissions in the access picker", () => {
	test("an agent that hosts models: Use models, Manage models and Model user are offered; Device admin is every permission", async () => {
		const mounted = await mount();
		await toPermissions(mounted);
		for (const capability of MODEL_CAPS) {
			expect(row(capability)?.getAttribute("data-blocked")).toBeNull();
			expect(ticked(capability)).toBe("false");
		}
		expect(row("model_use")?.textContent).toContain("Use models");
		await click(byRole("button", "Device admin", sheet()));
		for (const capability of KNOWN_CAPABILITIES)
			expect(ticked(capability)).toBe("true");
		expect(pressed("Device admin")).toBe("true");
		expect(summary()).toContain("15 permissions");

		await click(byRole("button", "Model user", sheet()));
		expect(pressed("Model user")).toBe("true");
		expect(summary()).toBe("Model user · 1 permission");
		expect(ticked("model_use")).toBe("true");
		expect(ticked("status")).toBe("false");
		await click(next(sheet()));
		await click(byRole("button", "Save access rules", sheet()));
		await mounted.settle();
		expect(
			policyOf(mounted.fake).grants.find((grant) => grant.user_id === ANNA)
				?.capabilities,
		).toEqual(["model_use"]);
	});

	test("a hub without the capability signal offers baseline Device admin even on a model host", async () => {
		const mounted = await mount(MODEL_HOST_FEATURES, true);
		await toPermissions(mounted);
		for (const capability of MODEL_CAPS)
			expect(row(capability) === null).toBe(true);
		expect(sheet().textContent).not.toContain("Model user");
		await click(byRole("button", "Device admin", sheet()));
		expect(summary()).toBe("Device admin · 13 permissions");
		await click(next(sheet()));
		await click(byRole("button", "Save access rules", sheet()));
		await mounted.settle();
		expect(
			policyOf(mounted.fake).grants.find((grant) => grant.user_id === ANNA)
				?.capabilities,
		).not.toContain("model_use");
	});

	test("the hub signal disappears before signing: no model permissions are saved", async () => {
		const mounted = await mount();
		await toPermissions(mounted);
		await click(byRole("button", "Model user", sheet()));
		await click(next(sheet()));
		mounted.fake.hub.supportedCapabilities = undefined;
		await click(byRole("button", "Save access rules", sheet()));
		await mounted.settle();
		expect(mounted.fake.api.sent("PUT", /management\/policy$/)).toHaveLength(0);
		expect(sheet().textContent).toContain(
			"This hub cannot save model permissions yet.",
		);
	});

	test("an agent from before model hosting is offered none of them: an old agent would refuse the whole rules", async () => {
		const mounted = await mount(PRE_MODEL_FEATURES);
		await toPermissions(mounted);
		for (const capability of MODEL_CAPS)
			expect(row(capability) === null).toBe(true);
		expect(sheet().textContent).not.toContain("Model user");
		await click(byRole("button", "Device admin", sheet()));
		expect(summary()).toBe("Device admin · 13 permissions");
		expect(pressed("Device admin")).toBe("true");
	});

	test("an app or service can't hold model permissions: Model user needs the whole device too", async () => {
		const mounted = await mount();
		await toPermissions(mounted);
		await click(byRole("button", "App", sheet()));
		expect(sheet().textContent).toContain(
			"Device admin and Model user need whole-device access.",
		);
		expect(
			byRole("button", "Model user", sheet()).hasAttribute("disabled"),
		).toBe(true);
		for (const capability of MODEL_CAPS)
			expect(row(capability)?.getAttribute("data-blocked")).toBe("device_only");
	});

	test("the agent stops hosting models before the change is signed: nothing is saved, and the permission says why", async () => {
		const mounted = await mount();
		const { fake } = mounted;
		await toPermissions(mounted);
		await click(byRole("button", "Model user", sheet()));
		await click(next(sheet()));
		const release = fake.api.hold({
			method: "GET",
			path: /management\/policy$/,
		});
		await click(byRole("button", "Save access rules", sheet()));
		fake.agent(edge).features = PRE_MODEL_FEATURES;
		await act(async () => {
			await fake.workspace.live.refreshInspection(edge);
		});
		release();
		await mounted.settle();
		expect(fake.api.sent("PUT", /management\/policy$/)).toHaveLength(0);
		expect(sheet().textContent).toContain(
			"edge-berlin-01's agent doesn't host models, so it would refuse these access rules. Update the agent and connect once, or remove Use models and Manage models.",
		);
		await click(byRole("button", "Back", sheet()));
		expect(row("model_use")?.getAttribute("data-blocked")).toBe(
			"models_unsupported",
		);
		expect(row("model_use")?.textContent).toContain(
			"edge-berlin-01's agent doesn't host models. Update the agent first.",
		);
		expect(row("model_manage")).toBeNull();
	});
});
