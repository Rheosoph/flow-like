import type { GuideTourId, IGuideTour } from "./guide-model";

export const GUIDE_BOTS_ROUTE = "/bots";
export const GUIDE_LIBRARY_ROUTE = "/library";
export const GUIDE_EXPLORE_ROUTE = "/store/explore";

/** The onboarding page with the introduction and the welcome question, in every app. */
export const GUIDE_WELCOME_ROUTE = "/onboarding/welcome";

export type GuideWelcomeStart = "story" | "question";

export function guideWelcomeHref(start: GuideWelcomeStart = "story"): string {
	return start === "question"
		? `${GUIDE_WELCOME_ROUTE}?start=question`
		: GUIDE_WELCOME_ROUTE;
}
const FLOWS_ROUTE = "/library/config/flows";
const BOARD_ROUTE = "/flow";

/** Bots and apps for people who want things done, not built. */
const USE_TOUR: IGuideTour = {
	id: "use",
	flags: { "bots:approval-shown": "approval" },
	steps: [
		{
			id: "bots",
			target: "nav-bots",
			tag: "click",
			act: "click",
			completeOn: [{ route: GUIDE_BOTS_ROUTE }],
		},
		{
			id: "crew",
			target: "crew-bot",
			tag: "click",
			act: "click",
			route: GUIDE_BOTS_ROUTE,
			passable: true,
			completeOn: [{ event: "bots:chat-opened" }],
		},
		{
			id: "send",
			target: "chat-composer",
			tag: "type",
			route: GUIDE_BOTS_ROUTE,
			completeOn: [{ event: "bots:message-sent" }],
		},
		{
			id: "watch",
			act: "next",
			completeOn: [
				{ event: "bots:reply-finished" },
				{ event: "bots:approval-shown" },
			],
		},
		{
			id: "approve",
			target: "approval",
			requires: "approval",
			completeOn: [{ event: "bots:approval-answered" }],
		},
		{
			id: "details",
			target: "bot-pill",
			tag: "click",
			act: "click",
			route: GUIDE_BOTS_ROUTE,
			completeOn: [{ event: "bots:details-opened" }],
		},
		{
			id: "explore",
			target: "nav-explore",
			tag: "click",
			act: "click",
			completeOn: [{ route: GUIDE_EXPLORE_ROUTE }],
		},
		{
			id: "app",
			target: "explore-app",
			tag: "open",
			act: "click",
			route: GUIDE_EXPLORE_ROUTE,
			completeOn: [{ event: "store:app-opened" }],
		},
	],
};

/** From an empty library to a flow that ran, through the real editor. */
const BUILD_TOUR: IGuideTour = {
	id: "build",
	steps: [
		{
			id: "library",
			target: "nav-library",
			tag: "click",
			act: "click",
			completeOn: [{ route: GUIDE_LIBRARY_ROUTE }],
		},
		{
			id: "newApp",
			target: "library-new-app",
			tag: "click",
			act: "click",
			route: GUIDE_LIBRARY_ROUTE,
			completeOn: [{ event: "library:create-opened" }],
		},
		{
			id: "createApp",
			target: "create-app-submit",
			tag: "click",
			completeOn: [{ event: "library:app-created" }],
		},
		{
			id: "newFlow",
			target: "app-new-flow",
			tag: "click",
			act: "click",
			completeOn: [{ route: FLOWS_ROUTE }, { route: BOARD_ROUTE }],
		},
		{
			id: "createFlow",
			target: "flows-create",
			tag: "click",
			act: "click",
			completeOn: [{ event: "flows:create-opened" }, { route: BOARD_ROUTE }],
		},
		{
			id: "confirmFlow",
			target: "flows-create-submit",
			tag: "click",
			completeOn: [{ event: "flows:board-created" }, { route: BOARD_ROUTE }],
		},
		{
			id: "openFlow",
			target: "flows-open-board",
			tag: "open",
			act: "click",
			completeOn: [{ route: BOARD_ROUTE }],
		},
		{
			id: "addEvent",
			target: "board-canvas",
			tag: "rightClick",
			route: BOARD_ROUTE,
			completeOn: [{ event: "board:node-added" }],
		},
		{
			id: "connect",
			target: "board-canvas",
			tag: "drag",
			route: BOARD_ROUTE,
			completeOn: [{ event: "board:pins-connected" }],
		},
		{
			id: "run",
			target: "event-run",
			tag: "click",
			act: "click",
			route: BOARD_ROUTE,
			completeOn: [{ event: "board:run-started" }],
		},
		{
			id: "wait",
			route: BOARD_ROUTE,
			completeOn: [{ event: "board:run-finished" }],
		},
		{
			id: "logs",
			target: "board-panel-traces",
			act: "next",
			route: BOARD_ROUTE,
			completeOn: [],
		},
	],
};

export const GUIDE_TOURS: Readonly<Record<GuideTourId, IGuideTour>> = {
	use: USE_TOUR,
	build: BUILD_TOUR,
};

/** Where "Take me there" goes for steps that start on a fixed page. */
export function guideStepHref(route: string | undefined): string | undefined {
	if (
		route === GUIDE_BOTS_ROUTE ||
		route === GUIDE_LIBRARY_ROUTE ||
		route === GUIDE_EXPLORE_ROUTE
	) {
		return route;
	}
	return undefined;
}

/** `data-guide` ids for sidebar destinations, so each app's sidebar marks the same targets. */
export function guideTargetForNav(url: string): string | undefined {
	if (url === GUIDE_BOTS_ROUTE) return "nav-bots";
	if (url === GUIDE_LIBRARY_ROUTE) return "nav-library";
	if (url === GUIDE_EXPLORE_ROUTE) return "nav-explore";
	return undefined;
}
