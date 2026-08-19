"use client";

import { TextEditor } from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { useState } from "react";

const EXAMPLE_FULL_MARKDOWN = `# Markdown Debug Preview

This is a **comprehensive** markdown test with _various_ features.

## Text Formatting

Regular text, **bold**, *italic*, ~~strikethrough~~, \`inline code\`, and ***bold italic***.

## Lists

- Bullet item 1
- Bullet item 2
  - Nested item
- Bullet item 3

1. Numbered item
2. Another item
3. Third item

## Code Blocks

\`\`\`typescript
const greeting = "Hello, World!";
console.log(greeting);
\`\`\`

## Charts

### Nivo Bar Chart

\`\`\`nivo
type: bar
title: Monthly Sales
---
month,sales,expenses,profit
Jan,4200,3100,1100
Feb,5100,3400,1700
Mar,4800,3200,1600
Apr,6200,4100,2100
\`\`\`

### Plotly Line Chart

\`\`\`plotly
type: line
title: Temperature Trends
---
month,New York,London
Jan,-2,5
Feb,0,6
Mar,5,9
Apr,12,12
May,18,15
\`\`\`

## Tables

| Feature | Status | Notes |
|---------|--------|-------|
| Tables | ✅ | Working |
| Charts | ✅ | Nivo & Plotly |
| Code | ✅ | Syntax highlighting |

## Blockquotes

> This is a blockquote.
> It can span multiple lines.

## Links and Images

[Link to Google](https://google.com)

---

*End of markdown preview*
`;

const EXAMPLE_NIVO_CSV = `\`\`\`nivo
type: bar
title: Monthly Sales
xLabel: Month
yLabel: Amount
showLegend: true
legendPosition: bottom
stacked: false
animate: false
---
month,sales,expenses,profit
Jan,4200,3100,1100
Feb,5100,3400,1700
Mar,4800,3200,1600
Apr,6200,4100,2100
May,5800,3900,1900
Jun,7100,4500,2600
\`\`\``;

const EXAMPLE_NIVO_LINE = `\`\`\`nivo
type: line
title: Stock Performance
colors: paired
---
date,AAPL,GOOGL,MSFT
Jan,150,140,310
Feb,155,145,320
Mar,148,150,315
Apr,160,155,330
May,165,148,340
Jun,170,160,355
\`\`\``;

const EXAMPLE_NIVO_PIE = `\`\`\`nivo
type: pie
title: Market Share
---
company,share
Apple,28
Samsung,21
Xiaomi,14
Oppo,10
Others,27
\`\`\``;

const EXAMPLE_NIVO_RADAR = `\`\`\`nivo
type: radar
title: Team Skills Assessment
---
skill,Frontend,Backend,DevOps
JavaScript,95,60,40
Python,30,90,70
React,90,20,15
Docker,25,70,95
SQL,40,85,50
AWS,35,65,90
\`\`\``;

const EXAMPLE_NIVO_HEATMAP = `\`\`\`nivo
type: heatmap
title: Weekly Activity
---
day,9am,12pm,3pm,6pm,9pm
Mon,45,78,62,38,15
Tue,58,95,71,42,22
Wed,52,88,68,45,18
Thu,65,92,75,55,28
Fri,48,85,60,72,45
\`\`\``;

const EXAMPLE_NIVO_JSON = `\`\`\`nivo
{
  "chartType": "bar",
  "data": [
    { "country": "USA", "burgers": 131, "fries": 85, "sandwiches": 72 },
    { "country": "Germany", "burgers": 95, "fries": 108, "sandwiches": 86 },
    { "country": "France", "burgers": 72, "fries": 102, "sandwiches": 95 },
    { "country": "UK", "burgers": 88, "fries": 95, "sandwiches": 110 }
  ],
  "indexBy": "country",
  "keys": ["burgers", "fries", "sandwiches"]
}
\`\`\``;

const EXAMPLE_PLOTLY_CSV = `\`\`\`plotly
type: bar
title: Quarterly Revenue
xLabel: Quarter
yLabel: Revenue ($M)
---
quarter,2023,2024
Q1,120,145
Q2,135,160
Q3,150,175
Q4,180,210
\`\`\``;

