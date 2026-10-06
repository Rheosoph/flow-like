"use client";

import type { WorkbenchShellProps } from "../contracts";
import { Dock } from "../dock/dock";
import { Rail } from "../rail/rail";
import { Stage } from "../stage/stage";
import { useRouteLabels } from "./route-labels";
import { ShellFrame, type ShellParts } from "./shell-frame";

const PARTS: ShellParts = { Rail, Dock, Stage };

/**
 * Root, layout, keyboard, focus, route buttons and modals of the form workbench: composes the rail,
 * the dock and the stage and owns everything that spans them.
 */
export function WorkbenchShell(props: Readonly<WorkbenchShellProps>) {
	const routeLabels = useRouteLabels(props.appId, props.state.form.routes);
	return <ShellFrame {...props} parts={PARTS} routeLabels={routeLabels} />;
}
