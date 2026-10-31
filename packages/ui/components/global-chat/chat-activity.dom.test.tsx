// @vitest-environment happy-dom

import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
	type ChatActivityTarget,
	recordChatActivity,
	useChatActivityStore,
} from "../../state/chat-activity-store";
import {
	ChatActivityBadge,
	ChatActivityMenu,
	ChatActivityProvider,
	useChatActivityVisible,
} from "./chat-activity";

const controls = vi.hoisted(() => ({
	pathname: "/library",
	push: vi.fn(),
	restore: vi.fn(),
	openOverlay: vi.fn(),
	dismissOverlay: vi.fn(),
	toast: Object.assign(vi.fn(), { dismiss: vi.fn(), error: vi.fn() }),
	t: (_key: string, fallback: string, values?: Record<string, unknown>) =>
		fallback.replace(/\{\{(\w+)\}\}/g, (_, name) =>
			String(values?.[name] ?? ""),
		),
}));

vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({ t: controls.t, i18n: { language: "en" } }),
}));
vi.mock("next/navigation", () => ({ usePathname: () => controls.pathname }));
vi.mock("../../lib/client-navigation", () => ({
	useClientRouter: () => ({ push: controls.push }),
}));
vi.mock("../../state/global-chat/global-chat-stream", () => ({
	restoreGlobalChatConversation: controls.restore,
}));
vi.mock("../../state/global-chat/global-chat-store", () => ({
	useGlobalChatStore: {
		getState: () => ({
			openOverlay: controls.openOverlay,
			dismissOverlay: controls.dismissOverlay,
		}),
	},
}));
vi.mock("sonner", () => ({ toast: controls.toast }));

let root: Root;
let container: HTMLDivElement;
let nextMessage = 0;

function VisibleChat({ target }: { target: ChatActivityTarget }) {
	useChatActivityVisible(target);
	return null;
}

function reply(target: ChatActivityTarget, title = "Research complete") {
	return recordChatActivity({
		target,
		messageId: `ui-test-message-${++nextMessage}`,
		title,
		preview: "The report is ready to review.",
	});
}

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	vi.spyOn(document, "hasFocus").mockReturnValue(true);
	vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
	useChatActivityStore.getState().clear();
	controls.pathname = "/library";
	controls.restore.mockResolvedValue(true);
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	useChatActivityStore.getState().clear();
	vi.restoreAllMocks();
	vi.clearAllMocks();
});

describe("conversation activity", () => {
	it("shows a popup and keeps the badge until the opened conversation is visible", async () => {
		const target = { kind: "flowpilot", sessionId: "background-chat" } as const;
		await act(async () =>
			root.render(
				<>
					<ChatActivityProvider />
					<ChatActivityBadge />
				</>,
			),
		);
		await act(async () => {
			reply(target);
		});

		expect(controls.toast).toHaveBeenCalledTimes(1);
		expect(container.querySelector("output")?.textContent).toBe("1");
		const options = controls.toast.mock.calls[0][1];
		expect(options.closeButton).toBe(true);
		expect(options.action.label).toBe("Open conversation");
		await act(async () => options.action.onClick());
		await vi.waitFor(() =>
			expect(controls.restore).toHaveBeenCalledWith("background-chat"),
		);
		expect(controls.openOverlay).toHaveBeenCalledTimes(1);
		expect(useChatActivityStore.getState().entries).toHaveLength(1);
		controls.toast.dismiss.mockClear();

		await act(async () =>
			root.render(
				<>
					<ChatActivityProvider />
					<ChatActivityBadge />
					<VisibleChat target={target} />
				</>,
			),
		);
		expect(useChatActivityStore.getState().entries).toHaveLength(0);
		expect(container.querySelector("output")).toBeNull();
		expect(controls.toast.dismiss).toHaveBeenCalledWith(options.id);
	});

	it("closes the FlowPilot dock before opening encoded app targets without marking them read", async () => {
		const target = {
			kind: "app",
			appId: "app &/name",
			eventId: "event?next=elsewhere",
			sessionId: "chat #1/2",
		} as const;
		reply(target, "Inventory bot");
		await act(async () => root.render(<ChatActivityMenu />));
		await act(async () =>
			container.querySelector<HTMLButtonElement>("button")?.click(),
		);
		const entry = Array.from(
			document.querySelectorAll<HTMLButtonElement>("button"),
		).find((button) => button.textContent?.includes("Inventory bot"));
		expect(entry).toBeDefined();
		expect(useChatActivityStore.getState().entries).toHaveLength(1);
		await act(async () => entry?.click());

		expect(controls.push).toHaveBeenCalledTimes(1);
		expect(controls.dismissOverlay).toHaveBeenCalledTimes(1);
		expect(controls.dismissOverlay.mock.invocationCallOrder[0]).toBeLessThan(
			controls.push.mock.invocationCallOrder[0],
		);
		const url = new URL(controls.push.mock.calls[0][0], "https://example.test");
		expect(url.pathname).toBe("/use");
		expect(Object.fromEntries(url.searchParams)).toEqual({
			id: target.appId,
			eventId: target.eventId,
			sessionId: target.sessionId,
		});
		expect(useChatActivityStore.getState().entries).toHaveLength(1);
		await act(async () => root.render(<VisibleChat target={target} />));
		expect(useChatActivityStore.getState().entries).toHaveLength(0);
	});

	it("retains unread activity when restoring a conversation fails", async () => {
		controls.restore.mockRejectedValue(new Error("Database unavailable"));
		await act(async () => root.render(<ChatActivityProvider />));
		reply({ kind: "flowpilot", sessionId: "failed-chat" });
		await act(async () => controls.toast.mock.calls[0][1].action.onClick());
		await vi.waitFor(() =>
			expect(controls.toast.error).toHaveBeenCalledWith(
				"Could not open this conversation.",
			),
		);
		expect(controls.openOverlay).not.toHaveBeenCalled();
		expect(useChatActivityStore.getState().entries).toHaveLength(1);
	});

	it("updates the badge when another window reads the conversation", async () => {
		await act(async () =>
			root.render(
				<>
					<ChatActivityProvider />
					<ChatActivityBadge />
				</>,
			),
		);
		await act(async () => {
			reply({ kind: "flowpilot", sessionId: "another-window" });
		});
		expect(container.querySelector("output")?.textContent).toBe("1");
		const key = "flow-like:chat-activity:v1";
		const saved = JSON.parse(localStorage.getItem(key) ?? "{}");
		localStorage.setItem(key, JSON.stringify({ ...saved, entries: [] }));
		await act(async () => {
			window.dispatchEvent(new StorageEvent("storage", { key }));
		});
		expect(container.querySelector("output")).toBeNull();
		expect(controls.toast).toHaveBeenCalledTimes(1);
		expect(controls.toast.dismiss).toHaveBeenCalledWith(
			controls.toast.mock.calls[0][1].id,
		);
	});

	it("suppresses active chat popups and removes the listener on unmount", async () => {
		const target = { kind: "flowpilot", sessionId: "visible-chat" } as const;
		await act(async () =>
			root.render(
				<>
					<ChatActivityProvider />
					<VisibleChat target={target} />
				</>,
			),
		);
		reply(target);
		expect(controls.toast).not.toHaveBeenCalled();
		expect(useChatActivityStore.getState().entries).toHaveLength(0);
		await act(async () => root.render(null));
		reply({ kind: "flowpilot", sessionId: "unmounted-chat" });
		expect(controls.toast).not.toHaveBeenCalled();
		expect(useChatActivityStore.getState().entries).toHaveLength(1);
	});
});
