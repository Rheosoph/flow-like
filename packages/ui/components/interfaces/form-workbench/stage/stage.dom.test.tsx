import {
	afterAll,
	afterEach,
	describe,
	expect,
	setDefaultTimeout,
	setSystemTime,
	test,
} from "bun:test";
import type { FormSessionState, RunEntry } from "../contracts";
import {
	advance,
	allByRole,
	allByText,
	byRole,
	byText,
	click,
	keyDown,
	mountWorkbench,
	queryByRole,
} from "../testing/dom";
import { fakeView } from "../testing/fake-actions";
import { type FixtureName, fixture, fixtureClock } from "../testing/fixtures";
import {
	DESKTOP_LAYOUT,
	PHONE_LAYOUT,
	layoutFor,
	withLayout,
} from "../testing/layouts";
import { installStageDom } from "./dom-setup";

setDefaultTimeout(60_000);

const dom = installStageDom();
const { Stage } = await import("./stage");

afterEach(async () => {
	setSystemTime();
	await dom.cleanup();
});
afterAll(dom.restore);

type Options = {
	layout?: typeof DESKTOP_LAYOUT;
	routeLabels?: Record<string, string>;
	patch?: (state: FormSessionState) => FormSessionState;
};

/** One run's pane (bar, links, body), so a word is looked up there and not in a tab of the same run. */
function paneOf(runId: string) {
	const pane = document.querySelector<HTMLElement>(`[data-fw-pane="${runId}"]`);
	if (!pane) throw new Error(`no pane for ${runId}`);
	return pane;
}

async function mount(name: FixtureName, options: Options = {}) {
	setSystemTime(new Date(fixtureClock(name).now));
	const layout = options.layout ?? DESKTOP_LAYOUT;
	const state = withLayout(
		options.patch?.(fixture(name)) ?? fixture(name),
		layout,
	);
	const view = fakeView(state, { layout, routeLabels: options.routeLabels });
	const mounted = await mountWorkbench(<Stage {...view.props} />, { layout });
	return { view, mounted, state };
}

const withOverlay =
	(overlay: FormSessionState["view"]["overlay"]) =>
	(state: FormSessionState): FormSessionState => ({
		...state,
		view: { ...state.view, overlay },
	});

describe("empty stage", () => {
	test("a form with inputs and no run says what to do", async () => {
		await mount("idle");
		expect(
			byText(
				"Fill in the inputs and press Run. Each run opens here in its own tab.",
			),
		).toBeTruthy();
		expect(queryByRole("tablist")).toBeNull();
	});

	test("the phone's Output pane says it shortly", async () => {
		await mount("idle", { layout: PHONE_LAYOUT });
		expect(byText("Nothing has run yet")).toBeTruthy();
		expect(byText("Fill in the inputs and press Run.")).toBeTruthy();
	});

	test("a form without inputs leaves its first-run card to the shell", async () => {
		const { mounted } = await mount("none");
		expect(mounted.container.innerHTML).toBe("");
	});

	test("while the first run's files send, the stage says so and the strip lists it", async () => {
		await mount("uploading");
		expect(
			byText("Your first run opens here once the files are sent."),
		).toBeTruthy();
		const tab = byRole("tab", /Run 1/);
		expect(tab.getAttribute("aria-selected")).toBe("false");
		expect(tab.getAttribute("tabindex")).toBe("0");
	});
});

