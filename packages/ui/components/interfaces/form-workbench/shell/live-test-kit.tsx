import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, useMemo, useState } from "react";
import type {
	FormSessionState,
	PickedFile,
	SessionClock,
	SessionCommand,
	SessionInput,
	WorkbenchLayout,
} from "../contracts";
import { sessionActions } from "../session/controller";
import { reduceSession } from "../session/reduce";
import { type MountedWorkbench, mountWorkbench } from "../testing/dom";
import { WorkbenchShell } from "./workbench-shell";

/*
 * The whole shell (real rail, dock and stage) on the real session reducer, for DOM tests of focus paths.
 * Commands come from the views' actions; runtime inputs (`runSettled`, …) come from the test. Effects are
 * dropped: no engine, no uploads, no storage. Import after `installWorkbenchDom()`.
 */

type Message =
	| { readonly kind: "command"; readonly command: SessionCommand }
	| { readonly kind: "input"; readonly input: SessionInput };

interface Driver {
	send(message: Message): void;
	state: FormSessionState;
}

const keepFiles = (files: readonly File[]): readonly PickedFile[] =>
	files.map((file, index) => ({
		slotId: `picked-${index}-${file.name}`,
		name: file.name,
		size: file.size,
		type: file.type,
	}));

function Live({
	initial,
	clock,
	driver,
}: Readonly<{
	initial: FormSessionState;
	clock: SessionClock;
	driver: Driver;
}>) {
	const [state, setState] = useState(initial);
	driver.state = state;
	const actions = useMemo(() => {
		driver.send = (message) =>
			setState((current) => reduceSession(current, message, clock).state);
		return sessionActions(
			(command) => driver.send({ kind: "command", command }),
			keepFiles,
		);
	}, [clock, driver]);
	const client = useMemo(
		() => new QueryClient({ defaultOptions: { queries: { retry: false } } }),
		[],
	);
	return (
		<QueryClientProvider client={client}>
			<WorkbenchShell
				state={state}
				actions={actions}
				appId={state.form.appId}
				toolbarRef={undefined}
				navigate={() => {}}
			/>
		</QueryClientProvider>
	);
}

export interface LiveShell extends MountedWorkbench {
	readonly root: HTMLElement;
	state(): FormSessionState;
	/** A runtime input, as the engine would report it. */
	report(input: SessionInput): Promise<void>;
}

export async function mountLiveShell(
	initial: FormSessionState,
	layout: WorkbenchLayout,
	clock: SessionClock,
): Promise<LiveShell> {
	const driver: Driver = { send: () => {}, state: initial };
	const view = await mountWorkbench(
		<Live initial={{ ...initial, layout }} clock={clock} driver={driver} />,
		{ layout },
	);
	const root = view.container.querySelector<HTMLElement>("[data-fw-root]");
	if (!root) throw new Error("mountLiveShell: no interface root");
	return {
		...view,
		root,
		state: () => driver.state,
		async report(input) {
			await act(async () => driver.send({ kind: "input", input }));
		},
	};
}
