import type { ReactElement } from "react";
import type { IToolBarActions } from "../../interfaces";
import type {
	DockProps,
	FormSessionState,
	NavigateIntent,
	RailProps,
	StageProps,
} from "../contracts";
import { type FakeActions, fakeActions } from "../testing/fake-actions";
import type { ShellFrameProps, ShellParts } from "./shell-frame";

/*
 * Stand-ins for the three views the shell composes, for the shell's DOM tests: they carry the
 * focus and file hooks the shell reaches for, and nothing else. Import `ShellFrame` itself
 * dynamically after `installWorkbenchDom()`; this file loads no react-dom.
 */

export function FakeRail({ state }: Readonly<RailProps>) {
	return (
		<div data-testid="rail" data-tab={state.rail.tab}>
			<input aria-label="Vendor" data-fw-focus="field:vendor_name" />
			<div data-fw-file-field="invoice_file">
				<button type="button" data-fw-focus="field:invoice_file">
					Invoice
				</button>
				<input type="file" data-fw-file-input="invoice_file" />
			</div>
			<div data-fw-file-field="supporting_documents">
				<button type="button" data-fw-focus="field:supporting_documents">
					Supporting documents
				</button>
				<input type="file" data-fw-file-input="supporting_documents" />
			</div>
			{state.rail.tab === "inputs" ? (
				<input
					type="search"
					aria-label="Filter fields"
					data-fw-focus="filter"
				/>
			) : null}
			<button type="button" data-fw-focus="preset-button">
				Presets
			</button>
		</div>
	);
}

export function FakeDock({ variant }: Readonly<DockProps>) {
	return (
		<div data-testid={`dock-${variant}`}>
			<button type="button" data-fw-focus="run">
				Run
			</button>
			<button type="button" data-fw-focus="stop">
				Stop
			</button>
			<button type="button" data-fw-focus="run-again">
				Run again
			</button>
		</div>
	);
}

export function FakeStage(_props: Readonly<StageProps>) {
	return (
		<div data-testid="stage">
			<button type="button" data-fw-focus="copy">
				Copy answer
			</button>
			<button type="button" data-fw-focus="tab:run-14">
				Run 14
			</button>
		</div>
	);
}

export const FAKE_PARTS: ShellParts = {
	Rail: FakeRail,
	Dock: FakeDock,
	Stage: FakeStage,
};

export interface ShellKit {
	readonly props: ShellFrameProps;
	readonly fake: FakeActions;
	readonly navigated: NavigateIntent[];
	/** What the shell pushed into the host's header, oldest first; `[]` is a clear. */
	readonly pushed: (readonly ReactElement[])[];
}

export interface ShellKitOptions {
	readonly withToolbar?: boolean;
	readonly routeLabels?: Readonly<Record<string, string>>;
}

export function shellKit(
	state: FormSessionState,
	options: ShellKitOptions = {},
): ShellKit {
	const fake = fakeActions();
	const navigated: NavigateIntent[] = [];
	const pushed: (readonly ReactElement[])[] = [];
	const toolbarRef = options.withToolbar
		? {
				current: {
					pushNavElements: (elements) => pushed.push(elements),
					pushToolbarElements: () => {},
				} satisfies IToolBarActions,
			}
		: undefined;
	const props: ShellFrameProps = {
		state,
		actions: fake.actions,
		appId: state.form.appId,
		toolbarRef,
		navigate: (intent) => navigated.push(intent),
		parts: FAKE_PARTS,
		routeLabels: options.routeLabels ?? {
			"/": "Home",
			"/review": "Review queue",
			"/chat": "Support chat",
		},
	};
	return { props, fake, navigated, pushed };
}
