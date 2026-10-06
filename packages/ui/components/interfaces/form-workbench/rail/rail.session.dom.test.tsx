import { afterAll, afterEach, describe, expect, mock, test } from "bun:test";
import { type ReactNode, useMemo, useState } from "react";
import type {
	FieldControlProps,
	FormSessionActions,
	FormSessionState,
	InterfaceModalProps,
	Preset,
	SessionCommand,
	WorkbenchLayout,
} from "../contracts";
import { reduceSession } from "../session/reduce";
import {
	byRole,
	click,
	inPortal,
	installWorkbenchDom,
	keyDown,
	mountWorkbench,
	queryByRole,
	settle,
	typeInto,
} from "../testing/dom";
import { fakeRoutes } from "../testing/fake-actions";
import { fixture } from "../testing/fixtures";
import { DESKTOP_LAYOUT } from "../testing/layouts";

/*
 * The rail wired to the real session reducer (S-STATE): the actions it calls turn into commands, the
 * state the reducer returns is what the rail draws next. Field controls and the modal frame are
 * stand-ins, as in rail.dom.test.tsx. DOM files run one per process.
 */

mock.module("../fields/field-control", () => ({
	FieldControl: (props: FieldControlProps) => (
		<button type="button" data-fw-focus={`field:${props.field.key}`}>
			{props.field.label}
		</button>
	),
}));

mock.module("../shell/interface-modal", () => ({
	InterfaceModal: (props: InterfaceModalProps): ReactNode =>
		props.open ? (
			<dialog open aria-labelledby={props.labelledBy}>
				{props.children}
			</dialog>
		) : null,
}));

const dom = installWorkbenchDom();
const { Rail } = await import("./rail");

afterEach(() => dom.cleanup());
afterAll(() => dom.restore());

/** The commands the rail sends; every other action of the contract is a no-op here. */
const COMMANDS: Readonly<
	Partial<
		Record<keyof FormSessionActions, (...args: never[]) => SessionCommand>
	>
> = {
	setFilter: (filter: Partial<FormSessionState["rail"]["filter"]>) => ({
		type: "setFilter",
		filter,
	}),
	setRailTab: (tab: FormSessionState["rail"]["tab"]) => ({
		type: "setRailTab",
		tab,
	}),
	openOverlay: (overlay: NonNullable<FormSessionState["view"]["overlay"]>) => ({
		type: "openOverlay",
		overlay,
	}),
	closeOverlay: () => ({ type: "closeOverlay" }),
	applyPreset: (presetId: string | null) => ({ type: "applyPreset", presetId }),
	savePreset: (draft: never) => ({ type: "savePreset", draft }),
	updatePreset: (presetId: string) => ({ type: "updatePreset", presetId }),
	deletePreset: (presetId: string) => ({ type: "deletePreset", presetId }),
	resetToPreset: () => ({ type: "resetToPreset" }),
	resetAll: () => ({ type: "resetAll" }),
	selectRun: (runId: string | null, how: never) => ({
		type: "selectRun",
		runId,
		how,
	}),
	railKey: () => ({ type: "railKey" }),
	focusHandled: (seq: number) => ({ type: "focusHandled", seq }),
};

type Observe = (state: FormSessionState) => void;

function Live({
	initial,
	layout,
	observe,
}: Readonly<{
	initial: FormSessionState;
	layout: WorkbenchLayout;
	observe: Observe;
}>) {
	const [state, setState] = useState<FormSessionState>({ ...initial, layout });
	observe(state);
	const actions = useMemo(() => {
		const send = (command: SessionCommand) =>
			setState(
				(current) =>
					reduceSession(
						current,
						{ kind: "command", command },
						{ now: Date.now(), today: "2026-10-05" },
					).state,
			);
		return new Proxy({} as FormSessionActions, {
			get: (_target, name: string) => {
				const make = COMMANDS[name as keyof FormSessionActions];
				return (...args: never[]) => {
					if (make) send(make(...args));
				};
			},
		});
	}, []);
	return (
		<Rail
			state={state}
			actions={actions}
			layout={layout}
			routes={fakeRoutes()}
		/>
	);
}

async function live(
	initial: FormSessionState,
	layout: WorkbenchLayout = DESKTOP_LAYOUT,
) {
	let latest = initial;
	const mounted = await mountWorkbench(
		<Live
			initial={initial}
			layout={layout}
			observe={(state) => {
				latest = state;
			}}
		/>,
		{ layout },
	);
	return { ...mounted, state: () => latest };
}

const messageKind = (state: FormSessionState) =>
	state.view.message ? state.view.message.message.kind : null;

const cellNames = (container: HTMLElement) =>
	Array.from(container.querySelectorAll<HTMLElement>("[data-rail-field]")).map(
		(cell) => cell.dataset.railField,
	);

