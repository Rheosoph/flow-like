// @vitest-environment happy-dom

import type { Surface } from "@flow-like/flow-like-ui/components/a2ui/types";
import {
	type NativeWidgetPageDefinition,
	nativeWidgetScope,
	saveNativeWidgetDefinitions,
} from "@flow-like/flow-like-ui/lib/native-widget";
import {
	NATIVE_WIDGET_PAGE_CAPTURE,
	type NativeWidgetPageCaptureDetail,
} from "@flow-like/flow-like-ui/lib/native-widget-page";
import type { IPage } from "@flow-like/flow-like-ui/state/backend-state/page-state";
import React, { act, createElement } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	backend: {
		profile: { id: "profile-a", hub: "api.test" },
		userState: { getProfile: vi.fn() },
	},
	profile: {
		isSuccess: true,
		isFetchedAfterMount: true,
		data: { id: "profile-a", hub: "api.test" },
	},
	auth: {
		isLoading: false,
		isAuthenticated: true,
		user: { profile: { sub: "viewer-a" } },
	},
	value: "Account A content",
}));
vi.mock("@flow-like/flow-like-ui/state/backend-state", () => ({
	useBackend: () => mocks.backend,
	useBackendReady: () => true,
}));
vi.mock("@flow-like/flow-like-ui/hooks/use-invoke", () => ({
	useInvoke: () => mocks.profile,
}));
vi.mock("react-oidc-context", () => ({ useAuth: () => mocks.auth }));
vi.mock("@flow-like/flow-like-ui/components/a2ui/DataContext", () => ({
	useData: () => ({ data: { value: mocks.value } }),
}));

import { NativeWidgetPageCaptureBridge } from "@flow-like/flow-like-ui/components/interfaces/native-widget-page-capture";

const page: IPage = {
	id: "page",
	name: "Page",
	content: [],
	components: [],
	layoutType: "stack",
	createdAt: "2026-09-01",
	updatedAt: "2026-09-13",
};
const surface: Surface = {
	id: "page",
	rootComponentId: "root",
	components: {
		root: {
			id: "root",
			component: { id: "root", type: "text", content: { path: "value" } },
		},
	},
};
const definition: NativeWidgetPageDefinition = {
	id: "page-widget",
	kind: "page",
	appId: "app",
	title: "Page",
	path: "/",
	queryParams: [],
	accent: "orange",
	refreshMinutes: 30,
	updatedAt: "2026-09-13T10:00:00.000Z",
};
let root: Root;
let host: HTMLDivElement;
let captures: NativeWidgetPageCaptureDetail[];
const onCapture = (event: Event) =>
	captures.push((event as CustomEvent<NativeWidgetPageCaptureDetail>).detail);
const scope = () =>
	nativeWidgetScope(mocks.backend as never, mocks.auth.user.profile.sub);
async function render(key = "original-page") {
	await act(async () =>
		root.render(
			createElement(NativeWidgetPageCaptureBridge, {
				key,
				appId: "app",
				page,
				surface,
				path: "/",
				search: "",
				pageRevision: "v1",
				ready: true,
			}),
		),
	);
}
async function flush() {
	await act(async () => {
		await vi.advanceTimersByTimeAsync(600);
	});
}

beforeEach(() => {
	vi.useFakeTimers();
	vi.stubGlobal("React", React);
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	const values = new Map<string, string>();
	vi.stubGlobal("localStorage", {
		getItem: (key: string) => values.get(key) ?? null,
		setItem: (key: string, value: string) => values.set(key, value),
	});
	mocks.backend.profile = { id: "profile-a", hub: "api.test" };
	mocks.profile.data = { ...mocks.backend.profile };
	mocks.profile.isFetchedAfterMount = true;
	mocks.profile.isSuccess = true;
	mocks.auth.isLoading = false;
	mocks.auth.user.profile.sub = "viewer-a";
	mocks.value = "Account A content";
	captures = [];
	window.addEventListener(NATIVE_WIDGET_PAGE_CAPTURE, onCapture);
	saveNativeWidgetDefinitions(scope(), [definition]);
	host = document.createElement("div");
	document.body.appendChild(host);
	root = createRoot(host);
});
afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
	window.removeEventListener(NATIVE_WIDGET_PAGE_CAPTURE, onCapture);
	vi.useRealTimers();
	vi.unstubAllGlobals();
});

describe("native page capture identity", () => {
	it("waits for a freshly observed profile before capturing the page", async () => {
		mocks.profile.isFetchedAfterMount = false;
		await render();
		await flush();
		expect(captures).toEqual([]);
		mocks.profile.isFetchedAfterMount = true;
		await render();
		await flush();
		expect(captures).toHaveLength(1);
		expect(captures[0].scope).toBe(scope());
	});

	it("does not relabel an existing page surface when the workspace changes", async () => {
		await render();
		await flush();
		captures.length = 0;
		mocks.backend.profile = { id: "profile-b", hub: "other-api.test" };
		mocks.profile.data = { ...mocks.backend.profile };
		await act(async () => saveNativeWidgetDefinitions(scope(), [definition]));
		await render();
		await flush();
		expect(captures).toEqual([]);
		mocks.value = "New workspace content";
		await render("remounted-page");
		await flush();
		expect(captures).toHaveLength(1);
		expect(captures[0].scope).toBe(scope());
		expect(captures[0].result.root?.text).toBe("New workspace content");
	});

	it("cancels pending captures during authentication changes and waits for a new page", async () => {
		await render();
		mocks.auth.isLoading = true;
		await render();
		await flush();
		expect(captures).toEqual([]);
		mocks.auth.isLoading = false;
		mocks.auth.user.profile.sub = "viewer-b";
		await act(async () => saveNativeWidgetDefinitions(scope(), [definition]));
		await render();
		await flush();
		expect(captures).toEqual([]);
		mocks.value = "Account B content";
		await render("new-account-page");
		await flush();
		expect(captures[0].scope).toBe(scope());
		expect(captures[0].result.root?.text).toBe("Account B content");
	});
});
