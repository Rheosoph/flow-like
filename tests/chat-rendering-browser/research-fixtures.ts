import type {
	IPlanStep,
	IToolActivityDetail,
} from "../../packages/ui/components/interfaces/chat-default/chat-db";

type Source = NonNullable<IToolActivityDetail["sources"]>[number];

const source = (url: string, title: string): Source => {
	const parsed = new URL(url);
	return { url, title, hostname: parsed.hostname, path: parsed.pathname };
};

export const researchSources = [
	source(
		"https://www.porsche.com/international/models/panamera/",
		"Panamera models and equipment",
	),
	source(
		"https://www.bmw.com/en/models/5-series.html",
		"BMW 5 Series overview",
	),
	source(
		"https://www.bmw.com/en/models/7-series.html",
		"BMW 7 Series overview",
	),
	source(
		"https://www.autocar.co.uk/car-review/porsche/panamera",
		"Porsche Panamera review",
	),
	source(
		"https://www.caranddriver.com/porsche/panamera",
		"Panamera road test and specifications",
	),
	source(
		"https://www.topgear.com/car-reviews/porsche/panamera",
		"Porsche Panamera: comfort and touring",
	),
	source(
		"https://www.adac.de/rund-ums-fahrzeug/autokatalog/marken-modelle/porsche/panamera/",
		"Porsche Panamera im Autotest",
	),
	source(
		"https://www.autocar.co.uk/car-review/bmw/7-series",
		"BMW 7 Series long-distance road test",
	),
];

export const researchQueries = [
	"Porsche Panamera vs BMW 5 Series 7 Series ride comfort",
	"Panamera air suspension long distance seats road test",
	"Panamera Komfort Langstrecke Vergleich BMW 7er",
];

/** Fixed examples exercise the presentation; no network research runs in this preview. */
export function researchSteps(settled = false): IPlanStep[] {
	const progress = settled ? "done" : "progress";
	const started = 1_700_000_000_000;
	return [
		{
			id: "apps",
			toolName: "list_apps",
			title: "Using list_apps",
			status: "done",
			startTime: started,
			endTime: started + 603,
			detail: {
				kind: "tool_activity",
				category: "apps",
				label: "Checking available apps",
				outcome: "3 apps found",
			},
		},
		{
			id: "research",
			toolName: "research_agent",
			title: "Using research_agent",
			status: progress,
			startTime: started + 603,
			...(settled ? { endTime: started + 96_000 } : {}),
			detail: {
				kind: "tool_activity",
				category: "search",
				label: "Researching sources",
			},
		},
		...researchQueries.map(
			(query, index): IPlanStep => ({
				id: `search-${index}`,
				toolName: "internet_search",
				title: "↳ Using internet_search",
				status: index === 2 ? progress : "done",
				detail: {
					kind: "tool_activity",
					category: "search",
					label: "Searching the web",
					query,
					queries: [query],
					sources:
						index === 0
							? researchSources.slice(0, 6)
							: index === 1
								? researchSources.slice(5)
								: [],
				},
			}),
		),
		...[0, 1, 3, 4, 5, 6].map(
			(sourceIndex, index): IPlanStep => ({
				id: `read-${index}`,
				toolName: "open_url",
				title: "↳ Using open_url",
				status: index === 4 ? "failed" : index === 5 ? progress : "done",
				detail: {
					kind: "tool_activity",
					category: "web",
					label: "Reading a webpage",
					sources: [researchSources[sourceIndex]],
					...(index === 4 ? { outcome: "Page could not be opened" } : {}),
				},
			}),
		),
	];
}