describe("run strip", () => {
	test("tabs newest first, the selected one marked, the rest behind N more", async () => {
		await mount("done");
		const tabs = allByRole("tab");
		expect(tabs[0]?.textContent).toContain("Run 14");
		expect(tabs[0]?.getAttribute("aria-selected")).toBe("true");
		expect(tabs[0]?.getAttribute("tabindex")).toBe("0");
		expect(tabs[1]?.getAttribute("tabindex")).toBe("-1");
		expect(tabs.length).toBe(6);
		expect(byRole("button", /8 more/)).toBeTruthy();
	});

	test("each tab names its status for a screen reader and carries the focus hook", async () => {
		await mount("failed");
		const tab = byRole("tab", /Run 13/);
		expect(tab.getAttribute("aria-label")).toBe("Run 13, Failed, 12 s");
		expect(tab.getAttribute("data-fw-focus")).toBe("tab:run-13");
	});

	test("the selected tab controls its pane, which is the tabpanel", async () => {
		await mount("done");
		const tab = byRole("tab", /Run 14/);
		const panel = byRole("tabpanel");
		expect(tab.getAttribute("aria-controls")).toBe(panel.id);
		expect(panel.getAttribute("aria-labelledby")).toBe(tab.id);
	});

	test("clicking a tab selects that run", async () => {
		const { view } = await mount("done");
		await click(byRole("tab", /Run 13/));
		expect(view.argsOf("selectRun")).toEqual([["run-13", "tab"]]);
	});

	test("arrow keys, Home and End move between tabs and the stage follows", async () => {
		const { view } = await mount("done");
		const first = byRole("tab", /Run 14/);
		first.focus();
		await keyDown(first, "ArrowRight");
		expect(view.argsOf("selectRun")).toEqual([["run-13", "keyboard"]]);
		await keyDown(first, "End");
		expect(view.argsOf("selectRun").at(-1)).toEqual(["run-9", "keyboard"]);
		await keyDown(byRole("tab", /Run 13/), "Home");
		expect(view.argsOf("selectRun").at(-1)).toEqual(["run-14", "keyboard"]);
	});

	test("a key during an IME composition does nothing", async () => {
		const { view } = await mount("done");
		const first = byRole("tab", /Run 14/);
		await keyDown(first, "ArrowRight", { isComposing: true });
		expect(view.argsOf("selectRun")).toEqual([]);
	});

	test("the overflow menu lists the earlier runs and picks one", async () => {
		const { view } = await mount("done", {
			patch: withOverlay({ id: "moreRuns" }),
		});
		const menu = byRole("menu");
		const items = allByRole("menuitem", undefined, menu);
		expect(items.length).toBe(8);
		await click(items[0] as HTMLElement);
		expect(view.argsOf("selectRun")[0]?.[1]).toBe("menu");
	});

	test("opening the overflow asks for its overlay", async () => {
		const { view } = await mount("done");
		await click(byRole("button", /8 more/));
		expect(view.argsOf("openOverlay")).toEqual([[{ id: "moreRuns" }]]);
	});

	test("an unseen failure behind the overflow is named on its button", async () => {
		await mount("series-failed");
		const button = byRole("button", /more/);
		expect(button.textContent).toContain("1 failed");
	});

	test("the pin sits on the selected tab and pins the run", async () => {
		const { view } = await mount("done");
		const pin = byRole("button", "Pin run 14 to compare");
		expect(pin.getAttribute("aria-pressed")).toBe("false");
		await click(pin);
		expect(view.argsOf("pinRun")).toEqual([["run-14"]]);
	});

	test("a pinned run shows its pin pressed and unpins", async () => {
		const { view } = await mount("compare");
		const pin = byRole("button", "Unpin run 12", undefined);
		expect(pin.getAttribute("aria-pressed")).toBe("true");
		await click(pin);
		expect(view.argsOf("pinRun")).toEqual([[null]]);
	});

	test("a queued tab has no pin and reads Queued", async () => {
		await mount("queued");
		const tab = byRole("tab", /Run 18/);
		expect(tab.textContent).toContain("Queued");
		expect(queryByRole("button", "Pin run 18 to compare")).toBeNull();
	});
});