const EXAMPLE_PLOTLY_LINE = `\`\`\`plotly
type: line
title: Temperature Trends
xLabel: Month
yLabel: Temperature (°C)
---
month,New York,London,Tokyo
Jan,-2,5,6
Feb,0,6,7
Mar,5,9,11
Apr,12,12,16
May,18,15,20
Jun,23,18,24
\`\`\``;

const EXAMPLE_PLOTLY_SCATTER = `\`\`\`plotly
type: scatter
title: Height vs Weight
xLabel: Height (cm)
yLabel: Weight (kg)
---
height,weight
160,55
165,62
170,68
175,75
180,82
185,88
172,70
168,65
178,78
\`\`\``;

const EXAMPLE_PLOTLY_PIE = `\`\`\`plotly
type: pie
title: Browser Market Share
---
browser,share
Chrome,65
Safari,18
Firefox,8
Edge,5
Others,4
\`\`\``;

const EXAMPLE_PLOTLY_JSON = `\`\`\`plotly
{
  "data": [
    {
      "x": ["Jan", "Feb", "Mar", "Apr", "May", "Jun"],
      "y": [20, 14, 25, 16, 18, 22],
      "type": "scatter",
      "mode": "lines+markers",
      "name": "Series A",
      "marker": { "color": "#8884d8" }
    },
    {
      "x": ["Jan", "Feb", "Mar", "Apr", "May", "Jun"],
      "y": [12, 18, 15, 22, 14, 20],
      "type": "scatter",
      "mode": "lines+markers",
      "name": "Series B",
      "marker": { "color": "#82ca9d" }
    }
  ],
  "layout": {
    "title": "Custom Plotly Chart",
    "xaxis": { "title": "Month" },
    "yaxis": { "title": "Value" }
  }
}
\`\`\``;

const CHART_EXAMPLES = [
	{ title: "Nivo Bar (CSV)", content: EXAMPLE_NIVO_CSV },
	{ title: "Nivo Line (CSV)", content: EXAMPLE_NIVO_LINE },
	{ title: "Nivo Pie (CSV)", content: EXAMPLE_NIVO_PIE },
	{ title: "Nivo Radar (CSV)", content: EXAMPLE_NIVO_RADAR },
	{ title: "Nivo Heatmap (CSV)", content: EXAMPLE_NIVO_HEATMAP },
	{ title: "Nivo Bar (JSON)", content: EXAMPLE_NIVO_JSON },
	{ title: "Plotly Bar (CSV)", content: EXAMPLE_PLOTLY_CSV },
	{ title: "Plotly Line (CSV)", content: EXAMPLE_PLOTLY_LINE },
	{ title: "Plotly Scatter (CSV)", content: EXAMPLE_PLOTLY_SCATTER },
	{ title: "Plotly Pie (CSV)", content: EXAMPLE_PLOTLY_PIE },
	{ title: "Plotly Custom (JSON)", content: EXAMPLE_PLOTLY_JSON },
];

// ========== CALLOUT / ADMONITION EXAMPLES ==========

const EXAMPLE_CALLOUT_INFO = `:::info
Workflow \`daily-report-gen\` completed in 3.2s. 47/47 nodes executed.
:::`;

const EXAMPLE_CALLOUT_SUCCESS = `:::success
All 12 TISAX validation checks passed. Audit trail archived.
:::`;

const EXAMPLE_CALLOUT_WARNING = `:::warning Rate Limiting
API node \`fetch-market-data\` is approaching the rate limit (847/1000 requests). Consider adding a throttle node.
:::`;

const EXAMPLE_CALLOUT_ERROR = `:::error
Node \`transform-payload\` failed: TypeError — Cannot read property 'items' of undefined.
Input schema mismatch detected. Check upstream node output.
:::`;

const EXAMPLE_CALLOUT_TIP = `:::tip
The \`csv-parse\` → \`filter\` → \`aggregate\` chain can be replaced with a single \`sql-query\` node.
:::`;