describe("presets through the real session", () => {
	const closed = () => {
		const state = fixture("presets");
		return { ...state, view: { ...state.view, overlay: null } };
	};

	test("the button opens the menu, a digit applies that preset and closes it", async () => {
		const view = await live(closed());
		await click(byRole("button", /Open presets/));
		expect(view.state().view.overlay?.id).toBe("presets");
		await settle();
		expect(inPortal("dialog")).toBeTruthy();
		await keyDown(document.activeElement as HTMLElement, "1");
		const next = view.state();
		expect(next.rail.activePresetId).toBe("preset-alpenfracht");
		expect(next.rail.values.vendor_name).toBe("Alpenfracht AG");
		expect(next.rail.values.max_pages).toBe("60");
		expect(next.view.overlay).toBeNull();
		expect(messageKind(next)).toBe("presetApplied");
		await settle();
		expect(queryByRole("dialog")).toBeNull();
		expect(byRole("button", /Open presets/).textContent).toContain(
			"Alpenfracht AG",
		);
	});

	test("the cross deletes a preset and the menu goes", async () => {
		const view = await live({
			...closed(),
			view: { ...closed().view, overlay: { id: "presets" } },
		});
		await click(byRole("button", "Delete Alpenfracht AG"));
		expect(view.state().memory.presets.map((preset) => preset.id)).toEqual([
			"preset-nordwind",
		]);
		expect(view.state().view.overlay).toBeNull();
	});

	test("Save as new preset goes from the menu to the dialog and saves a preset", async () => {
		const state = fixture("preset-save");
		const view = await live({
			...state,
			view: { ...state.view, overlay: { id: "presets" } },
			memory: { ...state.memory, presets: [] },
		});
		expect(inPortal("dialog").textContent).toContain(
			"Save inputs you use often, then fill the form in one step.",
		);
		await click(byRole("button", /Save these inputs as a preset/));
		expect(view.state().view.overlay?.id).toBe("presetSave");
		await settle();
		await click(byRole("button", "Save preset"));
		const saved = view.state();
		expect(saved.view.overlay).toBeNull();
		expect(saved.memory.presets).toHaveLength(1);
		const preset = saved.memory.presets[0] as Preset;
		expect({
			name: preset.name,
			openDefault: preset.openDefault,
			sets: Object.keys(preset.sets),
		}).toEqual({
			name: "Nordwind Logistik GmbH",
			openDefault: true,
			sets: ["vendor_name"],
		});
		expect(saved.rail.activePresetId).toBe(preset.id);
		expect(messageKind(saved)).toBe("presetSaved");
	});
});

describe("the head, the filter and the Runs list through the real session", () => {
	test("Inputs | Runs switches the rail; a row puts its run on the stage", async () => {
		const view = await live(fixture("done"));
		await click(byRole("button", /^Runs\s*14$/));
		expect(view.state().rail.tab).toBe("runs");
		const row = view.container.querySelector<HTMLElement>(
			'[data-run-id="run-13"]',
		);
		if (!row) throw new Error("no row for run 13");
		await click(row);
		expect(view.state().view.selectedRunId).toBe("run-13");
		expect(
			view.container
				.querySelector('[aria-current="true"]')
				?.getAttribute("data-run-id"),
		).toBe("run-13");
		await click(byRole("button", "Inputs"));
		expect(view.state().rail.tab).toBe("inputs");
	});

	test("typing in the filter narrows the list and Show all fields brings it back", async () => {
		const view = await live(fixture("large"));
		await typeInto(byRole("textbox", "Filter fields"), "zzzz");
		expect(view.state().rail.filter.query).toBe("zzzz");
		expect(cellNames(view.container)).toEqual([]);
		await click(byRole("button", "Show all fields"));
		expect(view.state().rail.filter).toEqual({ query: "", chip: "all" });
		expect(cellNames(view.container)).toHaveLength(29);
	});

	test("Reset to defaults puts typed values back and the button goes", async () => {
		const view = await live(fixture("done"));
		expect(view.state().rail.values.expected_total).not.toBe("0");
		await click(byRole("button", "Reset to defaults"));
		expect(view.state().rail.values.expected_total).toBe("0");
		await settle();
		expect(queryByRole("button", "Reset to defaults")).toBeNull();
	});

	test("a key press in the rail ends the start message", async () => {
		const state = fixture("series");
		const view = await live(state);
		expect(messageKind(view.state())).toBe("start");
		const field = view.container.querySelector<HTMLElement>(
			'[data-fw-focus^="field:"]',
		);
		if (!field) throw new Error("no field");
		await keyDown(field, "a");
		expect(view.state().view.message).toBeNull();
	});
});