describe("run bar", () => {
	test("running: the word, the step, a clock hidden from screen readers, and Stop", async () => {
		const { view } = await mount("running");
		const pane = paneOf("run-14");
		expect(byText("Running", pane)).toBeTruthy();
		expect(pane.textContent).toContain("Step 4: Match purchase order");
		const clock = byText("0:31", pane);
		expect(clock.closest("[aria-hidden='true']")).toBeTruthy();
		await click(byRole("button", "Stop run 14"));
		expect(view.argsOf("stop")).toEqual([["run-14"]]);
	});

	test("the Stop button carries its shortcut in the title", async () => {
		await mount("running");
		expect(byRole("button", "Stop run 14").getAttribute("title")).toBe(
			"Stop run 14 · ⌘.",
		);
	});

	test("the Stop square is the canvas's 12-unit rect, the one the dock draws too", async () => {
		await mount("running");
		const svg = byRole("button", "Stop run 14").querySelector("svg");
		expect(svg?.getAttribute("viewBox")).toBe("0 0 24 24");
		expect(svg?.getAttribute("aria-hidden")).toBe("true");
		const rect = svg?.querySelector("rect");
		expect(
			["width", "height", "x", "y", "rx"].map((name) =>
				rect?.getAttribute(name),
			),
		).toEqual(["12", "12", "6", "6", "1.5"]);
	});

	test("done: how long it took, Copy answer, no Run again while the rail matches", async () => {
		await mount("done");
		expect(byText("Done in 48 s")).toBeTruthy();
		expect(byRole("button", "Copy answer")).toBeTruthy();
		expect(queryByRole("button", "Run again")).toBeNull();
		expect(queryByRole("button", "Use these inputs")).toBeNull();
	});

	test("Copy answer copies the answer and says Copied", async () => {
		await mount("done");
		await click(byRole("button", "Copy answer"));
		expect(dom.clipboard.at(-1)).toContain("Invoice RE-2026-0917 extracted");
		expect(byRole("button", "Copied")).toBeTruthy();
	});

	test("Copy answer keeps the answer as written: the no-break ties are for the screen only", async () => {
		await mount("done");
		const shown = paneOf("run-14").textContent ?? "";
		expect(shown).toContain("17\u{a0}Oct\u{a0}2026");
		await click(byRole("button", "Copy answer"));
		const copied = dom.clipboard.at(-1) ?? "";
		expect(copied).toContain("17 Oct 2026");
		expect(copied).not.toContain("\u{a0}");
	});

	test("failed: Try again re-runs the run's own copy", async () => {
		const { view } = await mount("failed");
		await click(byRole("button", "Try again"));
		expect(view.argsOf("runAgain")).toEqual([["run-13"]]);
	});

	test("a held Enter never presses Try again twice", async () => {
		await mount("failed");
		const button = byRole("button", "Try again");
		let prevented = false;
		await (async () => {
			const view = button.ownerDocument
				.defaultView as unknown as typeof globalThis;
			const event = new view.KeyboardEvent("keydown", {
				key: "Enter",
				repeat: true,
				bubbles: true,
				cancelable: true,
			});
			button.dispatchEvent(event);
			prevented = event.defaultPrevented;
		})();
		expect(prevented).toBe(true);
	});

	test("stopped: Run again and the stopped clock", async () => {
		const { view } = await mount("stopped");
		expect(byText("Stopped at 0:37")).toBeTruthy();
		await click(byRole("button", "Run again"));
		expect(view.argsOf("runAgain")).toEqual([["run-14"]]);
	});

	test("an edited rail adds Run again, Use these inputs and the warning chip", async () => {
		const { view } = await mount("done", {
			patch: (state) => ({
				...state,
				rail: {
					...state.rail,
					values: { ...state.rail.values, vendor_name: "Alpenfracht AG" },
				},
			}),
		});
		expect(byRole("button", "Inputs edited since this run")).toBeTruthy();
		await click(byRole("button", "Run again"));
		expect(view.argsOf("runAgain")).toEqual([["run-14"]]);
		await click(byRole("button", "Use these inputs"));
		expect(view.argsOf("useInputs")).toEqual([["run-14"]]);
	});

	test("the change chips name what changed against the run before", async () => {
		await mount("running");
		expect(byText("Changed since run 13")).toBeTruthy();
		const chip = byText("Run OCR", paneOf("run-14"))
			.parentElement as HTMLElement;
		expect(chip.textContent).toContain("Off");
		expect(chip.textContent).toContain("On");
	});

	test("queued: the place in line, the note and Remove from queue", async () => {
		const { view } = await mount("queued");
		expect(byText(/1st in line/)).toBeTruthy();
		expect(
			byText(
				"This device runs 3 at a time from this window. This one starts by itself when one of them ends.",
			),
		).toBeTruthy();
		await click(byRole("button", "Remove from queue"));
		expect(view.argsOf("removeFromQueue")).toEqual([["run-18"]]);
		expect(queryByRole("tablist", "Sections of run 18")).toBeNull();
	});

	test("sending: starts when its files are sent, with Stop", async () => {
		await mount("uploading", {
			patch: (state) => ({
				...state,
				view: { ...state.view, selectedRunId: "run-1" },
			}),
		});
		expect(byText(/starts when its files are sent/)).toBeTruthy();
		expect(byRole("button", "Stop run 1")).toBeTruthy();
	});

	test("a run that was not started says why and offers Try again for its file", async () => {
		const { view } = await mount("done", {
			patch: (state) => {
				const run = state.runs[0] as RunEntry;
				const notStarted: RunEntry = {
					...run,
					status: "notStarted",
					startedAt: null,
					endedAt: null,
					output: null,
					outcome: { kind: "notStarted", reason: "fileNotSent" },
				};
				return { ...state, runs: [notStarted, ...state.runs.slice(1)] };
			},
		});
		expect(byText(/Not started/)).toBeTruthy();
		expect(byText(/its file was not sent/)).toBeTruthy();
		await click(byRole("button", "Try again"));
		expect(view.argsOf("runAgain")).toEqual([["run-14"]]);
	});

	test("asking: the question waits in its own section", async () => {
		await mount("asking");
		expect(byText("Waiting for you")).toBeTruthy();
		expect(byText("Line 4 is not on the purchase order")).toBeTruthy();
		expect(byText(/Waiting for your answer/)).toBeTruthy();
	});
});