// ========== SPOILER EXAMPLES ==========

const EXAMPLE_SPOILER_BLOCK = `:::spoiler Full Stack Trace
\`\`\`
Error: Connection refused at 10.0.0.5:5432
    at TCPConnectWrap.afterConnect [as oncomplete] (net.js:1141:16)
    at Protocol._enqueue (/app/node_modules/pg/lib/protocol.js:28:15)
    at Client.query (/app/node_modules/pg/lib/client.js:123:27)
\`\`\`
:::`;

const EXAMPLE_SPOILER_INLINE = `The credentials are ||admin:supersecret123|| — rotate immediately.

The API key is ||sk-1234-abcd-5678|| and should be rotated.`;

// ========== EMBED EXAMPLES ==========

const EXAMPLE_EMBED_YOUTUBE = `\`\`\`embed
https://youtube.com/watch?v=dQw4w9WgXcQ
\`\`\``;

const EXAMPLE_EMBED_YOUTUBE_OPTIONS = `\`\`\`embed
url: https://youtube.com/watch?v=dQw4w9WgXcQ
start: 45
\`\`\``;

const EXAMPLE_EMBED_GITHUB = `\`\`\`embed
https://github.com/Rheosoph/flow-like
\`\`\``;

const EXAMPLE_EMBED_GITHUB_ISSUE = `\`\`\`embed
https://github.com/Rheosoph/flow-like/issues/525
\`\`\``;

const EXAMPLE_EMBED_TWITTER = `\`\`\`embed
https://x.com/greatco_de/status/2022279156558356739
\`\`\``;

const EXAMPLE_EMBED_REDDIT = `\`\`\`embed
https://www.reddit.com/r/selfhosted/comments/1n41uu6/ive_spent_5000_hours_building_a_typed_workflow/
\`\`\``;

const EXAMPLE_EMBED_SPOTIFY = `\`\`\`embed
https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT
\`\`\``;

const EXAMPLE_EMBED_STACKOVERFLOW = `\`\`\`embed
https://stackoverflow.com/questions/927358/how-do-i-undo-the-most-recent-local-commits-in-git
\`\`\``;

const EXAMPLE_EMBED_LINKEDIN = `\`\`\`embed
https://www.linkedin.com/posts/activity-7123456789012345678
\`\`\``;

const EXAMPLE_EMBED_HACKERNEWS = `\`\`\`embed
https://news.ycombinator.com/item?id=12345
\`\`\``;

const EXAMPLE_EMBED_GENERIC = `\`\`\`embed
https://flow-like.com
\`\`\``;

// ========== MAP EXAMPLES ==========

const EXAMPLE_MAP_SINGLE = `\`\`\`map
lat: 48.1351
lng: 11.5820
label: Flow-Like HQ
zoom: 14
\`\`\``;

const EXAMPLE_MAP_MULTI_CSV = `\`\`\`map
title: Global Infrastructure
zoom: 2
---
lat,lng,label,color
48.1351,11.5820,EU-West (Primary),red
37.7749,-122.4194,US-West,blue
35.6762,139.6503,AP-Northeast,orange
\`\`\``;

const EXAMPLE_MAP_ROUTE = `\`\`\`map
type: route
mode: driving
---
lat,lng,label
48.1351,11.5820,Munich Office
48.2082,16.3738,Vienna DC
47.4979,19.0402,Budapest Client
\`\`\``;

const EXAMPLE_MAP_JSON = `\`\`\`map
{
  "zoom": 4,
  "markers": [
    { "lat": 48.1351, "lng": 11.5820, "label": "EU-West", "color": "blue" },
    { "lat": 37.7749, "lng": -122.4194, "label": "US-West", "color": "green" }
  ]
}
\`\`\``;

// ========== COMBINED FULL EXAMPLE ==========

