/**
 * The routes of the web app that render something without a session. Everything
 * else needs a sign-in, so these are the only ones worth pointing a crawler or
 * an agent at — app/sitemap.ts and app/llms.txt/route.ts both read this list.
 */
export interface PublicRoute {
	readonly path: string;
	readonly title: string;
	readonly description: string;
}

export const PUBLIC_ROUTES: readonly PublicRoute[] = [
	{
		path: "/",
		title: "Home",
		description:
			"Entry point of the hosted app: ask FlowPilot for something, or browse your apps",
	},
	{
		path: "/store/explore/apps",
		title: "Explore apps",
		description: "Browse apps and suites published to the Flow-Like store",
	},
	{
		path: "/store",
		title: "App detail",
		description:
			"Details, reviews and install for one published app — open with ?id=<app id>",
	},
	{
		path: "/store/packages",
		title: "Package detail",
		description:
			"Details and install for one WASM node package — open with ?id=<package id>",
	},
];