describe("run menu", () => {
	const open = withOverlay({ id: "runMenu", runId: "run-14" });

	test("lists pin, Use these inputs, save as a preset and remove", async () => {
		await mount("done", { patch: open });
		const menu = byRole("menu");
		const labels = allByRole("menuitem", undefined, menu).map(
			(item) => item.textContent,
		);
		expect(labels).toEqual([
			"Pin to compare",
			"Use these inputs",
			"Save these inputs as a preset…",
			"Remove from this device",
		]);
	});

	test("Use these inputs loads the run's copy", async () => {
		const { view } = await mount("done", { patch: open });
		await click(byRole("menuitem", "Use these inputs"));
		expect(view.argsOf("useInputs")).toEqual([["run-14"]]);
	});

	test("Save as a preset opens the save dialog for that run", async () => {
		const { view } = await mount("done", { patch: open });
		await click(byRole("menuitem", "Save these inputs as a preset…"));
		expect(view.argsOf("openOverlay")).toContainEqual([
			{ id: "presetSave", mode: "save", fromRunId: "run-14" },
		]);
		expect(view.argsOf("closeOverlay")).toEqual([]);
	});

	test("Remove deletes the run from this device", async () => {
		const { view } = await mount("done", { patch: open });
		await click(byRole("menuitem", "Remove from this device"));
		expect(view.argsOf("removeRun")).toEqual([["run-14"]]);
	});

	test("an older run whose files are gone shows Run again disabled with its reason", async () => {
		const { state } = await mount("reopen", {
			patch: withOverlay({ id: "runMenu", runId: "run-14" }),
		});
		expect(state.runs[0]?.origin).toBe("history");
		const repeat = byRole("menuitem", "Run again");
		expect(
			repeat.getAttribute("aria-disabled") ??
				repeat.getAttribute("data-disabled"),
		).not.toBeNull();
		expect(
			byText(
				"Its files can no longer be sent. Use these inputs and pick them again.",
			),
		).toBeTruthy();
	});

	test("the menu button names its run and asks for that run's menu", async () => {
		const { view } = await mount("done");
		const button = byRole("button", "More actions for run 14");
		expect(button.getAttribute("aria-expanded")).toBe("false");
		expect(queryByRole("menu")).toBeNull();
		await click(button);
		expect(view.argsOf("openOverlay")).toEqual([
			[{ id: "runMenu", runId: "run-14" }],
		]);
	});

	test("a saved run whose object property was not kept offers Use these inputs and says why", async () => {
		const { state } = await mount("reopen", {
			patch: (base) => {
				const run = base.runs[0] as RunEntry;
				const terms = base.form.fields.find((field) => field.kind === "group");
				if (!terms) throw new Error("the reopen form has no object field");
				const [first, second] = terms.props;
				const files = new Set(
					base.form.fields
						.filter((field) => field.kind === "file" || field.kind === "files")
						.map((field) => field.name),
				);
				const values = Object.fromEntries(
					Object.entries(run.copy.values).filter(([name]) => !files.has(name)),
				);
				const hidden: RunEntry = {
					...run,
					status: "stopped",
					copy: {
						...run.copy,
						values: {
							...values,
							[terms.name]: {
								[first.name]: "EUR",
								[second.name]: { $hidden: true },
							},
						},
					},
				};
				return withOverlay({ id: "runMenu", runId: run.id })({
					...base,
					runs: [hidden, ...base.runs.slice(1)],
				});
			},
		});
		const pane = paneOf(state.runs[0]?.id ?? "");
		expect(queryByRole("button", "Run again", pane)).toBeNull();
		expect(byRole("button", "Use these inputs", pane)).toBeTruthy();
		expect(
			byText(
				"Some of its inputs are not saved on this device. Use these inputs and enter them again.",
			),
		).toBeTruthy();
	});
});

