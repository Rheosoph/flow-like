import { afterAll, afterEach, expect, test } from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import type { IBackendState } from "../../../state/backend-state";
import {
	advance,
	byRole,
	click,
	installDom,
} from "../devices/testing/dom-harness";

const dom = installDom();
const { BackendContext } = await import("../../../state/backend-state");
const { AppAiActWizard } = await import("./app-ai-act-wizard");
const clients: QueryClient[] = [];

afterEach(async () => {
	await dom.cleanup();
	for (const client of clients.splice(0)) client.clear();
});
afterAll(dom.restore);

const recommendations = [
	{
		id: "ack-transparency",
		title: "Acknowledge transparency obligations",
		detail: "Confirm the applicable transparency duties.",
		category: "Transparency",
		potentialPoints: 17,
		answerKey: "ack_transparency",
	},
	{
		id: "confirm-personal-data",
		title: "Confirm personal-data handling",
		detail: "Confirm whether the app processes personal data.",
		category: "Data",
		potentialPoints: 6,
		answerKey: "uses_personal_data",
	},
	{
		id: "vet-models",
		title: "Vet and pin attached models",
		detail: "Review model posture.",
		category: "Posture",
		potentialPoints: 15,
	},
];

function classification(score: number) {
	return {
		riskCategory: "LIMITED",
		conformityScore: score,
		conformityBand: "amber",
		transparencyObligations: ["disclose_ai_interaction"],
		blocked: false,
		rationale: [],
	};
}

function questionnaire(answers: Record<string, unknown> = {}) {
	return {
		schema: {
			version: 2,
			screens: [
				{
					id: "transparency",
					title: "Transparency",
					description: "Review transparency duties.",
					questions: [
						{
							key: "ack_transparency",
							label: "Have you implemented the transparency duties?",
							kind: "yesno",
						},
					],
				},
				{
					id: "data",
					title: "Data handling",
					description: "Review data handling.",
					questions: [
						{
							key: "uses_personal_data",
							label: "Does the app process personal data?",
							kind: "yesno",
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
							label: "Can a person review and override decisions?",
							kind: "yesno",
							required: true,
						},
					],
				},
			],
		},
		signals: {},
		answers,
		classification: classification(47),
		recommendations,
		hasAssessment: true,
	};
}

async function open(
	options: {
		answers?: Record<string, unknown>;
		classify?: (answers: Record<string, unknown>) => Promise<{
			classification: ReturnType<typeof classification>;
			recommendations: typeof recommendations;
		}>;
	} = {},
) {
	const client = new QueryClient({
		defaultOptions: { queries: { retry: false, gcTime: 0 } },
	});
	clients.push(client);
	const saved: { path: string; body: unknown }[] = [];
	const classified: { path: string; answers: Record<string, unknown> }[] = [];
	const backend = {
		userState: {
			getSettingsProfile: async () => ({ hub_profile: {} }),
		},
		roleState: {
			getOwnRole: async () => ({ permissions: 1, is_owner: true }),
		},
		apiState: {
			stream: async (
				_profile: unknown,
				_path: string,
				_request: unknown,
				onMessage: (message: unknown) => void,
			) => {
				onMessage({
					model: "review-model",
					signals: {},
					suggestion: {
						suggestedAnswers: [
							{ key: "ack_transparency", value: "yes" },
							{ key: "uses_personal_data", value: "yes" },
						],
					},
				});
			},
			get: async (_profile: unknown, path: string) =>
				questionnaire(
					path.includes("second")
						? { uses_personal_data: "no" }
						: options.answers,
				),
			post: async (
				_profile: unknown,
				path: string,
				body: { answers: Record<string, unknown> },
			) => {
				classified.push({ path, answers: body.answers });
				return options.classify
					? options.classify(body.answers)
					: { classification: classification(64), recommendations };
			},
			put: async (_profile: unknown, path: string, body: unknown) => {
				saved.push({ path, body });
				return {};
			},
		},
	} as unknown as IBackendState;
	const tree = (appId: string) => (
		<QueryClientProvider client={client}>
			<BackendContext.Provider value={backend}>
				<AppAiActWizard appId={appId} />
			</BackendContext.Provider>
		</QueryClientProvider>
	);
	const view = await dom.render(tree("first"));
	await advance(20);
	await advance(20);
	return { ...view, saved, classified, tree };
}

function question(container: HTMLElement, key: string) {
	const element = container.querySelector<HTMLElement>(
		`[data-ai-act-question="${key}"]`,
	);
	if (!element) throw new Error(`Missing question ${key}`);
	return element;
}

test("recommendations focus editable answers and save them for a limited-risk app", async () => {
	const view = await open();
	await click(
		byRole(
			"button",
			"Review: Acknowledge transparency obligations",
			view.container,
		),
	);
	expect(
		question(view.container, "ack_transparency").contains(
			document.activeElement,
		),
	).toBe(true);
	await click(
		byRole("radio", "Yes", question(view.container, "ack_transparency")),
	);
	await click(
		byRole("button", "Review: Confirm personal-data handling", view.container),
	);
	expect(
		question(view.container, "uses_personal_data").contains(
			document.activeElement,
		),
	).toBe(true);
	await click(
		byRole("radio", "No", question(view.container, "uses_personal_data")),
	);
	await advance(450);
	expect(view.classified.at(-1)?.answers).toEqual({
		ack_transparency: "yes",
		uses_personal_data: "no",
	});
	expect(view.container.textContent).toContain("Conformity score: 64/100");
	await click(byRole("button", "Save draft", view.container));
	expect(view.saved[0]).toEqual({
		path: "apps/first/ai-act/assessment",
		body: {
			answers: { ack_transparency: "yes", uses_personal_data: "no" },
			submit: false,
		},
	});
});