const EXAMPLE_ALL_ELEMENTS = `# All Extended Markdown Elements

## Callouts

:::info
Informational message — neutral context, stats, metadata.
:::

:::success
All validation checks passed. Ready for deployment.
:::

:::warning TISAX Compliance
Data leaving the EU boundary was detected in node \`s3-upload\`.
:::

:::error
Node \`fetch-api-data\` failed: HTTP 503 Service Unavailable
:::

:::tip
You can parallelize the \`transform-*\` nodes to cut execution time by ~40%.
:::

## Spoilers

The credentials are ||admin:supersecret123|| — rotate immediately.

:::spoiler Raw API Response
\`\`\`json
{ "secret": "sk-1234", "tokens": 48291 }
\`\`\`
:::

## Embeds

\`\`\`embed
https://github.com/Rheosoph/flow-like
\`\`\`

\`\`\`embed
https://x.com/elikiiii/status/1802394840273530979
\`\`\`

\`\`\`embed
https://www.reddit.com/r/selfhosted/comments/1abc123/flow_like
\`\`\`

\`\`\`embed
https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT
\`\`\`

## Maps

\`\`\`map
lat: 48.1351
lng: 11.5820
label: Munich Office
zoom: 14
\`\`\`
`;

const CALLOUT_EXAMPLES = [
	{ title: "Info", content: EXAMPLE_CALLOUT_INFO },
	{ title: "Success", content: EXAMPLE_CALLOUT_SUCCESS },
	{ title: "Warning (custom title)", content: EXAMPLE_CALLOUT_WARNING },
	{ title: "Error", content: EXAMPLE_CALLOUT_ERROR },
	{ title: "Tip", content: EXAMPLE_CALLOUT_TIP },
];

const SPOILER_EXAMPLES = [
	{ title: "Block Spoiler", content: EXAMPLE_SPOILER_BLOCK },
	{ title: "Inline Spoiler", content: EXAMPLE_SPOILER_INLINE },
];

const EMBED_EXAMPLES = [
	{ title: "YouTube", content: EXAMPLE_EMBED_YOUTUBE },
	{ title: "YouTube (with options)", content: EXAMPLE_EMBED_YOUTUBE_OPTIONS },
	{ title: "GitHub Repo", content: EXAMPLE_EMBED_GITHUB },
	{ title: "GitHub Issue", content: EXAMPLE_EMBED_GITHUB_ISSUE },
	{ title: "X (Twitter)", content: EXAMPLE_EMBED_TWITTER },
	{ title: "Reddit", content: EXAMPLE_EMBED_REDDIT },
	{ title: "Spotify", content: EXAMPLE_EMBED_SPOTIFY },
	{ title: "Stack Overflow", content: EXAMPLE_EMBED_STACKOVERFLOW },
	{ title: "LinkedIn", content: EXAMPLE_EMBED_LINKEDIN },
	{ title: "Hacker News", content: EXAMPLE_EMBED_HACKERNEWS },
	{ title: "Generic", content: EXAMPLE_EMBED_GENERIC },
];

const MAP_EXAMPLES = [
	{ title: "Single Location", content: EXAMPLE_MAP_SINGLE },
	{ title: "Multi Marker (CSV)", content: EXAMPLE_MAP_MULTI_CSV },
	{ title: "Route", content: EXAMPLE_MAP_ROUTE },
	{ title: "JSON Mode", content: EXAMPLE_MAP_JSON },
];