describe("body", () => {
	test("done: Steps folded, the answer, the result, the files, the inputs", async () => {
		await mount("done");
		expect(byText("6 steps finished")).toBeTruthy();
		expect(byText("Show steps")).toBeTruthy();
		expect(byText("Invoice RE-2026-0917 extracted")).toBeTruthy();
		expect(byText("Same as the form")).toBeTruthy();
		const links = allByRole(
			"button",
			undefined,
			byRole("navigation", "Sections of run 14"),
		);
		expect(links.map((link) => link.textContent)).toEqual([
			"Steps",
			"Answer",
			"Result",
			"Files12",
			"Inputs",
		]);
	});

	test("Show steps opens the list and Hide steps folds it again", async () => {
		await mount("done");
		await click(byText("Show steps"));
		expect(byText("Read documents")).toBeTruthy();
		await click(byText("Hide steps"));
		expect(byText("6 steps finished")).toBeTruthy();
	});

	test("failed: the failure card with the step, Details, and the steps open", async () => {
		await mount("failed");
		expect(
			byText("The run failed at “Run OCR”. No files were saved."),
		).toBeTruthy();
		expect(
			byText("Try again with the same inputs, or change one and press Run."),
		).toBeTruthy();
		expect(byText("Failed", paneOf("run-13"))).toBeTruthy();
		const details = byRole("button", "Details");
		expect(details.getAttribute("aria-expanded")).toBe("false");
		await click(details);
		expect(details.getAttribute("aria-expanded")).toBe("true");
		expect(
			byText(/Failed to execute node: Array value is not an array/),
		).toBeTruthy();
		expect(byText(/run_7f3a91c2/)).toBeTruthy();
	});

	test("stopped: the quiet note and the step marked Stopped", async () => {
		await mount("stopped");
		expect(byText("You stopped this run during step 4.")).toBeTruthy();
		expect(byText("Stopped", paneOf("run-14"))).toBeTruthy();
	});

	test("running: the follow marker adds no gap under the steps; the Inputs label row keeps 28 px", async () => {
		await mount("running");
		const steps = document.querySelector('[data-sec="steps"]');
		const tail = steps?.querySelector("[data-fw-tail]");
		expect(tail).toBeTruthy();
		expect(tail?.parentElement).not.toBe(steps);
		const inputs = document.querySelector('[data-sec="inputs"]');
		expect(inputs?.firstElementChild?.className).toContain("min-h-7");
		expect(steps?.firstElementChild?.className).toContain("min-h-5");
	});

	test("a note box has a solid edge in the unknown tone, never the devices' dashed one", async () => {
		await mount("stopped");
		const box = byText("You stopped this run during step 4.").closest(
			".rounded-lg",
		);
		expect(box?.className).toContain("border-unknown-line");
		expect(box?.className).not.toContain("border-dashed");
	});

	test("this session's run whose stream ended without a result says so, not that the form closed", async () => {
		await mount("done", {
			patch: (state) => {
				const run = state.runs[0] as RunEntry;
				const lost: RunEntry = {
					...run,
					status: "unknown",
					outcome: { kind: "unknown" },
					output: null,
				};
				return { ...state, runs: [lost, ...state.runs.slice(1)] };
			},
		});
		expect(
			byText("The connection ended before this run reported a result."),
		).toBeTruthy();
		expect(
			allByText(
				"The form was closed while this run was going, so how it ended is not known.",
			),
		).toEqual([]);
	});

	test("running without output yet: nothing came back, and a slow-run line after 10 s", async () => {
		await mount("running", {
			patch: (state) => {
				const run = state.runs[0] as RunEntry;
				const bare: RunEntry = {
					...run,
					output: run.output && { ...run.output, steps: [] },
				};
				return { ...state, runs: [bare, ...state.runs.slice(1)] };
			},
		});
		expect(byText("Nothing has come back yet.")).toBeTruthy();
		expect(byText("Still running. This can take a while.")).toBeTruthy();
	});

	test("the Inputs section reads each value, with a bar where the rail differs", async () => {
		await mount("done", {
			patch: (state) => ({
				...state,
				rail: {
					...state.rail,
					values: { ...state.rail.values, vendor_name: "Alpenfracht AG" },
				},
			}),
		});
		expect(byText("Nordwind Logistik GmbH")).toBeTruthy();
		expect(byText("17 Sep 2026")).toBeTruthy();
		expect(byText("4400, 4410")).toBeTruthy();
		const bars = document.querySelectorAll(
			"[data-sec='inputs'] span[aria-hidden='true']",
		);
		expect(bars.length).toBe(1);
		expect(paneOf("run-14").textContent).not.toContain("Same as the form");
	});

	test("a finished run offers the form's routes through the leave guard", async () => {
		const { view } = await mount("none-done", {
			routeLabels: { "/support": "Support chat" },
		});
		const go = byRole("button", /Go to/);
		expect(go.textContent).toContain("Go to");
		await click(go);
		expect(view.routes.gone.length).toBe(1);
	});

	test("the centred route button is the canvas's 36 px one, 44 px on a touch screen", async () => {
		await mount("none-done");
		const go = byRole("button", /Go to/);
		for (const size of ["h-9", "px-3.5", "text-[13.5px]", "has-[>svg]:px-3.5"])
			expect(go.className).toContain(size);
		expect(go.className).not.toMatch(/(^| )h-8( |$)/);
		await dom.cleanup();
		await mount("none-done", { layout: PHONE_LAYOUT });
		const touch = byRole("button", /Go to/);
		expect(touch.className).toMatch(/(^| )h-11( |$)/);
		expect(touch.className).not.toMatch(/(^| )h-9( |$)/);
	});

	test("a form without inputs shows its name and 'This run returned nothing.' in one card", async () => {
		await mount("none-done");
		expect(byText("Triage selected request")).toBeTruthy();
		expect(byText("This run returned nothing.")).toBeTruthy();
		expect(queryByRole("navigation")).toBeNull();
	});
});

