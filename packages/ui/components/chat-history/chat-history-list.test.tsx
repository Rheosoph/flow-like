import { afterAll, describe, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { createRoot } from "react-dom/client";
import type { IHistoryEntry } from "./chat-history-types";

// The `lib` barrel imports back into components, so a direct unit import of the list re-enters this
// module graph. Stub the one helper the component actually uses.
mock.module("../../lib/utils", () => ({
	cn: (...classes: unknown[]) => classes.filter(Boolean).join(" "),
}));

afterAll(() => mock.restore());

const NOW = Date.now();

const ENTRIES: IHistoryEntry[] = [
	{ id: "a", title: "Deploy the pipeline", updatedAt: NOW - 1000 },
	{ id: "b", title: "Quarterly cost report", updatedAt: NOW - 3 * 86_400_000 },
	{
		id: "c",
		title: "Pinned onboarding notes",
		updatedAt: NOW - 400 * 86_400_000,
		pinnedAt: NOW - 5000,
	},
];

async function renderList(
	props: Partial<
		Parameters<typeof import("./chat-history-list").ChatHistoryList>[0]
	> = {},
) {
	const window = new Window();
	// happy-dom's Window does not carry the error constructors its own selector parser reaches for,
	// so querying inside this harness throws unless they are patched in.
	Object.assign(window, { SyntaxError, TypeError, Error });
	Object.assign(globalThis, {
		document: window.document,
		HTMLElement: window.HTMLElement,
		HTMLInputElement: window.HTMLInputElement,
		Node: window.Node,
		Event: window.Event,
		navigator: window.navigator,
		requestAnimationFrame: window.requestAnimationFrame.bind(window),
		cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
		getComputedStyle: window.getComputedStyle.bind(window),
		window,
	});
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

	const { ChatHistoryList } = await import("./chat-history-list");
	const container = window.document.createElement("div");
	window.document.body.append(container);
	const root = createRoot(container);

	await act(async () => {
		root.render(
			<ChatHistoryList entries={ENTRIES} onSelect={() => {}} {...props} />,
		);
	});

	const typeSearch = async (value: string) => {
		const input = container.getElementsByTagName("input")[0];
		if (!input) throw new Error("search input not rendered");
		// React's synthetic `input` → `onChange` bridge does not fire under a bare happy-dom Window,
		// so invoke the handler React actually mounted on the node. Same code path the real
		// keystroke takes, minus the event plumbing.
		const propsKey = Object.keys(input).find((key) =>
			key.startsWith("__reactProps$"),
		);
		const onChange = propsKey
			? (
					input as unknown as Record<
						string,
						{ onChange?: (e: unknown) => void }
					>
				)[propsKey]?.onChange
			: undefined;
		if (!onChange) throw new Error("search input has no React onChange");

		await act(async () => {
			onChange({ target: { value } });
		});
		// The query is debounced before it reaches the index.
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 260));
		});
	};

	const cleanup = async () => {
		await act(async () => root.unmount());
		window.close();
	};

	return { container, typeSearch, cleanup };
}

describe("ChatHistoryList", () => {
	test("groups pinned conversations first, out of their date bucket", async () => {
		const { container, cleanup } = await renderList();
		const headings = [...container.querySelectorAll("h3")].map(
			(h) => h.textContent?.replace(/\d+$/, "").trim() ?? "",
		);

		expect(headings[0]).toBe("Pinned");
		expect(headings).toContain("Today");
		// The pinned entry is 400 days old but must not also appear under "Older".
		expect(headings).not.toContain("Older");
		expect(container.querySelectorAll("li").length).toBe(3);

		await cleanup();
	});

	test("shows the no-match empty state when a search returns nothing", async () => {
		const { container, typeSearch, cleanup } = await renderList();
		await typeSearch("zzzzqqq");

		expect(container.querySelectorAll("li").length).toBe(0);
		// Regression: an always-present "Results" group made this state unreachable, leaving the
		// user staring at a bare "RESULTS 0" header with no way out.
		expect(container.textContent).toContain("No matches");
		expect(container.textContent).toContain("Clear search");

		await cleanup();
	});

	test("ranks matches and drops non-matching conversations", async () => {
		const { container, typeSearch, cleanup } = await renderList();
		await typeSearch("cost");

		const titles = [...container.querySelectorAll("li")].map(
			(row) => row.textContent ?? "",
		);
		expect(titles.length).toBe(1);
		expect(titles[0]).toContain("Quarterly cost report");

		await cleanup();
	});

	test("reports search activity so callers can defer loading message bodies", async () => {
		const seen: boolean[] = [];
		const { typeSearch, cleanup } = await renderList({
			onSearchActiveChange: (active) => seen.push(active),
		});

		expect(seen).toEqual([false]);
		await typeSearch("de");
		expect(seen.at(-1)).toBe(true);

		// Unmounting mid-search must release it, or the caller keeps a live Dexie subscription for
		// the rest of the session.
		await cleanup();
		expect(seen.at(-1)).toBe(false);
	});
});
