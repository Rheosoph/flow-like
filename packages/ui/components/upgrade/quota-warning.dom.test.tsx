import { afterAll, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { createRoot } from "react-dom/client";
import type { QuotaOverview } from "../../lib/quota";

const notices: {
	type: string;
	title: string;
	description: string;
	action: { onClick: () => void };
}[] = [];
const capture =
	(type: string) =>
	(
		title: string,
		options: { description: string; action: { onClick: () => void } },
	) =>
		notices.push({ type, title, ...options });
mock.module("sonner", () => ({
	toast: { warning: capture("warning"), info: capture("info") },
}));
afterAll(() => mock.restore());

test("quota warnings deduplicate thresholds, ignore temporary reservations and rearm storage", async () => {
	const window = new Window({ url: "https://example.com" });
	Object.assign(window, { SyntaxError, TypeError, Error });
	Object.assign(globalThis, {
		window,
		document: window.document,
		HTMLElement: window.HTMLElement,
		Node: window.Node,
		Event: window.Event,
		navigator: window.navigator,
		localStorage: window.localStorage,
		IS_REACT_ACT_ENVIRONMENT: true,
	});
	const { QuotaWarnings } = await import("./quota-warning");
	const container = window.document.createElement("div");
	window.document.body.append(container);
	const root = createRoot(container as unknown as HTMLElement);
	const data: QuotaOverview = {
		plan: "FREE",
		payerId: "warning-test",
		periodStart: "2026-09-01",
		periodEnd: "2026-10-01",
		updatedAt: "2026-09-13",
		usage: [],
		resources: [
			{
				resource: "cloud_runtime_ms",
				used: 10,
				reserved: 90,
				limit: 100,
				remaining: 0,
				unit: "milliseconds",
				threshold: 0,
			},
		],
	};
	await act(async () => root.render(<QuotaWarnings overview={data} />));
	expect(notices).toHaveLength(0);
	const warn = {
		...data,
		resources: [{ ...data.resources[0], used: 75, reserved: 0, threshold: 75 }],
	};
	await act(async () => root.render(<QuotaWarnings overview={warn} />));
	await act(async () => root.render(<QuotaWarnings overview={{ ...warn }} />));
	expect(notices).toHaveLength(1);
	expect(notices[0].type).toBe("info");
	const storage = {
		...data,
		resources: [
			{
				...data.resources[0],
				resource: "storage_bytes",
				used: 90,
				reserved: 0,
				threshold: 90,
			},
		],
	};
	await act(async () => root.render(<QuotaWarnings overview={storage} />));
	await act(async () =>
		root.render(
			<QuotaWarnings overview={{ ...storage, periodStart: "2026-10-01" }} />,
		),
	);
	expect(notices).toHaveLength(2);
	await act(async () =>
		root.render(
			<QuotaWarnings
				overview={{
					...storage,
					resources: [{ ...storage.resources[0], used: 20, threshold: 0 }],
				}}
			/>,
		),
	);
	await act(async () => root.render(<QuotaWarnings overview={storage} />));
	expect(notices).toHaveLength(3);
	expect(notices[2].type).toBe("warning");
	expect(notices[2].description).not.toContain("Renews");
	const serverNotice = {
		...data,
		plan: "PREMIUM",
		resources: [
			{
				...data.resources[0],
				resource: "hosted_ai_cost_micros",
				used: 2_250_000,
				limit: 3_000_000,
				threshold: 75,
			},
		],
		warnings: [
			{
				id: "server-warning-ai-75",
				resource: "hosted_ai_cost_micros",
				threshold: 75,
				episode: 1,
				title: "PREMIUM long server message",
				description: "Renews 2026-10-01T00:00:00.000Z. Long explanation.",
			},
		],
	};
	await act(async () => root.render(<QuotaWarnings overview={serverNotice} />));
	await act(async () =>
		root.render(<QuotaWarnings overview={{ ...serverNotice }} />),
	);
	expect(notices).toHaveLength(4);
	expect(notices[3].type).toBe("info");
	expect(notices[3].title).toContain("Premium plan");
	expect(notices[3].description).toContain("2.25");
	expect(notices[3].description).toContain("3.00");
	expect(notices[3].description).not.toContain("T00:");
	expect(notices[3].description).not.toContain("Long explanation");
	const reached = {
		...serverNotice,
		warnings: [
			{
				...serverNotice.warnings[0],
				id: "server-warning-ai-100",
				threshold: 100,
			},
		],
		resources: [
			{ ...serverNotice.resources[0], used: 3_000_000, threshold: 100 },
		],
	};
	await act(async () => root.render(<QuotaWarnings overview={reached} />));
	expect(notices[4].type).toBe("warning");
	expect(notices[4].title).toContain("reached");
	notices[4].action.onClick();
	expect(window.location.pathname + window.location.search).toBe(
		"/subscription?tab=usage",
	);
	await act(async () => root.unmount());
	window.happyDOM.abort();
});
