"use client";
import { ChevronDownIcon, HomeIcon, InfoIcon } from "lucide-react";
import Link from "next/link";
import {
	type ReactNode,
	memo,
	useEffect,
	useImperativeHandle,
	useMemo,
	useState,
} from "react";
import { normalizeRoutePath } from "../../lib/route-path";
import type { IRouteMapping } from "../../state/backend-state/route-state";
import {
	Button,
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuTrigger,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	useMobileHeader,
} from "../ui";
import type { IToolBarActions } from "./interfaces";

interface HeaderProps {
	routes?: IRouteMapping[];
	currentRoutePath?: string;
	onNavigateRoute?: (path: string) => void;
	usableEvents: Set<string>;
	currentEvent: any;
	sortedEvents: any[];
	metadata: any;
	appId: string;
	switchEvent: (eventId: string) => void;
}

function getRouteLabel(route: IRouteMapping): string {
	if (route.path === "/") return "Home";
	return route.path.replace(/^\//, "").replace(/-/g, " ").replace(/\//g, " / ");
}

function getRouteIcon(route: IRouteMapping): ReactNode {
	if (route.path === "/") return <HomeIcon className="h-4 w-4" />;
	return null;
}

const HeaderInner = ({
	ref,
	routes,
	currentRoutePath,
	onNavigateRoute,
	usableEvents,
	currentEvent,
	sortedEvents,
	metadata,
	appId,
	switchEvent,
}: HeaderProps & {
	ref: React.RefObject<IToolBarActions>;
}) => {
	const [toolbarElements, setToolbarElements] = useState<ReactNode[]>([]);
	const [navElements, setNavElements] = useState<ReactNode[]>([]);

	useImperativeHandle(ref, () => ({
		pushToolbarElements: (elements: ReactNode[]) => {
			setToolbarElements(elements);
		},
		pushNavElements: (elements: ReactNode[]) => {
			setNavElements(elements);
		},
	}));

	const { update } = useMobileHeader();

	const sortedRoutes = useMemo(() => {
		const list = (routes ?? []).slice();
		list.sort((a, b) => a.path.localeCompare(b.path));
		return list;
	}, [routes]);

	const hasRoutes = sortedRoutes.length > 0;
	const normalizedCurrentRoutePath = normalizeRoutePath(currentRoutePath);
	const activeRoute = useMemo(() => {
		if (!hasRoutes) return null;
		return (
			sortedRoutes.find(
				(r) => normalizeRoutePath(r.path) === normalizedCurrentRoutePath,
			) ??
			sortedRoutes.find((r) => r.path === "/") ??
			sortedRoutes[0] ??
			null
		);
	}, [hasRoutes, sortedRoutes, normalizedCurrentRoutePath]);

	const routeNav = useMemo(() => {
		if (!hasRoutes || !onNavigateRoute) return null;

		// Single route: clean pill button
		if (sortedRoutes.length === 1) {
			const r = sortedRoutes[0];
			const icon = getRouteIcon(r);
			return (
				<Button
					variant="outline"
					size="sm"
					onClick={() => onNavigateRoute(r.path)}
					className="rounded-full px-4 gap-2 font-medium"
				>
					{icon}
					{getRouteLabel(r)}
				</Button>
			);
		}

		// Two routes: segmented control style
		if (sortedRoutes.length === 2) {
			return (
				<div className="inline-flex items-center rounded-full bg-muted/50 p-0.5">
					{sortedRoutes.map((r) => {
						const isActive =
							normalizeRoutePath(r.path) === normalizedCurrentRoutePath;
						const icon = getRouteIcon(r);
						return (
							<button
								key={r.path}
								type="button"
								onClick={() => onNavigateRoute(r.path)}
								className={`
									inline-flex items-center gap-1.5 px-3 py-1.5 text-sm font-medium rounded-full transition-all
									${
										isActive
											? "bg-background text-foreground shadow-sm"
											: "text-muted-foreground hover:text-foreground"
									}
								`}
							>
								{icon}
								{getRouteLabel(r)}
							</button>
						);
					})}
				</div>
			);
		}

		// 3+ routes: dropdown with current selection shown
		return (
			<DropdownMenu>
				<DropdownMenuTrigger asChild>
					<Button
						variant="outline"
						size="sm"
						className="rounded-full px-4 gap-2 font-medium"
					>
						{activeRoute && getRouteIcon(activeRoute)}
						{activeRoute ? getRouteLabel(activeRoute) : "Navigate"}
						<ChevronDownIcon className="h-3.5 w-3.5 opacity-60" />
					</Button>
				</DropdownMenuTrigger>
				<DropdownMenuContent align="start" className="min-w-[160px]">
					{sortedRoutes.map((r) => {
						const isActive =
							normalizeRoutePath(r.path) === normalizedCurrentRoutePath;
						const icon = getRouteIcon(r);
						return (
							<DropdownMenuItem
								key={r.path}
								onSelect={() => onNavigateRoute(r.path)}
								className={`gap-2 ${isActive ? "bg-muted" : ""}`}
							>
								{icon}
								{getRouteLabel(r)}
							</DropdownMenuItem>
						);
					})}
				</DropdownMenuContent>
			</DropdownMenu>
		);
	}, [
		hasRoutes,
		onNavigateRoute,
		sortedRoutes,
		normalizedCurrentRoutePath,
		activeRoute,
	]);

	useEffect(() => {
		const right: ReactNode[] = [];
		if (appId && currentEvent) {
			right.push(
				<Link
					key="info"
					href={`/store?id=${appId}&eventId=${currentEvent?.id}`}
				>
					<Button
						variant="ghost"
						size="icon"
						onClick={() => {
							console.log("Open chat history");
						}}
						className="h-8 w-8 p-0"
					>
						<InfoIcon className="h-4 w-4" />
					</Button>
				</Link>,
			);
		}

		// Route navigation is handled by interface toolbar elements (chat, page, etc.)
		// Only show event selector when there are no routes configured
		if (!hasRoutes && appId && currentEvent && sortedEvents.length > 1) {
			right.push(
				<Select
					key="events"
					value={currentEvent.id}
					onValueChange={switchEvent}
				>
					<SelectTrigger className="max-w-[200px] flex flex-row justify-between h-8 bg-muted/20 border-transparent">
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						{sortedEvents
							.filter((event) => usableEvents.has(event.event_type))
							.map((event) => (
								<SelectItem key={event.id} value={event.id}>
									{event.name ?? event.event_type}
								</SelectItem>
							))}
					</SelectContent>
				</Select>,
			);
		}

		const rightWithNav = [...navElements, ...right];

		update({
			right: rightWithNav.length > 0 ? rightWithNav : undefined,
			left: toolbarElements.length > 0 ? toolbarElements : undefined,
		});
	}, [
		update,
		appId,
		currentEvent,
		sortedEvents,
		switchEvent,
		usableEvents,
		toolbarElements,
		navElements,
		hasRoutes,
		routeNav,
	]);

	if (!appId) return null;
	if (!currentEvent && !hasRoutes) return null;

	const header = (
		<div className="hidden h-0 items-center justify-between p-4 bg-background backdrop-blur-xs md:flex md:h-fit">
			<div className="flex items-center gap-1">
				{!hasRoutes && currentEvent && sortedEvents.length > 1 && (
					<Select value={currentEvent.id} onValueChange={switchEvent}>
						<SelectTrigger className="max-w-[200px] flex flex-row justify-between h-8 bg-muted/20 border-transparent">
							<SelectValue />
						</SelectTrigger>
						<SelectContent>
							{sortedEvents
								.filter((event) => usableEvents.has(event.event_type))
								.map((event) => (
									<SelectItem key={event.id} value={event.id}>
										{event.name ?? event.event_type}
									</SelectItem>
								))}
						</SelectContent>
					</Select>
				)}
				<div className="flex items-center gap-1">
					{toolbarElements.map((element, index) => (
						<div key={index}>{element}</div>
					))}
				</div>
				{navElements.length > 0 && (
					<div className="flex items-center gap-1">
						{navElements.map((element, index) => (
							<div key={index}>{element}</div>
						))}
					</div>
				)}
			</div>
			<div className="flex items-center gap-2">
				<h1 className="text-lg font-semibold">{metadata?.name}</h1>
				{currentEvent && (
					<Link href={`/store?id=${appId}&eventId=${currentEvent.id}`}>
						<Button
							variant="ghost"
							size="icon"
							onClick={() => {
								console.log("Open chat history");
							}}
							className="h-8 w-8 p-0"
						>
							<InfoIcon className="h-4 w-4" />
						</Button>
					</Link>
				)}
			</div>
		</div>
	);

	return header;
};

export const Header = memo(HeaderInner);
Header.displayName = "Header";
