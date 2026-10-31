import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { IEventPayload } from "../../../lib/schema/flow/event-payload";
import {
	BackendContext,
	type IBackendState,
} from "../../../state/backend-state";
import {
	byRole,
	click,
	installDom,
	settle,
} from "../../settings/devices/testing/dom-harness";
import type { IConfigInterfaceProps } from "../interfaces";

const dom = installDom();
const { SimpleChatConfig } = await import("./simple_chat");
const clients: QueryClient[] = [];

afterEach(async () => {
	await dom.cleanup();
	for (const client of clients.splice(0)) client.clear();
});
afterAll(dom.restore);

async function render(props: Partial<IConfigInterfaceProps> = {}) {
	const updates: IEventPayload[] = [];
	const client = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	clients.push(client);
	const backend = {
		routeState: {
			async getRoutes() {
				return [];
			},
		},
	} as unknown as IBackendState;
	const view = await dom.render(
		<QueryClientProvider client={client}>
			<BackendContext.Provider value={backend}>
				<SimpleChatConfig
					isEditing
					appId="app"
					boardId="board"
					nodeId="node"
					node={{ id: "node" } as IConfigInterfaceProps["node"]}
					config={{ history_elements: 10 }}
					onConfigUpdate={(payload) => updates.push(payload)}
					section="capabilities"
					{...props}
				/>
			</BackendContext.Provider>
		</QueryClientProvider>,
	);
	await settle();
	return { ...view, updates };
}

describe("chat model settings", () => {
	test("existing events enable model choice and leave the vision filter off", async () => {
		await render();
		expect(byRole("switch", "Model Picker").getAttribute("aria-checked")).toBe(
			"true",
		);
		expect(
			byRole("switch", "Require Vision Model").getAttribute("aria-checked"),
		).toBe("false");
	});

	test("changing either setting preserves the remaining event config", async () => {
		const { updates } = await render();
		await click(byRole("switch", "Require Vision Model"));
		expect(updates.at(-1)).toEqual({ history_elements: 10, require_vlm: true });
		await click(byRole("switch", "Model Picker"));
		expect(updates.at(-1)).toEqual({
			history_elements: 10,
			allow_model_selection: false,
		});
	});

	test("saved settings are shown and a hidden picker disables its vision filter", async () => {
		await render({
			config: { allow_model_selection: false, require_vlm: true },
		});
		expect(byRole("switch", "Model Picker").getAttribute("aria-checked")).toBe(
			"false",
		);
		const vision = byRole(
			"switch",
			"Require Vision Model",
		) as HTMLButtonElement;
		expect(vision.getAttribute("aria-checked")).toBe("true");
		expect(vision.disabled).toBe(true);
	});

	test("read-only event settings cannot change either option", async () => {
		const { updates } = await render({ isEditing: false });
		for (const label of ["Model Picker", "Require Vision Model"]) {
			const toggle = byRole("switch", label) as HTMLButtonElement;
			expect(toggle.disabled).toBe(true);
			await click(toggle);
		}
		expect(updates).toEqual([]);
	});
});
