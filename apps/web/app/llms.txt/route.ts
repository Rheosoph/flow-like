import { PUBLIC_ROUTES } from "../../lib/public-routes";

export const dynamic = "force-static";

const siteUrl = process.env.NEXT_PUBLIC_SITE_URL || "https://app.flow-like.com";

const SUMMARY =
	"The hosted Flow-Like client: build, run and operate typed visual workflows, data and apps in the browser. Same client as the desktop app, against a hosted or self-hosted backend.";

const INTRO = [
	"Almost everything here is behind a sign-in — flows and the node editor, Data Studio, pages and widgets, chat, packages, team and app settings, telemetry and the learning area. Those routes render nothing useful without a session, so they are not listed.",
	"Read about the product on https://flow-like.com and how it works on https://docs.flow-like.com. Both publish an llms.txt of their own.",
];

const OPTIONAL = [
	{
		title: "Flow-Like docs",
		url: "https://docs.flow-like.com/llms.txt",
		description: "guides, self-hosting, SDKs and the full node catalog",
	},
	{
		title: "Flow-Like website",
		url: "https://flow-like.com/llms.txt",
		description: "product pages, pricing, comparisons and the engineering blog",
	},
	{
		title: "Sitemap",
		url: `${siteUrl}/sitemap.xml`,
		description: "the same public routes, as XML",
	},
	{
		title: "GitHub",
		url: "https://github.com/Rheosoph/flow-like",
		description: "source, issues and release notes",
	},
];

function entry({
	title,
	url,
	description,
}: {
	title: string;
	url: string;
	description: string;
}) {
	return `- [${title}](${url}): ${description}`;
}

function body() {
	return [
		"# Flow-Like Web App",
		"",
		`> ${SUMMARY}`,
		"",
		...INTRO.flatMap((paragraph) => [paragraph, ""]),
		"## Public pages",
		"",
		...PUBLIC_ROUTES.map(({ path, title, description }) =>
			entry({ title, url: `${siteUrl}${path}`, description }),
		),
		"",
		"## Optional",
		"",
		...OPTIONAL.map(entry),
		"",
	].join("\n");
}

export function GET() {
	return new Response(body(), {
		headers: {
			"content-type": "text/plain; charset=utf-8",
			"cache-control": "public, max-age=3600",
		},
	});
}