describe("compare", () => {
	test("two panes, each bar names its run, and the table of differing inputs", async () => {
		await mount("compare");
		expect(byText("3 inputs differ")).toBeTruthy();
		const table = byRole("table");
		expect(table.textContent).toContain("invoice-RE-2026-0911.pdf");
		expect(table.textContent).toContain("invoice-RE-2026-0917.pdf");
		expect(table.textContent).toContain("Run 12 · pinned");
		expect(allByRole("tabpanel").length).toBe(1);
		const sections = document.querySelectorAll("[data-fw-pane]");
		expect(sections.length).toBe(2);
		expect(paneOf("run-12").textContent).toContain("Run 12");
		expect(paneOf("run-12").textContent).toContain("Done in 41 s");
		expect(paneOf("run-14").textContent).toContain("Done in 48 s");
	});

	test("Unpin run 12 in the header unpins", async () => {
		const { view } = await mount("compare");
		const buttons = allByRole("button", "Unpin run 12");
		await click(
			buttons.find((b) => b.textContent === "Unpin run 12") as HTMLElement,
		);
		expect(view.argsOf("pinRun")).toEqual([[null]]);
	});

	test("the pinned run is the one on the stage: a hint instead of a second pane", async () => {
		await mount("compare", {
			patch: (state) => ({
				...state,
				view: { ...state.view, selectedRunId: state.view.pinnedRunId },
			}),
		});
		expect(
			byText(
				"Run 12 is pinned. Pick another run, or change an input and press Run, to see the two side by side.",
			),
		).toBeTruthy();
	});

	test("a 960 px box names the pinned run in a line with Unpin", async () => {
		await mount("compare", { layout: layoutFor(960, 836) });
		expect(byText("Run 12 is pinned")).toBeTruthy();
		expect(byText("Widen the window to see it beside run 14.")).toBeTruthy();
		expect(document.querySelectorAll("[data-fw-pane]").length).toBe(1);
	});
});