export default function DebugMarkdownPage() {
	const { t } = useTranslation("common");
	const [customMarkdown, setCustomMarkdown] = useState(EXAMPLE_FULL_MARKDOWN);

	return (
		<div className="container mx-auto py-8 space-y-8 px-2 md:px-4">
			<div>
				<h1 className="text-3xl font-bold mb-2">
					{t("markdownDebugPreview", "Markdown Debug Preview")}
				</h1>
				<p className="text-muted-foreground">
					{t(
						"debugPageForTestingMarkdownRenderingIncludingChartsCalloutsSpoilersEmbedsAndMaps",
						"Debug page for testing markdown rendering including charts, callouts, spoilers, embeds, and maps.",
					)}
				</p>
			</div>

			{/* Live Editor */}
			<section className="space-y-4" data-doc-screenshot="live-editor">
				<h2 className="text-xl font-semibold">
					{t("liveEditor", "Live Editor")}
				</h2>
				<div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
					<div>
						<label
							htmlFor="markdown-debug-source"
							className="block text-sm font-medium mb-2"
						>
							{t("rawMarkdown", "Raw Markdown")}
						</label>
						<textarea
							id="markdown-debug-source"
							className="w-full h-[600px] p-4 font-mono text-sm bg-muted/50 rounded-md border resize-none"
							value={customMarkdown}
							onChange={(e) => setCustomMarkdown(e.target.value)}
						/>
					</div>
					<div data-doc-screenshot="rendered-output">
						<div className="block text-sm font-medium mb-2">
							{t("renderedOutput", "Rendered Output")}
						</div>
						<div className="h-[600px] p-4 bg-background border rounded-md overflow-auto">
							<TextEditor
								key={customMarkdown}
								initialContent={customMarkdown}
								isMarkdown={true}
								editable={false}
							/>
						</div>
					</div>
				</div>
				<div className="flex flex-wrap gap-2">
					<button
						type="button"
						className="text-xs px-3 py-1.5 rounded-md border bg-muted/50 hover:bg-muted"
						onClick={() => setCustomMarkdown(EXAMPLE_ALL_ELEMENTS)}
					>
						{t("loadAllElements", "Load All Elements")}
					</button>
					<button
						type="button"
						className="text-xs px-3 py-1.5 rounded-md border bg-muted/50 hover:bg-muted"
						onClick={() => setCustomMarkdown(EXAMPLE_FULL_MARKDOWN)}
					>
						{t("loadBasicMarkdown", "Load Basic Markdown")}
					</button>
				</div>
			</section>

			{/* Callout Examples */}
			<ExampleSection
				title={t("calloutAdmonitionExamples", "Callout / Admonition Examples")}
				examples={CALLOUT_EXAMPLES}
				onLoad={setCustomMarkdown}
			/>

			{/* Spoiler Examples */}
			<ExampleSection
				title={t("spoilerExamples", "Spoiler Examples")}
				examples={SPOILER_EXAMPLES}
				onLoad={setCustomMarkdown}
			/>

			{/* Embed Examples */}
			<ExampleSection
				title={t("embedExamples", "Embed Examples")}
				examples={EMBED_EXAMPLES}
				onLoad={setCustomMarkdown}
			/>

			{/* Map Examples */}
			<ExampleSection
				title={t("mapExamples", "Map Examples")}
				examples={MAP_EXAMPLES}
				onLoad={setCustomMarkdown}
			/>

			{/* Chart Examples */}
			<ExampleSection
				title={t("chartExamples", "Chart Examples")}
				examples={CHART_EXAMPLES}
				onLoad={setCustomMarkdown}
			/>
		</div>
	);
}

function ExampleSection({
	title,
	examples,
	onLoad,
}: {
	title: string;
	examples: { title: string; content: string }[];
	onLoad: (content: string) => void;
}) {
	const { t } = useTranslation("common");
	return (
		<section className="space-y-4">
			<h2 className="text-xl font-semibold">{title}</h2>
			<div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-4">
				{examples.map((example) => (
					<div
						key={example.title}
						data-example-title={example.title}
						className="border rounded-lg overflow-hidden"
					>
						<div className="bg-muted/50 px-4 py-2 flex items-center justify-between">
							<h3 className="font-medium text-sm">{example.title}</h3>
							<div className="flex gap-2">
								<button
									type="button"
									className="text-xs text-muted-foreground hover:text-foreground"
									onClick={() => {
										void navigator.clipboard.writeText(example.content);
									}}
								>
									{t("copy", "Copy")}
								</button>
								<button
									type="button"
									className="text-xs text-muted-foreground hover:text-foreground"
									onClick={() => onLoad(example.content)}
								>
									{t("load", "Load")}
								</button>
							</div>
						</div>
						<div className="p-4 min-h-[200px]">
							<TextEditor
								initialContent={example.content}
								isMarkdown={true}
								editable={false}
							/>
						</div>
					</div>
				))}
			</div>
		</section>
	);
}
