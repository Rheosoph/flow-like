import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

export function tmpDir(prefix: string): string {
	return mkdtempSync(join(tmpdir(), `${prefix}-`));
}

export const HELLO_WIDGET_CONFIG = `import { defineWidget } from "@flow-like/widget-sdk";

interface Inputs {
	/** Greeting text @default "Hello" */
	greeting: string;
}

interface Events {
	dismissed: void;
}

interface Queries {
	getGreeting: { args: void; returns: string };
}

export default defineWidget<Inputs, Events, Queries>({
	id: "hello-widget",
	name: "Hello Widget",
	description: "Says hello",
	sizing: { defaultHeight: 200, resizable: false, maxHeight: 600 },
});
`;

export const HELLO_BUILT_HTML = `<!doctype html>
<html lang="en">
	<head>
		<meta charset="utf-8" />
		<title>Hello Widget</title>
		<link rel="modulepreload" crossorigin href="/shared/react-abc123.js" />
		<link rel="stylesheet" href="./style.css" />
	</head>
	<body>
		<div id="root"></div>
		<script type="module" crossorigin src="/shared/react-abc123.js"></script>
		<script type="module" src="./index.js"></script>
	</body>
</html>
`;

export interface ProjectFixture {
	projectDir: string;
	groupDir: string;
	widgetConfigPath: string;
	builtHtmlPath: string;
}

/** Synthetic package project: flow-like.toml + one react group with a prebuilt dist. */
export function makeProjectFixture(): ProjectFixture {
	const projectDir = tmpDir("flwb-project");
	writeFileSync(
		join(projectDir, "flow-like.toml"),
		'id = "com.example.demo"\nversion = "1.2.0"\n',
	);

	const groupDir = join(projectDir, "widgets", "react");
	const srcWidget = join(groupDir, "src", "widgets", "hello-widget");
	mkdirSync(srcWidget, { recursive: true });
	writeFileSync(
		join(groupDir, "package.json"),
		JSON.stringify({ name: "react-group", dependencies: { react: "^19.0.0" } }),
	);
	const widgetConfigPath = join(srcWidget, "widget.config.ts");
	writeFileSync(widgetConfigPath, HELLO_WIDGET_CONFIG);
	writeFileSync(
		join(srcWidget, "index.html"),
		'<!doctype html>\n<html>\n\t<head><title>Hello Widget</title></head>\n\t<body><script type="module" src="./index.ts"></script></body>\n</html>\n',
	);

	const distWidget = join(groupDir, "dist", "src", "widgets", "hello-widget");
	mkdirSync(distWidget, { recursive: true });
	mkdirSync(join(groupDir, "dist", "shared"), { recursive: true });
	const builtHtmlPath = join(distWidget, "index.html");
	writeFileSync(builtHtmlPath, HELLO_BUILT_HTML);
	writeFileSync(join(distWidget, "index.js"), 'console.log("hello entry");\n');
	writeFileSync(join(distWidget, "style.css"), "#root { color: red; }\n");
	writeFileSync(
		join(groupDir, "dist", "shared", "react-abc123.js"),
		'console.log("react runtime");\n',
	);

	return { projectDir, groupDir, widgetConfigPath, builtHtmlPath };
}
