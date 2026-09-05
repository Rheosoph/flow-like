// @vitest-environment happy-dom

import {
	ActionProvider,
	useExecuteAction,
} from "@flow-like/flow-like-ui/components/a2ui/ActionHandler";
import { act, createElement } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	push: vi.fn(),
	backend: { eventState: { alwaysRemote: true }, boardState: {} },
}));

vi.mock("@flow-like/locales", () => ({ i18n: { t: vi.fn() } }));
vi.mock("next/navigation", () => ({
	useRouter: () => ({ push: mocks.push }),
	usePathname: () => "/use",
}));
vi.mock("@flow-like/flow-like-ui/state/backend-state", () => ({
	useBackend: () => mocks.backend,
}));
vi.mock("@flow-like/flow-like-ui/state/execution-service-context", () => ({
	useExecutionServiceOptional: () => undefined,
}));
vi.mock("@flow-like/flow-like-ui/components/a2ui/RouteDialogProvider", () => ({
	useRouteDialogSafe: () => undefined,
}));
vi.mock(
	"@flow-like/flow-like-ui/components/a2ui/layout/A2UIWidgetInstance",
	() => ({
		useWidgetInstance: () => undefined,
		resolveWidgetInstanceEventRoute: vi.fn(),
	}),
);
vi.mock(
	"@flow-like/flow-like-ui/components/a2ui/hooks/use-element-storage",
	() => ({
		useElementStorage: () => ({
			storeElementValue: vi.fn(),
			restoreSurfaceValues: vi.fn(),
		}),
	}),
);
vi.mock("@flow-like/flow-like-ui/lib/idb-storage", () => ({
	appGlobalState: {},
	pageLocalState: {},
}));
vi.mock("@flow-like/flow-like-ui/lib/channel", () => ({
	isChannelHandle: () => false,
	replyToChannel: vi.fn(),
}));

let root: Root;
let container: HTMLDivElement;
let executeAction: ReturnType<typeof useExecuteAction>["executeAction"];

function ActionProbe() {
	executeAction = useExecuteAction().executeAction;
	return null;
}

beforeEach(() => {
	vi.clearAllMocks();
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	vi.spyOn(console, "log").mockImplementation(() => {});
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	vi.restoreAllMocks();
	vi.unstubAllGlobals();
});

test.each([false, true])(
	"null queryParams do not interrupt navigation (intercepted: %s)",
	async (intercepted) => {
		const onNavigationMessage = intercepted ? vi.fn() : undefined;
		await act(async () => {
			root.render(
				createElement(ActionProvider, {
					surfaceId: "page-id",
					isPreviewMode: true,
					onNavigationMessage,
					// biome-ignore lint/correctness/noChildrenProp: ActionProvider requires children in its props type.
					children: createElement(ActionProbe),
				}),
			);
		});

		await act(async () => {
			await executeAction({
				name: "navigate_page",
				context: { route: "/orders", queryParams: "null" },
			});
		});

		expect(console.log).toHaveBeenCalledWith("[ActionHandler] navigate_page", {
			hasRoute: true,
			hasAppContext: false,
			queryParamKeys: [],
		});
		if (onNavigationMessage) {
			expect(onNavigationMessage).toHaveBeenCalledExactlyOnceWith({
				type: "navigateTo",
				route: "/orders",
				replace: false,
			});
			expect(mocks.push).not.toHaveBeenCalled();
		} else {
			expect(mocks.push).toHaveBeenCalledExactlyOnceWith("/orders");
		}
	},
);
