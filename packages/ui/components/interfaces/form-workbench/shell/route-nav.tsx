"use client";

import { ChevronDownIcon, HomeIcon } from "lucide-react";
import { type ReactElement, type RefObject, useEffect, useRef } from "react";
import { Button } from "../../../ui/button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuTrigger,
} from "../../../ui/dropdown-menu";
import type { IToolBarActions } from "../../interfaces";

export interface RouteNavModel {
	readonly routes: readonly string[];
	readonly labels: Readonly<Record<string, string>>;
	readonly go: (route: string) => void;
	/** The 3+ routes dropdown's label, already translated: these elements render in the host's header. */
	readonly navigateLabel: string;
}

const labelOf = (model: RouteNavModel, route: string) =>
	model.labels[route] ?? route;

const homeIcon = (route: string) =>
	route === "/" ? <HomeIcon className="h-4 w-4" /> : null;

/** The one-route pill and the "Navigate" trigger: 13 px, weight 500 (canvas header), one size under the segments. */
const OUTLINE_PILL = "gap-2 rounded-full px-4 text-[13px]";

function RoutePill({
	route,
	model,
}: Readonly<{ route: string; model: RouteNavModel }>) {
	return (
		<Button
			variant="outline"
			size="sm"
			onClick={() => model.go(route)}
			className={OUTLINE_PILL}
		>
			{homeIcon(route)}
			{labelOf(model, route)}
		</Button>
	);
}

function RouteSegments({ model }: Readonly<{ model: RouteNavModel }>) {
	return (
		<div className="inline-flex items-center rounded-full bg-muted/50 p-0.5">
			{model.routes.map((route) => (
				<button
					key={route}
					type="button"
					onClick={() => model.go(route)}
					className="inline-flex h-8 items-center gap-1.5 rounded-full px-3 font-medium text-muted-foreground text-sm hover:bg-background hover:text-foreground"
				>
					{homeIcon(route)}
					{labelOf(model, route)}
				</button>
			))}
		</div>
	);
}

function RouteMenu({ model }: Readonly<{ model: RouteNavModel }>) {
	const picked = useRef(false);
	const pick = (route: string) => {
		picked.current = true;
		model.go(route);
	};
	return (
		<DropdownMenu>
			<DropdownMenuTrigger asChild>
				<Button variant="outline" size="sm" className={OUTLINE_PILL}>
					{model.navigateLabel}
					<ChevronDownIcon className="h-3.5 w-3.5 opacity-60" />
				</Button>
			</DropdownMenuTrigger>
			<DropdownMenuContent
				align="start"
				className="min-w-40"
				onCloseAutoFocus={(event) => {
					if (picked.current) event.preventDefault();
					picked.current = false;
				}}
			>
				{model.routes.map((route) => (
					<DropdownMenuItem
						key={route}
						onSelect={() => pick(route)}
						className="gap-2"
					>
						{homeIcon(route)}
						{labelOf(model, route)}
					</DropdownMenuItem>
				))}
			</DropdownMenuContent>
		</DropdownMenu>
	);
}

/**
 * The form's route buttons: one route is an outline pill, two a segmented pair, three or more a
 * "Navigate" dropdown; "/" gets the home icon (lifted from the old form).
 */
export function routeNavElements(model: RouteNavModel): ReactElement[] {
	const { routes } = model;
	if (routes.length === 0) return [];
	if (routes.length === 1)
		return [
			<RoutePill
				key={`navigate-${routes[0]}`}
				route={routes[0]}
				model={model}
			/>,
		];
	if (routes.length === 2)
		return [<RouteSegments key="route-nav" model={model} />];
	return [<RouteMenu key="navigate-menu" model={model} />];
}

/**
 * Pushes the route buttons into the host's header from an effect and clears them when the form goes
 * away. A host without a header (no `toolbarRef`) gets `InlineRouteBar` instead.
 */
export function useRouteToolbar(
	toolbarRef: RefObject<IToolBarActions | null> | undefined,
	elements: readonly ReactElement[],
) {
	useEffect(() => {
		const toolbar = toolbarRef?.current;
		if (!toolbar || elements.length === 0) return;
		toolbar.pushNavElements([...elements]);
		return () => toolbarRef?.current?.pushNavElements([]);
	}, [toolbarRef, elements]);
}

/** The route buttons in a 40 px bar at the top of the interface, for hosts that give the form no header. */
export function InlineRouteBar({
	elements,
	label,
}: Readonly<{ elements: readonly ReactElement[]; label: string }>) {
	if (elements.length === 0) return null;
	return (
		<nav
			aria-label={label}
			data-fw-route-bar=""
			className="flex h-10 shrink-0 items-center gap-1 border-hairline border-b px-4"
		>
			{elements}
		</nav>
	);
}
