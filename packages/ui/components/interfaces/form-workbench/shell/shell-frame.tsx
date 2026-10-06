"use client";

import { useTranslation } from "@flow-like/locales";
import { type ReactElement, useMemo, useRef } from "react";
import { cx } from "../../../settings/devices/primitives/tone";
import type {
	FormSessionActions,
	FormSessionState,
	NavigateIntent,
	WorkbenchLayout,
	WorkbenchShellProps,
} from "../contracts";
import {
	QUIET_FOCUS_CSS,
	SELECTION_CSS,
	useAutofocus,
	useFocusExecutor,
} from "./focus";
import { useFocusRescue } from "./focus-rescue";
import { ModalLayerContext, useModalLayer } from "./interface-modal";
import { LeaveDialog } from "./leave-dialog";
import { LiveRegion } from "./live-region";
import { InlineRouteBar, routeNavElements, useRouteToolbar } from "./route-nav";
import {
	type BodyProps,
	type ShellParts,
	SingleBody,
	SplitBody,
} from "./shell-bodies";
import {
	useRouteNav,
	useShellKeys,
	useShellLayout,
	useViewProps,
} from "./shell-hooks";
import { ShortcutsSheet } from "./shortcuts-sheet";
import type { BoxSize } from "./use-box-layout";

export { HERO_WAIT_MS, type ShellParts } from "./shell-bodies";

export interface ShellFrameProps extends WorkbenchShellProps {
	readonly parts: ShellParts;
	/** Route → label, from `useRouteLabels`. */
	readonly routeLabels: Readonly<Record<string, string>>;
	/** Gives the first frame a layout before the observer reports (tests). */
	readonly initialSize?: BoxSize;
}

/** Hosts that bring their own bordered header (hosted, service, tiles) get no hairline of ours. */
function hasTopHairline(state: FormSessionState, layout: WorkbenchLayout) {
	const { host } = state.form;
	return (
		host.kind === "app" &&
		host.presentation === "page" &&
		(layout.split || !layout.touch)
	);
}

interface ContentProps extends BodyProps {
	readonly layout: WorkbenchLayout;
	readonly actions: FormSessionActions;
	readonly navigate: (intent: NavigateIntent) => void;
	readonly elements: readonly ReactElement[];
	readonly hasToolbar: boolean;
}

function ShellContent({
	parts,
	view,
	layout,
	actions,
	navigate,
	elements,
	hasToolbar,
}: Readonly<ContentProps>) {
	const { t } = useTranslation("interfaces");
	const routesLabel = t("workbench.shell.routesLabel", "Screens of this app");
	const { state } = view;
	return (
		<>
			{hasToolbar ? null : (
				<InlineRouteBar elements={elements} label={routesLabel} />
			)}
			<div
				className={cx(
					"flex min-h-0 flex-1 flex-col",
					hasTopHairline(state, layout) && "border-hairline border-t",
				)}
			>
				{layout.split ? (
					<SplitBody parts={parts} view={view} />
				) : (
					<SingleBody parts={parts} view={view} />
				)}
			</div>
			<LeaveDialog state={state} actions={actions} navigate={navigate} />
			<ShortcutsSheet state={state} actions={actions} />
		</>
	);
}

/**
 * The interface root and everything interface-wide: layout, composition of rail, dock and stage, route
 * buttons, the leave dialog, the shortcuts sheet, chords, focus and the live region.
 */
export function ShellFrame({
	state,
	actions,
	toolbarRef,
	navigate,
	parts,
	routeLabels,
	initialSize,
}: Readonly<ShellFrameProps>) {
	const { t } = useTranslation("interfaces");
	const navigateLabel = t("workbench.shell.navigate", "Navigate");
	const rootRef = useRef<HTMLDivElement>(null);
	const layout = useShellLayout(rootRef, state, actions, initialSize);
	const routes = useRouteNav({ state, actions, navigate, labels: routeLabels });
	const elements = useMemo(
		() =>
			routeNavElements({
				routes: state.form.routes,
				labels: routes.labels,
				go: routes.go,
				navigateLabel,
			}),
		[state.form.routes, routes, navigateLabel],
	);
	useRouteToolbar(toolbarRef, elements);
	const modalLayer = useModalLayer(rootRef);
	useFocusExecutor(
		rootRef,
		state.view.focus,
		actions.focusHandled,
		layout?.split ?? false,
	);
	useAutofocus(rootRef, state, layout);
	const rescue = useFocusRescue(rootRef, state);
	const { onKeyDown, showRings } = useShellKeys(rootRef, state, actions);
	const view = useViewProps(state, actions, layout, routes);
	const arrangement = layout?.split ? "split" : "single";

	return (
		<ModalLayerContext.Provider value={modalLayer}>
			<div
				ref={rootRef}
				tabIndex={-1}
				data-fw-root=""
				data-fw-host={state.form.host.kind}
				data-fw-layout={layout ? arrangement : "pending"}
				onKeyDown={onKeyDown}
				onKeyDownCapture={showRings}
				onPointerDownCapture={showRings}
				onFocus={rescue.onFocus}
				onBlur={rescue.onBlur}
				className="@container/fw relative flex h-full min-h-0 flex-col overflow-hidden bg-background text-foreground antialiased outline-none"
			>
				<style>{QUIET_FOCUS_CSS + SELECTION_CSS}</style>
				{view && layout ? (
					<ShellContent
						parts={parts}
						view={view}
						layout={layout}
						actions={actions}
						navigate={navigate}
						elements={elements}
						hasToolbar={toolbarRef !== undefined}
					/>
				) : null}
				<LiveRegion announcement={state.announcement} />
			</div>
		</ModalLayerContext.Provider>
	);
}
