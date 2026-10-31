import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AdminAiActInventoryPage } from "../../packages/ui/components/pages/admin/admin-ai-act-inventory-page";

const fixture = vi.hoisted(() => ({
	get: vi.fn(),
	put: vi.fn(),
	post: vi.fn(),
}));
vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({
		t: (key: string, fallback?: string) => fallback ?? key,
	}),
	i18n: { language: "en" },
}));
vi.mock("../../packages/ui/hooks/use-invoke", () => ({
	useInvoke: () => ({ data: { id: "profile" } }),
}));
vi.mock("../../packages/ui/state/backend-state", () => ({
	useBackend: () => ({ userState: { getProfile: vi.fn() }, apiState: fixture }),
}));
vi.mock(
	"../../packages/ui/lib",
	async () => import("../../packages/ui/lib/utils"),
);
vi.mock("../../packages/ui/components/ui", async () => ({
	...(await import("../../packages/ui/components/ui/accordion")),
	...(await import("../../packages/ui/components/ui/alert")),
	...(await import("../../packages/ui/components/ui/avatar")),
	...(await import("../../packages/ui/components/ui/badge")),
	...(await import("../../packages/ui/components/ui/button")),
	...(await import("../../packages/ui/components/ui/card")),
	...(await import("../../packages/ui/components/ui/checkbox")),
	...(await import("../../packages/ui/components/ui/dialog")),
	...(await import("../../packages/ui/components/ui/input")),
	...(await import("../../packages/ui/components/ui/label")),
	...(await import("../../packages/ui/components/ui/progress")),
	...(await import("../../packages/ui/components/ui/radio-group")),
	...(await import("../../packages/ui/components/ui/relative-time")),
	...(await import("../../packages/ui/components/ui/select")),
	...(await import("../../packages/ui/components/ui/separator")),
	...(await import("../../packages/ui/components/ui/sheet")),
	...(await import("../../packages/ui/components/ui/skeleton")),
	...(await import("../../packages/ui/components/ui/switch")),
	...(await import("../../packages/ui/components/ui/table")),
	...(await import("../../packages/ui/components/ui/tabs")),
	...(await import("../../packages/ui/components/ui/textarea")),
	...(await import("../../packages/ui/components/ui/tooltip")),
}));

const recommendations = [
	{
		id: "ack-transparency",
		title: "Acknowledge transparency obligations",
		category: "Transparency",
		detail: "Confirm the duties are implemented.",
		answerKey: "ack_transparency",
		potentialPoints: 17,
	},
	{
		id: "confirm-personal-data",
		title: "Confirm personal-data handling",
		category: "Data",
		detail: "Confirm whether personal data is used.",
		answerKey: "uses_personal_data",
		potentialPoints: 6,
	},
	{
		id: "vet-models",
		title: "Vet and pin attached models",
		category: "Posture",
		detail: "Review the models.",
		potentialPoints: 15,
	},
];
const options = [
	{ value: "yes", label: "Yes" },
	{ value: "no", label: "No" },
	{ value: "unsure", label: "Not sure" },
];
function detail(score = 47) {
	return {
		appId: "app-one",
		appName: "Example app",
		hasAssessment: true,
		assessment: { status: "DRAFT", updatedAt: "2026-10-30T10:00:00Z" },
		classification: {
			riskCategory: "LIMITED",
			conformityScore: score,
			conformityBand: "red",
			transparencyObligations: ["disclose_ai_interaction"],
			blocked: false,
			rationale: [],
		},
		answers: {},
		signals: {},
		recommendations,
		schema: {
			version: 2,
			screens: [
				{
					id: "transparency",
					title: "Interaction & content",
					questions: [
						{
							key: "ack_transparency",
							kind: "yesno",
							label: "Transparency obligations implemented",
							options,
						},
					],
				},
				{
					id: "data",
					title: "Data handling",
					questions: [
						{
							key: "uses_personal_data",
							kind: "yesno",
							label: "Does this app process personal data?",
							options,
						},
					],
				},
				{
					id: "oversight",
					title: "Human oversight",
					highRiskOnly: true,
					questions: [
						{
							key: "human_oversight",
							kind: "yesno",
							label: "Can a person override decisions?",
							options,
						},
					],
				},
			],
		},
		models: [
			{
				id: "obs",
				modelId: "test-model",
				provider: "test-provider",
				source: "BOARD_SCAN",
				posture: "HOSTED",
				vetted: false,
				dynamicSelector: false,
				driftFlagged: false,
			},
		],
	};
}
let root: Root;
let container: HTMLDivElement;
let client: QueryClient;
beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	vi.spyOn(HTMLElement.prototype, "scrollIntoView").mockImplementation(
		() => {},
	);
	fixture.get.mockReset();
	fixture.put.mockReset();
	fixture.post.mockReset();
	fixture.get.mockImplementation(async (_profile, path) => {
		if (path === "admin/ai-act/inventory/app-one") return detail();
		if (path === "admin/ai-act/models")
			return [
				{
					provider: "test-provider",
					modelId: "test-model",
					posture: "HOSTED",
					vetted: false,
					registered: false,
					observed: true,
					needsRating: true,
				},
			];
		return { boards: [] };
	});
	fixture.put.mockResolvedValue({});
	client = new QueryClient({
		defaultOptions: {
			queries: { retry: false, gcTime: Number.POSITIVE_INFINITY },
		},
	});
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});
afterEach(async () => {
	await act(async () => root.unmount());
	client.clear();
	container.remove();
	vi.restoreAllMocks();
});
async function settle(assertion: () => void) {
	let error: unknown;
	for (let i = 0; i < 60; i++) {
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 20));
		});
		try {
			assertion();
			return;
		} catch (e) {
			error = e;
		}
	}
	throw error;
}
async function render() {
	await act(async () =>
		root.render(
			<QueryClientProvider client={client}>
				<AdminAiActInventoryPage initialAppId="app-one" />
			</QueryClientProvider>,
		),
	);
	await settle(() => expect(container.textContent).toContain("Example app"));
}
function button(label: string) {
	const result = [
		...document.querySelectorAll<HTMLButtonElement>("button"),
	].find(
		(item) =>
			item.getAttribute("aria-label") === label || item.textContent === label,
	);
	if (!result) throw new Error(`Missing button: ${label}`);
	return result;
}
async function click(label: string) {
	await act(async () => button(label).click());
}

