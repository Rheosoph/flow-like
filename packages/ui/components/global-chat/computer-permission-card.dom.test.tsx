// @vitest-environment happy-dom
import botsLocale from "@flow-like/locales/locales/en/bots.json";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { buildToolActivity } from "../../state/global-chat/tool-step-activity";
import type { IPlanStep } from "../interfaces/chat-default/chat-db";
import { PlanSteps } from "../interfaces/chat-default/plan-steps";
import { ToolActivityRow } from "../interfaces/chat-default/tool-activity";
import { RPA_PERMISSIONS_REQUIRED } from "./computer-permission";

function english(key: string): string {
	const value = key
		.split(".")
		.reduce<unknown>(
			(node, part) => (node as Record<string, unknown> | undefined)?.[part],
			botsLocale,
		);
	if (typeof value !== "string")
		throw new Error(`bots locale has no English text for ${key}`);
	return value;
}

vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({
		t: (key: string, fallback?: unknown, options?: Record<string, unknown>) =>
			(key.startsWith("computer.")
				? english(key)
				: typeof fallback === "string"
					? fallback
					: key
			).replace(/\{\{(\w+)\}\}/g, (_, name: string) =>
				String(options?.[name] ?? ""),
			),
	}),
}));

let root: Root;
let host: HTMLDivElement;

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
});

afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
	Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
});

const desktop = () =>
	Object.assign(window, { __TAURI_INTERNALS__: {} }) as unknown;

function failedStep(
	permission: string,
	{ restart = false, code = "permission_missing" } = {},
): IPlanStep {
	const detail = buildToolActivity("computer_screenshot", {
		tool_call_id: "call_1",
		tool: "computer_screenshot",
		status: "error",
		result_summary: "error",
		result_preview: JSON.stringify({
			status: "error",
			tool: "computer_screenshot",
			permission,
			restart_required: restart,
			error: {
				code,
				message: "Flow Like has no Accessibility permission.",
			},
		}),
	});
	return {
		id: "call_1",
		title: "Using computer_screenshot",
		status: "failed",
		toolName: "computer_screenshot",
		detail,
	};
}

async function renderRow(step: IPlanStep) {
	if (step.detail?.kind !== "tool_activity") throw new Error("not a tool row");
	const detail = step.detail;
	await act(async () =>
		root.render(
			<ToolActivityRow
				step={step}
				detail={detail}
				isActive={false}
				isExpanded={false}
				onToggle={() => {}}
				statusIcon={null}
				duration={null}
			/>,
		),
	);
	return host.querySelector<HTMLElement>("[data-computer-permission]");
}

describe("ComputerPermissionCard in a tool row", () => {
	test("a permission_missing row shows the card, and its button opens the permission dialog", async () => {
		desktop();
		const card = await renderRow(failedStep("screen_recording"));
		expect(card?.dataset.computerPermission).toBe("screen_recording");
		expect(card?.textContent).toContain(
			"Flow Like needs Screen Recording access",
		);
		expect(card?.textContent).toContain(
			"After you allow it, quit and reopen Flow Like.",
		);
		expect(host.textContent).not.toContain(
			"Flow Like has no Accessibility permission.",
		);

		const seen: unknown[] = [];
		const listen = (event: Event) => seen.push((event as CustomEvent).detail);
		window.addEventListener(RPA_PERMISSIONS_REQUIRED, listen);
		const button = card?.querySelector("button");
		expect(button?.textContent).toBe("Open permissions");
		await act(async () => button?.click());
		window.removeEventListener(RPA_PERMISSIONS_REQUIRED, listen);

		expect(seen).toHaveLength(1);
		expect(seen[0]).toMatchObject({ required: ["screen_capture"] });
		expect(typeof (seen[0] as { requestId?: unknown }).requestId).toBe(
			"string",
		);
	});

	test("Accessibility asks the dialog for accessibility", async () => {
		desktop();
		const card = await renderRow(failedStep("accessibility"));
		expect(card?.textContent).toContain("Flow Like needs Accessibility access");
		const seen: unknown[] = [];
		const listen = (event: Event) => seen.push((event as CustomEvent).detail);
		window.addEventListener(RPA_PERMISSIONS_REQUIRED, listen);
		await act(async () => card?.querySelector("button")?.click());
		window.removeEventListener(RPA_PERMISSIONS_REQUIRED, listen);
		expect(seen[0]).toMatchObject({ required: ["accessibility"] });
	});

	test("restart_required says to quit and reopen Flow Like, without a button", async () => {
		desktop();
		const card = await renderRow(
			failedStep("screen_recording", { restart: true }),
		);
		expect(card?.dataset.restartRequired).toBe("true");
		expect(card?.textContent).toContain("Quit and reopen Flow Like");
		expect(card?.textContent).toContain(
			"macOS applies Screen Recording only after Flow Like starts again.",
		);
		expect(card?.querySelector("button")).toBeNull();
	});

	test("on the web the card is text only", async () => {
		const card = await renderRow(failedStep("screen_recording"));
		expect(card?.textContent).toContain(
			"System Settings > Privacy & Security on the Mac that runs this bot",
		);
		expect(card?.querySelector("button")).toBeNull();
	});

	test("the activity group stays open after the turn when a step asks for a permission", async () => {
		desktop();
		const other = (id: string): IPlanStep => ({
			...failedStep("screen_recording", { code: "no_display" }),
			id,
			status: "done",
		});
		const render = (loading: boolean, steps: IPlanStep[]) =>
			act(async () =>
				root.render(<PlanSteps steps={steps} loading={loading} />),
			);
		const asking = [failedStep("screen_recording"), other("b"), other("c")];
		await render(true, asking);
		await render(false, asking);
		expect(host.querySelector("[data-computer-permission]")).not.toBeNull();

		const plain = [other("a"), other("b")];
		await render(true, plain);
		await render(false, plain);
		expect(host.querySelector("[data-tool-activity]")).toBeNull();
	});

	test("another error code keeps the plain row with the tool's message", async () => {
		desktop();
		const card = await renderRow(
			failedStep("screen_recording", { code: "no_display" }),
		);
		expect(card).toBeNull();
		expect(host.textContent).toContain(
			"Flow Like has no Accessibility permission.",
		);
	});
});
