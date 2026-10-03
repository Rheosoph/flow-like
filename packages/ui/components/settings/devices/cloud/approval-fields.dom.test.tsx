import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { useState } from "react";
import type {
	ApprovalDraft,
	SpendingDraft,
} from "../../../../lib/device-management/model/deploy-plan";
import {
	byRole,
	click,
	installDom,
	queryByRole,
	typeInto,
} from "../testing/dom-harness";
import type { FakeWorkspace } from "../testing/fake-workspace";
import type { CloudApprovalFieldsProps } from "./approval-fields";
import type { MountCloudOptions } from "./cloud-test-kit";

const dom = installDom();
const { cleanupDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { SAMPLE_APPS, SAMPLE_IDS, SAMPLE_NOW } = await import(
	"../../../../lib/device-management/model/__fixtures__/sample-fleet"
);
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const { MACHINE_WORDS, mountCloud, textOf } = await import("./cloud-test-kit");
const { CloudApprovalFields, SpendingLimitFields } = await import(
	"./approval-fields"
);

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

const EDGE = SAMPLE_IDS.edge;
const APP = SAMPLE_APPS.invoiceAi;
const DAY = 86_400;
const EDGE_GRANT = "d99ba88b-717b-445e-a719-a2084df3aec0";

const draft = (patch: Partial<ApprovalDraft> = {}): ApprovalDraft => ({
	files: "none",
	ownerConsent: false,
	models: [],
	maxInstances: 1,
	expiresAt: SAMPLE_NOW + 30 * DAY,
	...patch,
});

type FieldProps = Omit<CloudApprovalFieldsProps, "value" | "onChange">;

function Approval({
	initial,
	...props
}: Readonly<FieldProps & { initial: ApprovalDraft }>) {
	const [value, setValue] = useState(initial);
	return (
		<>
			<CloudApprovalFields {...props} value={value} onChange={setValue} />
			<output data-draft="">{JSON.stringify(value)}</output>
		</>
	);
}

function Spending({
	approval,
	grantId,
}: Readonly<{ approval: ApprovalDraft; grantId?: string }>) {
	const [value, setValue] = useState<SpendingDraft>({
		limitMicros: 10_000_000,
		expiresAt: 0,
		consent: false,
	});
	return (
		<>
			<SpendingLimitFields
				deviceId={EDGE}
				value={value}
				approval={approval}
				onChange={setValue}
				{...(grantId ? { grantId } : {})}
			/>
			<output data-draft="">{JSON.stringify(value)}</output>
		</>
	);
}

const mount = (
	node: React.ReactNode,
	options: MountCloudOptions & { arrange?(fake: FakeWorkspace): void } = {},
) =>
	createFakeWorkspace(undefined, options).then(async (fake) => {
		options.arrange?.(fake);
		const view = await mountCloud(node, {
			appModels: { [APP]: ["gpt-4.1-mini", "bge-m3"] },
			...options,
			fake,
		});
		await view.settle();
		return view;
	});

const read = <T,>(root: ParentNode): T =>
	JSON.parse(
		(root.querySelector("[data-draft]") as HTMLElement).textContent ?? "{}",
	) as T;

const online: FieldProps = {
	deviceId: EDGE,
	appId: APP,
	modelOnly: false,
	serviceMaxInstances: 2,
	isAppOwner: true,
};

describe("CloudApprovalFields", () => {
	test("project files need the owner's tick; models come from the app", async () => {
		const view = await mount(
			<Approval {...online} initial={draft({ maxInstances: 2 })} />,
		);
		const { container } = view;
		expect(textOf(container)).toContain("Project files");
		expect(textOf(container)).toContain("Model calls only.");
		expect(textOf(container)).toContain(
			"Pick at least one model, or allow project files.",
		);
		expect(queryByRole("checkbox", /I own Invoice AI/, container)).toBeNull();

		await click(byRole("button", "Read & write", container));
		expect(textOf(container)).toContain(
			"Reads and changes files in Invoice AI's cloud storage. Needed for write buffering.",
		);
		expect(textOf(container)).toContain(
			"Tick the box to allow access to the project files.",
		);
		await click(
			byRole(
				"checkbox",
				"I own Invoice AI and allow this service to read and change its files",
				container,
			),
		);
		expect(textOf(container)).not.toContain("Tick the box");

		await click(byRole("checkbox", "GPT-4.1 mini", container));
		await click(byRole("checkbox", "BGE-M3 embeddings", container));
		expect(read<ApprovalDraft>(container)).toEqual({
			files: "read_write",
			ownerConsent: true,
			models: ["bge-m3", "gpt-4.1-mini"],
			maxInstances: 2,
			expiresAt: SAMPLE_NOW + 30 * DAY,
		});

		await click(byRole("button", "None", container));
		expect(read<ApprovalDraft>(container).ownerConsent).toBe(false);
		expect(textOf(container)).not.toMatch(MACHINE_WORDS);
	});

	test("instances can't go below the service's, and the end is 1 to 365 days shown as a date", async () => {
		const view = await mount(
			<Approval
				{...online}
				initial={draft({ models: ["gpt-4.1-mini"], maxInstances: 2 })}
			/>,
		);
		const { container } = view;
		const [instances, days] = [
			...container.querySelectorAll<HTMLInputElement>("input:not([type])"),
			...container.querySelectorAll<HTMLInputElement>('input[type="text"]'),
		];
		if (!instances || !days) throw new Error("two number fields expected");
		expect(days.value).toBe("30");
		expect(textOf(container)).toContain(
			"At least 2, this service's instances.",
		);
		expect(textOf(container)).toContain("· at most 365 days");

		await typeInto(instances, "1");
		expect(textOf(container)).toContain(
			"Must be at least 2, this service's instances.",
		);
		await typeInto(instances, "101");
		expect(textOf(container)).toContain("Enter 1 to 100 instances.");
		await typeInto(instances, "3");
		expect(read<ApprovalDraft>(container).maxInstances).toBe(3);

		await typeInto(days, "400");
		expect(textOf(container)).toContain("Enter 1 to 365 days.");
		await typeInto(days, "0");
		expect(textOf(container)).toContain("Enter 1 to 365 days.");
		await typeInto(days, "7");
		expect(read<ApprovalDraft>(container).expiresAt).toBe(SAMPLE_NOW + 7 * DAY);
		expect(textOf(container)).not.toContain("Enter 1 to 365 days.");
	});

	test("model-only (offline copies): no project files, the fixed line, model wording", async () => {
		const view = await mount(
			<Approval
				deviceId={EDGE}
				appId={null}
				modelsAppId={APP}
				modelOnly
				initial={draft()}
			/>,
		);
		const { container } = view;
		expect(textOf(container)).not.toContain("Project files");
		expect(container.querySelector("[data-files]")).toBeNull();
		expect(textOf(container)).toContain(
			"Local-only apps can't get access to cloud files.",
		);
		expect(textOf(container)).toContain("Pick at least one model.");
		expect(textOf(container)).not.toContain("or allow project files");
		expect(byRole("checkbox", "GPT-4.1 mini", container)).not.toBeNull();
	});

	test("someone who doesn't own the app sees why project files are off", async () => {
		const view = await mount(
			<Approval {...online} isAppOwner={false} initial={draft()} />,
		);
		const { container } = view;
		expect(textOf(container)).toContain(
			"Only the owner of Invoice AI can allow access to its project files.",
		);
		const write = byRole("button", "Read & write", container);
		await click(write);
		expect(read<ApprovalDraft>(container).files).toBe("none");
		expect(queryByRole("checkbox", /I own Invoice AI/, container)).toBeNull();
	});

	test("a role that can't approve: every control is off and the reason is one line", async () => {
		const view = await mount(
			<Approval
				{...online}
				disabledReason={{ code: "role_admin_execute" }}
				initial={draft({ models: ["gpt-4.1-mini"] })}
			/>,
		);
		const { container } = view;
		expect(textOf(container)).toContain(
			"Needs Admin or Owner on the app with Execute boards.",
		);
		const before = read<ApprovalDraft>(container);
		await click(byRole("button", "Read", container));
		await click(byRole("checkbox", "BGE-M3 embeddings", container));
		expect(read<ApprovalDraft>(container)).toEqual(before);
	});

	test("a model the app no longer uses stays listed while it is approved", async () => {
		const view = await mount(
			<Approval {...online} initial={draft({ models: ["mistral-small-3"] })} />,
		);
		expect(
			byRole("checkbox", "Mistral Small 3", view.container).getAttribute(
				"aria-checked",
			),
		).toBe("true");
	});
});

describe("SpendingLimitFields", () => {
	const approval = draft({ models: ["gpt-4.1-mini"] });

	test("€10.00 by default, ends with the approval, paid by you, with a consent tick", async () => {
		const view = await mount(<Spending approval={approval} />);
		const { container } = view;
		const amount = container.querySelector("input") as HTMLInputElement;
		expect(amount.value).toBe("10.00");
		expect(read<SpendingDraft>(container).expiresAt).toBe(approval.expiresAt);
		const terms = textOf(
			container.querySelector("[data-spend-terms]") as HTMLElement,
		);
		expect(terms).toContain(
			"€10.00 at most · paid by you · doesn't renew · ends with the approval",
		);

		await typeInto(amount, "12,5");
		expect(read<SpendingDraft>(container).limitMicros).toBe(12_500_000);
		await typeInto(amount, "10.000001");
		expect(read<SpendingDraft>(container).limitMicros).toBe(10_000_001);
		expect(textOf(container)).toContain("€10.000001 at most");
		// What isn't an amount never keeps the amount typed before it.
		for (const bad of ["0", "0.0000001", "1000000.01", "ten"]) {
			await typeInto(amount, bad);
			expect(amount.value).toBe(bad);
			expect(textOf(container)).toContain(
				"Enter an amount above €0 and up to €1,000,000.",
			);
			expect(read<SpendingDraft>(container).limitMicros).toBe(0);
			expect(textOf(container)).not.toContain("at most");
		}
		await typeInto(amount, "25");
		expect(read<SpendingDraft>(container).limitMicros).toBe(25_000_000);

		await click(
			byRole(
				"checkbox",
				"I pay for model use by this service up to this limit.",
				container,
			),
		);
		expect(read<SpendingDraft>(container).consent).toBe(true);
	});

	test("before an approval exists the plan check is the interim sentence, and nothing is asked", async () => {
		const view = await mount(<Spending approval={approval} />);
		expect(textOf(view.container)).toContain(
			"Whether your plan covers each model is checked when the service first calls it.",
		);
		expect(view.fake.api.sent("GET", /\/eligibility$/)).toEqual([]);
	});

	test("an existing approval: the hub says whether the plan covers the models", async () => {
		const covered = await mount(
			<Spending approval={approval} grantId={EDGE_GRANT} />,
		);
		expect(textOf(covered.container)).toContain(
			"Your PRO plan covers these models.",
		);
		await covered.unmount();

		const refused = await mount(
			<Spending approval={approval} grantId={EDGE_GRANT} />,
			{
				arrange: (fake) => {
					fake.hub.eligibility.set(EDGE_GRANT, {
						payer_id: fake.hub.me,
						plan: "FREE",
						eligible: false,
						models: [
							{ model_id: "gpt-4.1-mini", tier: "PRO", allowed: false },
							{ model_id: "bge-m3", tier: null, allowed: true },
						],
					});
				},
			},
		);
		expect(textOf(refused.container)).toContain(
			"Your FREE plan doesn't cover GPT-4.1 mini. Calls to them are refused even with a limit.",
		);
	});

	test("an older hub can't say: the interim sentence, one request, no error", async () => {
		const view = await mount(
			<Spending approval={approval} grantId={EDGE_GRANT} />,
			{ hubVersion: "old" },
		);
		expect(textOf(view.container)).toContain(
			"Whether your plan covers each model is checked when the service first calls it.",
		);
		expect(queryByRole("alert", undefined, view.container)).toBeNull();
		expect(view.fake.api.sent("GET", /\/eligibility$/).length).toBe(1);
	});
});