describe("phone", () => {
	test("a row of chips, one per run, the selected one marked", async () => {
		await mount("done", { layout: PHONE_LAYOUT });
		const chips = allByRole("tab");
		expect(chips.length).toBe(14);
		expect(chips[0]?.getAttribute("aria-selected")).toBe("true");
		expect(queryByRole("button", /more/)).toBeNull();
	});

	test("Copy answer moves into the run menu", async () => {
		await mount("done", {
			layout: PHONE_LAYOUT,
			patch: withOverlay({ id: "runMenu", runId: "run-14" }),
		});
		expect(queryByRole("button", "Copy answer")).toBeNull();
		expect(byRole("menuitem", "Copy the answer")).toBeTruthy();
	});

	test("controls are 44 px on a touch screen", async () => {
		await mount("failed", { layout: PHONE_LAYOUT });
		expect(byRole("button", "Try again").className).toContain("h-11");
	});
});

describe("clock", () => {
	test("a finished stage schedules no timer", async () => {
		const timers: unknown[] = [];
		const realSet = globalThis.setInterval;
		globalThis.setInterval = ((...args: Parameters<typeof setInterval>) => {
			timers.push(args);
			return realSet(...args);
		}) as typeof setInterval;
		try {
			await mount("done");
			expect(timers.length).toBe(0);
		} finally {
			globalThis.setInterval = realSet;
		}
	});

	test("a live run ticks once a second and the stage unmounts cleanly", async () => {
		const { mounted } = await mount("running");
		const pane = paneOf("run-14");
		expect(byText("0:31", pane)).toBeTruthy();
		setSystemTime(new Date(fixtureClock("running").now + 5_000));
		await advance(1_100);
		expect(byText("0:36", pane)).toBeTruthy();
		await mounted.unmount();
	});
});

describe("sections and links", () => {
	test("at the top the first link is current and a click makes another current", async () => {
		await mount("done");
		const nav = byRole("navigation", "Sections of run 14");
		const links = allByRole("button", undefined, nav);
		expect(links[0]?.getAttribute("aria-current")).toBe("true");
		await click(links[3] as HTMLElement);
		expect(links[3]?.getAttribute("aria-current")).toBe("true");
		expect(links[0]?.getAttribute("aria-current")).toBeNull();
	});

	test("a run with one section has no links, only a hairline", async () => {
		await mount("small", {
			patch: (state) => state,
		});
		expect(queryByRole("navigation")).toBeNull();
	});

	test("the files section lists names, sizes and a Save button per file", async () => {
		await mount("done");
		const files = document.querySelector("[data-sec='files']") as HTMLElement;
		expect(files.textContent).toContain("Save all");
		expect(files.textContent).toContain("reconciliation.xlsx");
		expect(files.textContent).toContain("47.5 KB");
		expect(byRole("button", "Save reconciliation.xlsx", files)).toBeTruthy();
		expect(byRole("button", "Open page-01-header.png", files)).toBeTruthy();
	});

	test("a hosted page that only stopped listening says so", async () => {
		await mount("stopped", {
			patch: (state) => ({
				...state,
				form: {
					...state.form,
					host: { ...state.form.host, stop: "detach" as const },
				},
			}),
		});
		expect(byText("You stopped waiting for this run.")).toBeTruthy();
		expect(
			byText(
				/It may still finish on the server, but this page will not show it\./,
			),
		).toBeTruthy();
	});

	test("a form without inputs says 'Press Run again' when a run failed", async () => {
		await mount("none-done", {
			patch: (state) => {
				const run = state.runs[0] as RunEntry;
				const failed: RunEntry = {
					...run,
					status: "failed",
					output: run.output && {
						...run.output,
						steps: [
							{
								id: "s1",
								number: 1,
								title: "Fetch",
								detail: null,
								message: null,
								state: "failed",
							},
						],
					},
					outcome: {
						kind: "failed",
						failure: "flow",
						message: "Boom",
						detail: "Boom",
					},
				};
				return { ...state, runs: [failed] };
			},
		});
		expect(byText("Press Run again to try again.")).toBeTruthy();
		expect(
			byText("The run failed at “Fetch”. No files were saved."),
		).toBeTruthy();
	});
});
