import { afterAll, afterEach, beforeEach, expect, test } from "bun:test";
import { act, createElement } from "react";
import {
	installDomGlobals,
	restoreDomGlobals,
	settle,
} from "../../../bots-tab/testing/dom-harness";

const { createRoot } = await import("react-dom/client");
const { QueryClient, QueryClientProvider } = await import(
	"@tanstack/react-query"
);
const { useBackendStore } = await import("../../../../state/backend-state");
const { useWidgetDocuments } = await import("./use-widget-documents");
import type { IInboxFileHandle } from "../../../../state/backend-state/inbox-state";
import type { IChatWidget } from "../chat-db";

const priorBackend = useBackendStore.getState().backend;
const chartDocument = {
	type: "app_widget",
	origin: { app_id: "app1", board_id: "b1" },
	widget: {
		instance_id: "w1",
		widget_id: "chart",
		surface_id: "s1",
		component: { widgetInstance: {} },
	},
};
const files: Record<string, unknown> = {
	"ui1.json": chartDocument,
	"ui2.json": { type: "page" },
};
const handle = (path: string): IInboxFileHandle => ({
	app_id: "conversation_room1",
	path,
	bytes: 1,
	sha256: path,
	mime: "application/json",
});

let root: ReturnType<typeof createRoot>;
let client: InstanceType<typeof QueryClient>;
let downloads: string[];
let shown: IChatWidget[] = [];
let warnings = 0;
const warn = console.warn;

function Probe({ documents }: { documents: IInboxFileHandle[] }) {
	shown = useWidgetDocuments(documents);
	return null;
}

async function show(documents: IInboxFileHandle[]) {
	await act(async () =>
		root.render(
			createElement(
				QueryClientProvider,
				{ client },
				createElement(Probe, { documents }),
			),
		),
	);
	await settle();
}

beforeEach(() => {
	installDomGlobals();
	downloads = [];
	warnings = 0;
	console.warn = () => {
		warnings += 1;
	};
	useBackendStore.getState().setBackend({
		storageState: {},
		inboxState: {
			conversationFileDownload: async (
				conversationId: string,
				{ path }: { path: string },
			) => {
				downloads.push(`${conversationId}/${path}`);
				return {
					url: `data:application/json,${encodeURIComponent(JSON.stringify(files[path]))}`,
					name: path,
					mime: "application/json",
					bytes: 1,
				};
			},
		},
	} as never);
	client = new QueryClient();
	root = createRoot(document.createElement("div"));
});

afterEach(async () => {
	await act(async () => root.unmount());
	client.clear();
	console.warn = warn;
});

afterAll(() => {
	useBackendStore.setState({ backend: priorBackend });
	restoreDomGlobals();
});

test("a stored widget document loads once through its signed link; one that is no app widget is left out", async () => {
	await show([handle("ui1.json"), handle("ui2.json")]);
	expect(shown).toEqual([
		{
			instance_id: "w1",
			widget_id: "chart",
			surface_id: "s1",
			component: { widgetInstance: {} },
			origin: { appId: "app1", boardId: "b1" },
		},
	]);
	await show([handle("ui1.json"), handle("ui2.json")]);
	expect(downloads).toEqual(["room1/ui1.json", "room1/ui2.json"]);
	expect(warnings).toBe(1);
});