describe("AI Act assessment actions", () => {
	it("opens the exact transparency question without duplicate acknowledgement fields", async () => {
		await render();
		await click("Review: Acknowledge transparency obligations");
		await settle(() =>
			expect(document.activeElement?.id).toBe(
				"assessment-question-ack_transparency",
			),
		);
		expect(
			document.querySelectorAll(
				"[role='dialog'] [id='q-ack_transparency-yes']",
			),
		).toHaveLength(1);
		await act(async () =>
			document
				.querySelector<HTMLButtonElement>("#q-ack_transparency-yes")
				?.click(),
		);
		await click("Save & recompute");
		await settle(() =>
			expect(fixture.put).toHaveBeenCalledWith(
				expect.anything(),
				"admin/ai-act/inventory/app-one/assessment",
				expect.objectContaining({ answers: { ack_transparency: "yes" } }),
			),
		);
	});
	it("records an explicit No from an unanswered field and refreshes the score", async () => {
		await render();
		await click("Review: Confirm personal-data handling");
		await settle(() =>
			expect(document.activeElement?.id).toBe(
				"assessment-question-uses_personal_data",
			),
		);
		expect(
			document
				.querySelector("#q-uses_personal_data-no")
				?.getAttribute("aria-checked"),
		).toBe("false");
		await act(async () =>
			document
				.querySelector<HTMLButtonElement>("#q-uses_personal_data-no")
				?.click(),
		);
		fixture.get.mockImplementation(async (_profile, path) =>
			path === "admin/ai-act/inventory/app-one" ? detail(53) : { boards: [] },
		);
		await click("Save & recompute");
		await settle(() =>
			expect(fixture.put).toHaveBeenCalledWith(
				expect.anything(),
				"admin/ai-act/inventory/app-one/assessment",
				expect.objectContaining({ answers: { uses_personal_data: "no" } }),
			),
		);
		await settle(() =>
			expect(document.querySelector("[role='dialog']")).toBeNull(),
		);
		expect(container.textContent).toContain("53");
	});
	it("opens model review without a routing callback and saves vetting to the registry", async () => {
		await render();
		await click("Review: Vet and pin attached models");
		expect(document.activeElement?.textContent).toContain("Attached Models");
		expect(
			container.querySelector("a[href='/library/config/flows?id=app-one']"),
		).not.toBeNull();
		await click("Rate");
		await settle(() =>
			expect(document.querySelector("[role='dialog']")?.textContent).toContain(
				"Rate observed model",
			),
		);
		await act(async () =>
			document
				.querySelector<HTMLButtonElement>(
					"[role='dialog'] button[role='switch'][aria-label='Vetted']",
				)
				?.click(),
		);
		await click("Save rating");
		await settle(() =>
			expect(fixture.put).toHaveBeenCalledWith(
				expect.anything(),
				"admin/ai-act/models",
				expect.objectContaining({
					provider: "test-provider",
					modelId: "test-model",
					posture: "HOSTED",
					vetted: true,
				}),
			),
		);
	});
});