test("an older classification response cannot overwrite the latest score", async () => {
	const pending: ((value: {
		classification: ReturnType<typeof classification>;
		recommendations: typeof recommendations;
	}) => void)[] = [];
	const view = await open({
		classify: () => new Promise((resolve) => pending.push(resolve)),
	});
	await advance(450);
	await click(
		byRole("radio", "Yes", question(view.container, "ack_transparency")),
	);
	await advance(450);
	expect(pending).toHaveLength(2);
	await act(async () =>
		pending[1]({ classification: classification(81), recommendations: [] }),
	);
	await act(async () =>
		pending[0]({ classification: classification(47), recommendations }),
	);
	expect(view.container.textContent).toContain("Conformity score: 81/100");
	expect(view.container.textContent).not.toContain("Conformity score: 47/100");
});

test("switching apps does not submit the previous app's answers", async () => {
	const view = await open({ answers: { ack_transparency: "yes" } });
	await view.rerender(view.tree("second"));
	await advance(20);
	await advance(450);
	expect(view.classified.at(-1)).toEqual({
		path: "apps/second/ai-act/classify",
		answers: { uses_personal_data: "no" },
	});
	await click(byRole("button", "Save draft", view.container));
	expect(view.saved[0]).toEqual({
		path: "apps/second/ai-act/assessment",
		body: { answers: { uses_personal_data: "no" }, submit: false },
	});
});

test("model guidance explains registry permissions and refreshes the current answers", async () => {
	const view = await open({ answers: { ack_transparency: true } });
	expect(
		byRole(
			"radio",
			"Yes",
			question(view.container, "ack_transparency"),
		).getAttribute("aria-checked"),
	).toBe("true");
	await click(
		byRole("button", "Review: Vet and pin attached models", view.container),
	);
	expect(view.container.textContent).toContain("Ask a platform administrator");
	expect(
		byRole("link", "Open app flows", view.container).getAttribute("href"),
	).toBe("/library/config/flows?id=first");
	await advance(450);
	const before = view.classified.length;
	await click(byRole("button", "Refresh score", view.container));
	await advance(450);
	expect(view.classified).toHaveLength(before + 1);
	expect(view.classified.at(-1)?.answers).toEqual({ ack_transparency: true });
});

test("a failed preview keeps the answers and offers a score retry", async () => {
	let fail = true;
	const view = await open({
		answers: { ack_transparency: "yes" },
		classify: async () => {
			if (fail) throw new Error("Temporarily unavailable");
			return { classification: classification(76), recommendations: [] };
		},
	});
	await advance(450);
	expect(view.container.textContent).toContain("Could not update the score.");
	expect(
		byRole("button", "Submit assessment", view.container).hasAttribute(
			"disabled",
		),
	).toBe(true);
	expect(
		byRole("button", "Save draft", view.container).hasAttribute("disabled"),
	).toBe(false);
	fail = false;
	await click(byRole("button", "Refresh score", view.container));
	expect(
		byRole("button", "Submit assessment", view.container).hasAttribute(
			"disabled",
		),
	).toBe(true);
	await advance(450);
	expect(view.classified.at(-1)?.answers).toEqual({ ack_transparency: "yes" });
	expect(view.container.textContent).toContain("Conformity score: 76/100");
	expect(view.container.textContent).not.toContain(
		"Could not update the score.",
	);
	expect(
		byRole("button", "Submit assessment", view.container).hasAttribute(
			"disabled",
		),
	).toBe(false);
});

test("submission waits for the latest risk classification and its required questions", async () => {
	const highResponse = {
		classification: { ...classification(64), riskCategory: "HIGH" },
		recommendations: [],
	};
	let resolveHigh: ((value: typeof highResponse) => void) | undefined;
	const view = await open({
		classify: async (answers) => {
			if (!answers.uses_personal_data)
				return { classification: classification(64), recommendations };
			if (answers.human_oversight) return highResponse;
			return new Promise((resolve) => {
				resolveHigh = resolve;
			});
		},
	});
	await advance(450);
	expect(
		byRole("button", "Submit assessment", view.container).hasAttribute(
			"disabled",
		),
	).toBe(false);
	await click(
		byRole("radio", "Yes", question(view.container, "uses_personal_data")),
	);
	expect(
		byRole("button", "Submit assessment", view.container).hasAttribute(
			"disabled",
		),
	).toBe(true);
	expect(
		byRole("button", "Save draft", view.container).hasAttribute("disabled"),
	).toBe(false);
	await advance(450);
	await act(async () => resolveHigh?.(highResponse));
	expect(question(view.container, "human_oversight")).toBeDefined();
	expect(
		byRole("button", "Submit assessment", view.container).hasAttribute(
			"disabled",
		),
	).toBe(true);
	await click(
		byRole("radio", "Yes", question(view.container, "human_oversight")),
	);
	expect(
		byRole("button", "Submit assessment", view.container).hasAttribute(
			"disabled",
		),
	).toBe(true);
	await advance(450);
	expect(
		byRole("button", "Submit assessment", view.container).hasAttribute(
			"disabled",
		),
	).toBe(false);
});

test("assistant suggestions cannot acknowledge transparency duties for the owner", async () => {
	const view = await open({ answers: { ack_transparency: "no" } });
	await click(byRole("button", "Help me answer", view.container));
	await advance(450);
	expect(view.classified.at(-1)?.answers).toEqual({
		ack_transparency: "no",
		uses_personal_data: "yes",
	});
	expect(
		byRole(
			"radio",
			"No",
			question(view.container, "ack_transparency"),
		).getAttribute("aria-checked"),
	).toBe("true");
});
