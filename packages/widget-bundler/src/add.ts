import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { isValidWidgetId } from "./contract-types";

export interface AddResult {
	widgetDir: string;
	files: string[];
}

function displayName(widgetId: string): string {
	return widgetId
		.split("-")
		.map((part) => part.charAt(0).toUpperCase() + part.slice(1))
		.join(" ");
}

function widgetConfigTemplate(widgetId: string, name: string): string {
	return `import { defineWidget } from "@flow-like/widget-sdk";

interface Inputs {
	/** Widget headline @default "${name}" */
	title: string;
}

interface Events {
	titleClicked: { title: string };
}

interface Queries {
	getTitle: { args: void; returns: string };
}

export default defineWidget<Inputs, Events, Queries>({
	id: "${widgetId}",
	name: "${name}",
	description: "Describe what this widget does.",
	sizing: { defaultHeight: 320, resizable: true },
});
`;
}

function indexHtmlTemplate(name: string): string {
	return `<!doctype html>
<html lang="en">
	<head>
		<meta charset="utf-8" />
		<meta name="viewport" content="width=device-width, initial-scale=1" />
		<title>${name}</title>
	</head>
	<body>
		<div id="root"></div>
		<script type="module" src="./index.ts"></script>
	</body>
</html>
`;
}

function indexTsTemplate(): string {
	return `import { mountFlowWidget } from "@flow-like/widget-sdk";
import widget from "./widget.config";

const { $props, emit, onQuery } = mountFlowWidget(widget);

const root = document.getElementById("root");
if (root) {
	$props.subscribe((props) => {
		root.textContent = props.title;
	});
	root.addEventListener("click", () => {
		emit("titleClicked", { title: $props.get().title });
	});
}

onQuery("getTitle", () => $props.get().title);
`;
}

/** Scaffold `src/widgets/<id>/` inside a framework group. */
export function addWidget(groupDir: string, widgetId: string): AddResult {
	if (!isValidWidgetId(widgetId)) {
		throw new Error(
			`Invalid widget id '${widgetId}': must be non-empty lowercase kebab-case ([a-z0-9-])`,
		);
	}
	const group = resolve(groupDir);
	if (!existsSync(group)) {
		throw new Error(`Framework group directory not found: ${group}`);
	}
	const widgetDir = join(group, "src", "widgets", widgetId);
	if (existsSync(widgetDir)) {
		throw new Error(`Widget directory already exists: ${widgetDir}`);
	}
	mkdirSync(widgetDir, { recursive: true });

	const name = displayName(widgetId);
	const files = [
		["widget.config.ts", widgetConfigTemplate(widgetId, name)],
		["index.html", indexHtmlTemplate(name)],
		["index.ts", indexTsTemplate()],
	] as const;

	const written: string[] = [];
	for (const [file, content] of files) {
		const path = join(widgetDir, file);
		writeFileSync(path, content);
		written.push(path);
	}
	return { widgetDir, files: written };
}
