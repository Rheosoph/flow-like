import { afterEach, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import type {
	IBotApp,
	IBotRestrictions,
} from "../../../state/backend-state/bot-state";
import { presetRestrictions } from "../bot-restrictions-model";
import { type BotAppDraft, useBotAppDraft } from "./use-bot-app-draft";

let cleanup: (() => Promise<void>) | undefined;
afterEach(async () => {
	await cleanup?.();
	cleanup = undefined;
});

function app(appId: string, restrictions?: IBotRestrictions): IBotApp {
	return {
		app_id: appId,
		added_at: "2026-11-01T00:00:00Z",
		owner_role: "owner",
		restrictions,
	};
}

async function renderDraft(initial: readonly IBotApp[] | undefined) {
	const window = new Window();
	const globals: Record<string, unknown> = {
		window,
		document: window.document,
		navigator: window.navigator,
		Node: window.Node,
		Element: window.Element,
		HTMLElement: window.HTMLElement,
		IS_REACT_ACT_ENVIRONMENT: true,
	};
	const previous = Object.keys(globals).map(
		(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
	);
	Object.assign(globalThis, globals);
	const { createRoot } = await import("react-dom/client");
	const container = window.document.createElement("div");
	window.document.body.append(container);
	const root = createRoot(container as unknown as HTMLElement);
	let current: BotAppDraft;
	function Probe({ apps }: { apps: readonly IBotApp[] | undefined }) {
		current = useBotAppDraft(apps);
		return null;
	}
	const rerender = (apps: readonly IBotApp[] | undefined) =>
		act(async () => root.render(<Probe apps={apps} />));
	await rerender(initial);
	cleanup = async () => {
		await act(async () => root.unmount());
		await window.happyDOM.close();
		for (const [key, descriptor] of previous) {
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		}
	};
	return {
		current: () => current,
		rerender,
		change: (perform: (draft: BotAppDraft) => void) =>
			act(async () => perform(current)),
	};
}

describe("useBotAppDraft", () => {
	test("app selections become pending changes and reverting restores a clean draft", async () => {
		const view = await renderDraft([app("atlas")]);
		expect(view.current().changes).toEqual([]);
		await view.change((draft) => draft.setAttached("beacon", true));
		expect(view.current().changes).toEqual([
			{ appId: "beacon", attached: true },
		]);
		expect(view.current().apps[1]).toEqual({
			app_id: "beacon",
			added_at: "",
			owner_role: null,
		});
		await view.change((draft) => draft.setAttached("atlas", false));
		expect(view.current().changes).toEqual([
			{ appId: "atlas", attached: false },
			{ appId: "beacon", attached: true },
		]);
		await view.change((draft) => {
			draft.setAttached("atlas", true);
			draft.setAttached("beacon", false);
		});
		expect(view.current().changes).toEqual([]);
	});

	test("role edits survive deselection and can be reverted", async () => {
		const view = await renderDraft([app("atlas")]);
		const readOnly = presetRestrictions("read_only");
		await view.change((draft) => draft.setRestrictions("atlas", readOnly));
		expect(view.current().changes).toEqual([
			{ appId: "atlas", restrictions: readOnly },
		]);
		await view.change((draft) => draft.setAttached("atlas", false));
		expect(view.current().changes).toEqual([
			{ appId: "atlas", attached: false },
		]);
		await view.change((draft) => draft.setAttached("atlas", true));
		expect(view.current().apps[0].restrictions).toEqual(readOnly);
		await view.change((draft) =>
			draft.setRestrictions("atlas", presetRestrictions("full")),
		);
		expect(view.current().changes).toEqual([]);
	});

	test("permission and scope ordering and omitted unrestricted scopes are equivalent", async () => {
		const original: IBotRestrictions = {
			permissions: ["read_files", "read_database"],
			scopes: {
				files: { prefixes: ["invoices", "reports"], read_only: true },
				tables: ["sales", "customers"],
				events: null,
			},
		};
		const view = await renderDraft([app("atlas", original)]);
		await view.change((draft) =>
			draft.setRestrictions("atlas", {
				permissions: ["read_database", "read_files"],
				scopes: {
					files: { prefixes: ["reports", "invoices"], read_only: true },
					tables: ["customers", "sales"],
					boards: null,
				},
			}),
		);
		expect(view.current().changes).toEqual([]);
		await view.change((draft) =>
			draft.setRestrictions("atlas", {
				...original,
				scopes: { ...original.scopes, events: [] },
			}),
		);
		expect(view.current().changes).toHaveLength(1);
	});

	test("the first loaded app list remains the baseline across refetches", async () => {
		const view = await renderDraft(undefined);
		expect(view.current().loading).toBe(true);
		const loaded = presetRestrictions("read_only");
		await view.rerender([app("atlas", loaded)]);
		expect(view.current().loading).toBe(false);
		await view.change((draft) => draft.setAttached("beacon", true));
		loaded.permissions?.push("write_files");
		await view.rerender([app("cedar")]);
		expect(view.current().apps.map((entry) => entry.app_id)).toEqual([
			"atlas",
			"beacon",
		]);
		expect(view.current().apps[0].restrictions).toEqual(
			presetRestrictions("read_only"),
		);
		expect(view.current().changes).toEqual([
			{ appId: "beacon", attached: true },
		]);
	});

	test("a partial save retries only unfinished attachments and restrictions", async () => {
		const view = await renderDraft([app("atlas")]);
		const readOnly = presetRestrictions("read_only");
		await view.change((draft) => {
			draft.setAttached("atlas", false);
			draft.setAttached("beacon", true);
			draft.setRestrictions("beacon", readOnly);
			draft.setAttached("cedar", true);
		});
		await view.change((draft) => {
			draft.markAttachedSaved("atlas", false);
			draft.markAttachedSaved("beacon", true);
		});
		expect(view.current().changes).toEqual([
			{ appId: "beacon", restrictions: readOnly },
			{ appId: "cedar", attached: true },
		]);
		await view.change((draft) =>
			draft.markRestrictionsSaved("beacon", readOnly),
		);
		expect(view.current().changes).toEqual([
			{ appId: "cedar", attached: true },
		]);
		await view.change((draft) => draft.markAttachedSaved("cedar", true));
		expect(view.current().changes).toEqual([]);
	});

	test("acknowledging a saved role keeps a newer role edit pending", async () => {
		const view = await renderDraft([app("atlas")]);
		const readOnly = presetRestrictions("read_only");
		const readAndCall = presetRestrictions("read_and_call_events");
		await view.change((draft) => draft.setRestrictions("atlas", readOnly));
		await view.change((draft) => draft.setRestrictions("atlas", readAndCall));
		await view.change((draft) =>
			draft.markRestrictionsSaved("atlas", readOnly),
		);
		expect(view.current().changes).toEqual([
			{ appId: "atlas", restrictions: readAndCall },
		]);
	});
});
