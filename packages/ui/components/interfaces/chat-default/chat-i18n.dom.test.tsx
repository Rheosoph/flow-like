import { afterAll, afterEach, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { createInstance } from "i18next";
import { act } from "react";
import { I18nextProvider } from "react-i18next";
import deChat from "../../../../locales/locales/de/chat.json";
import deCommon from "../../../../locales/locales/de/common.json";
import enChat from "../../../../locales/locales/en/chat.json";
import enCommon from "../../../../locales/locales/en/common.json";
import { buildToolActivity } from "../../../state/global-chat/tool-step-activity";
import type { ProcessedAttachment } from "./attachment";
import type { IPlanStep } from "./chat-db";

const window = new Window({ url: "https://localhost" });
const globals = {
	window,
	document: window.document,
	navigator: window.navigator,
	HTMLElement: window.HTMLElement,
	Element: window.Element,
	Node: window.Node,
	getComputedStyle: window.getComputedStyle.bind(window),
	requestAnimationFrame: window.requestAnimationFrame.bind(window),
	cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
	IS_REACT_ACT_ENVIRONMENT: true,
};
const descriptors = Object.keys(globals).map(
	(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
);
Object.assign(globalThis, globals);
const { createRoot } = await import("react-dom/client");
const { ResearchActivity } = await import("./research-activity");
const { AttachmentStrip } = await import("./attachment-strip");
const { InlineStepGroup, PlanSteps } = await import("./plan-steps");
const { ToolActivityRow } = await import("./tool-activity");

const roots: ReturnType<typeof createRoot>[] = [];
afterEach(async () => {
	await act(async () => {
		for (const root of roots.splice(0)) root.unmount();
	});
	window.document.body.replaceChildren();
});
afterAll(() => {
	for (const [key, descriptor] of descriptors) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
	window.happyDOM.abort();
});

async function render(ui: React.ReactNode) {
	const i18n = createInstance();
	await i18n.init({
		lng: "de",
		fallbackLng: "en",
		resources: {
			de: { chat: deChat, common: deCommon },
			en: { chat: enChat, common: enCommon },
		},
		interpolation: { escapeValue: false },
		initAsync: false,
	});
	const container = window.document.createElement("div");
	window.document.body.append(container);
	const root = createRoot(container as unknown as Element);
	roots.push(root);
	await act(async () => {
		root.render(<I18nextProvider i18n={i18n}>{ui}</I18nextProvider>);
	});
	return { container, i18n };
}

test("research uses German counts, statuses and controls, then updates when the language changes", async () => {
	const sources = Array.from({ length: 5 }, (_, index) => ({
		url: `https://source${index}.example/page`,
		hostname: `source${index}.example`,
		title: `Source ${index}`,
	}));
	const steps: IPlanStep[] = [
		{
			id: "search",
			title: "Search",
			toolName: "internet_search",
			status: "done",
			detail: {
				kind: "tool_activity",
				category: "search",
				label: "Search",
				queries: ["a", "b", "c", "d", "e"],
				sources,
			},
		},
		{
			id: "read",
			title: "Read",
			toolName: "open_url",
			status: "progress",
			detail: {
				kind: "tool_activity",
				category: "web",
				label: "Read",
				sources: [sources[0]],
			},
		},
	];
	const { container, i18n } = await render(<ResearchActivity steps={steps} />);
	expect(container.querySelector("section")?.getAttribute("aria-label")).toBe(
		"Webrecherche",
	);
	expect(container.textContent).toContain("5 Suchanfragen");
	expect(container.textContent).toContain("5 Seiten");
	expect(container.textContent).toContain("1 frühere Suchanfrage anzeigen");
	expect(container.textContent).toContain("Alle 5 Seiten anzeigen");
	expect(container.textContent).toContain("Wird gelesen");
	expect(container.textContent).toContain("source0.example wird gelesen");
	const more = [...container.querySelectorAll("button")].find(
		(button) => button.textContent === "Alle 5 Seiten anzeigen",
	);
	await act(async () => more?.click());
	expect(container.querySelectorAll("[data-research-source]")).toHaveLength(5);
	expect(container.textContent).toContain("Weniger Seiten anzeigen");
	await act(async () => {
		await i18n.changeLanguage("en");
	});
	expect(container.querySelector("section")?.getAttribute("aria-label")).toBe(
		"Web research",
	);
	expect(container.textContent).toContain("5 searches");
	expect(container.textContent).toContain("Reading source0.example");
	expect(container.textContent).toContain("Show fewer pages");
});

test("attachment summaries use German singular and plural names without changing filenames", async () => {
	const types = [
		"image",
		"audio",
		"audio",
		"document",
		"document",
		"document",
		"document",
	] as const;
	const files: ProcessedAttachment[] = types.map((type, index) => ({
		type,
		url: `https://files.example/${index}`,
		displayName: `Document ${index}`,
		name: `Document ${index}`,
		ext: "",
		isDataUrl: false,
	}));
	const { container, i18n } = await render(
		<AttachmentStrip files={files} onFileClick={() => {}} />,
	);
	expect(container.textContent).toContain("1 Bild · 2 Aufnahmen · 4 Dateien");
	await act(async () => {
		await i18n.changeLanguage("en");
	});
	expect(container.textContent).toContain("1 image · 2 recordings · 4 files");
	await act(async () => container.querySelector("button")?.click());
	expect(container.textContent).toContain("Document 6");
});

test("saved tool labels follow the provider language without changing model text or stored activity", async () => {
	const steps: IPlanStep[] = [
		{
			id: "file",
			title: "↳ Using read_file",
			toolName: "read_file",
			status: "done",
			description: "Check the customer's invoice",
			detail: buildToolActivity("read_file", {
				arguments: { path: "invoices/customer.txt" },
			}),
		},
		{
			id: "request",
			title: "Using mcp__tools__http_request",
			toolName: "mcp__tools__http_request",
			status: "done",
			detail: buildToolActivity("mcp__tools__http_request", {
				arguments: { method: "POST" },
				result: { http_status: 201 },
				result_summary: "The provider saved the invoice",
			}),
		},
		{
			id: "forecast",
			title: "Using forecast_tool",
			toolName: "forecast_tool",
			status: "done",
			detail: buildToolActivity("forecast_tool", {
				arguments: { operation: "run" },
			}),
		},
		{
			id: "authored",
			title: "Review the contract before sending",
			toolName: "read_file",
			status: "done",
			detail: buildToolActivity("read_file", {}),
		},
		{
			id: "custom",
			title: "Using custom_lookup",
			toolName: "custom_lookup",
			status: "done",
			detail: buildToolActivity("custom_lookup", {}),
		},
	];
	const saved = JSON.stringify(steps);
	const { container, i18n } = await render(
		steps.map((step) =>
			step.detail?.kind === "tool_activity" ? (
				<ToolActivityRow
					key={step.id}
					step={step}
					detail={step.detail}
					isActive={false}
					isExpanded={false}
					onToggle={() => {}}
					statusIcon={null}
					duration={null}
				/>
			) : null,
		),
	);
	expect(container.textContent).toContain("Datei wird gelesen");
	expect(container.textContent).toContain("HTTP-Anfrage wird gesendet");
	expect(container.textContent).toContain("Prognose wird ausgeführt");
	expect(container.textContent).toContain("Check the customer's invoice");
	expect(container.textContent).toContain("invoices/customer.txt");
	expect(container.textContent).toContain("POST");
	expect(container.textContent).toContain("HTTP 201");
	expect(container.textContent).toContain("The provider saved the invoice");
	expect(container.textContent).toContain("Review the contract before sending");
	expect(container.textContent).toContain("Custom lookup");
	await act(async () => {
		await i18n.changeLanguage("en");
	});
	expect(container.textContent).toContain("Reading a file");
	expect(container.textContent).toContain("Sending an HTTP request");
	expect(container.textContent).toContain("Running a forecast");
	expect(JSON.stringify(steps)).toBe(saved);
});

test("collapsed legacy tool titles and plan status summaries switch languages", async () => {
	const { container, i18n } = await render(
		<>
			<InlineStepGroup
				steps={[
					{
						id: "legacy",
						title: "Using mcp__tools__read_file",
						toolName: "mcp__tools__read_file",
						status: "done",
					},
				]}
			/>
			<PlanSteps
				steps={[
					{
						id: "data",
						title: "Create the customer table",
						status: "done",
						detail: { kind: "build_lane", lane: "data", target: "Customers" },
					},
					{
						id: "page",
						title: "Create a customer page",
						status: "failed",
						detail: { kind: "build_lane", lane: "page", target: "Directory" },
					},
				]}
			/>
		</>,
	);
	expect(container.textContent).toContain("Datei wird gelesen");
	expect(container.textContent).toContain(
		"2 Schritte · in 2 Bereichen · 1 nicht abgeschlossen",
	);
	await act(async () => container.querySelectorAll("button")[1]?.click());
	expect(container.textContent).toContain("Daten");
	expect(container.textContent).toContain("Seite");
	expect(
		container.querySelector('[aria-label="Abgeschlossen"]'),
	).not.toBeNull();
	expect(
		container.querySelector('[aria-label="Nicht abgeschlossen"]'),
	).not.toBeNull();
	await act(async () => {
		await i18n.changeLanguage("en");
	});
	expect(container.textContent).toContain("Reading a file");
	expect(container.textContent).toContain(
		"2 steps · across 2 lanes · 1 not completed",
	);
	expect(container.querySelector('[aria-label="Completed"]')).not.toBeNull();
});

test("earlier-step controls keep German word order when expanded", async () => {
	const { container } = await render(
		<PlanSteps
			turnActive
			steps={Array.from({ length: 6 }, (_, index) => ({
				id: `step-${index}`,
				title: `Authored step ${index}`,
				status: "done" as const,
			}))}
		/>,
	);
	expect(container.textContent).toContain("Wird bearbeitet…");
	const earlier = [...container.querySelectorAll("button")].find(
		(button) => button.textContent === "2 frühere Schritte anzeigen",
	);
	expect(earlier).toBeDefined();
	await act(async () => earlier?.click());
	expect(container.textContent).toContain("2 frühere Schritte ausblenden");
	expect(container.textContent).toContain("Authored step 0");
});
