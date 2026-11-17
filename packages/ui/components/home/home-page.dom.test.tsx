import {
	QueryClient,
	QueryClientProvider,
	focusManager,
} from "@tanstack/react-query";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { HomeEditorProps } from "./home-editor";
import { HomePage } from "./home-page";
import type { IHomeDefaults, IHomeLayout } from "./types";

const fixture = vi.hoisted(() => ({
	profile: {
		id: "personal-profile",
		name: "Personal profile",
		hub: "https://home-fixture.invalid",
		updated: "2026-09-07T10:00:00Z",
		home_default_id: null as string | null,
		home_layout: null as IHomeLayout | null,
	},
	editorProps: null as HomeEditorProps | null,
	pendingLayout: null as IHomeLayout | null,
	saveError: undefined as unknown,
	getProfile: vi.fn(),
	getDefaults: vi.fn(),
	saveLayout: vi.fn(),
}));

vi.mock("react-oidc-context", () => ({
	useAuth: () => ({
		isAuthenticated: true,
		user: { profile: { sub: "personal-user" } },
	}),
}));
vi.mock("../../state/backend-state", () => {
	const userState = {
		getProfile: fixture.getProfile,
		getHomeDefaults: fixture.getDefaults,
		saveHomeLayout: fixture.saveLayout,
	};
	return {
		useBackendReady: () => true,
		useBackend: () => ({ profile: fixture.profile, userState }),
	};
});
vi.mock("../../state/bot-den", () => ({
	useRequestBotDen: () => {},
}));

// Keep the real page and QueryClient so focus events and failed reads exercise
// the same cache transitions as the application. Editor behavior has separate tests.
vi.mock("./home-editor", () => ({
	HomeEditor: function EditorBoundary(props: HomeEditorProps) {
		fixture.editorProps = props;
		const save = async (reset = false) => {
			fixture.saveError = undefined;
			try {
				if (reset) await props.onReset();
				else if (fixture.pendingLayout)
					await props.onSave(fixture.pendingLayout);
				props.onEditingChange?.(false);
			} catch (error) {
				fixture.saveError = error;
			}
		};
		return (
			<section aria-label="Home editor boundary">
				<output aria-label="Rendered layout">{props.layout.title}</output>
				<output aria-label="Inherited layout">
					{props.defaultLayout.title}
				</output>
				<span>{props.sourceLabel}</span>
				<button type="button" onClick={() => props.onEditingChange?.(true)}>
					Edit
				</button>
				<button type="button" onClick={() => props.onEditingChange?.(false)}>
					Discard
				</button>
				<button type="button" onClick={() => void save()}>
					Save
				</button>
				<button type="button" onClick={() => void save(true)}>
					Use default
				</button>
			</section>
		);
	},
}));

const layout = (title: string): IHomeLayout => ({
	version: 1,
	title,
	widgets: [],
});
let defaults: IHomeDefaults;
let client: QueryClient;
let root: Root;
let container: HTMLDivElement;

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	vi.useFakeTimers({ toFake: ["Date"] });
	focusManager.setFocused(true);
	fixture.editorProps = null;
	fixture.pendingLayout = layout("Saved changes");
	fixture.saveError = undefined;
	fixture.profile.home_layout = layout("Original personal home");
	fixture.profile.home_default_id = null;
	fixture.profile.updated = "2026-09-07T10:00:00Z";
	fixture.getProfile.mockReset();
	fixture.getDefaults.mockReset();
	fixture.saveLayout.mockReset();
	defaults = {
		main: { id: "main", revision: "r1", layout: layout("Published default") },
		profile: null,
	};
	fixture.getProfile.mockImplementation(async () =>
		structuredClone(fixture.profile),
	);
	fixture.getDefaults.mockImplementation(async () => structuredClone(defaults));
	fixture.saveLayout.mockImplementation(async (value: IHomeLayout | null) => {
		fixture.profile.home_layout = structuredClone(value);
		fixture.profile.updated = "2026-09-07T10:01:00Z";
		return structuredClone(fixture.profile);
	});
	client = new QueryClient({
		defaultOptions: {
			queries: {
				retry: false,
				retryDelay: 0,
				refetchOnWindowFocus: false,
				gcTime: Number.POSITIVE_INFINITY,
			},
		},
	});
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
	localStorage.clear();
});

afterEach(async () => {
	await act(async () => root.unmount());
	client.clear();
	container.remove();
	localStorage.clear();
	focusManager.setFocused(undefined);
	vi.useRealTimers();
});

async function settled(assertion: () => void) {
	let lastError: unknown;
	for (let attempt = 0; attempt < 100; attempt++) {
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 10));
		});
		try {
			assertion();
			return;
		} catch (error) {
			lastError = error;
		}
	}
	throw lastError;
}

async function renderPage() {
	await act(async () => {
		root.render(
			<QueryClientProvider client={client}>
				<HomePage />
			</QueryClientProvider>,
		);
	});
	await settled(() => {
		expect(fixture.editorProps).not.toBeNull();
		expect(fixture.getDefaults).toHaveBeenCalledOnce();
		expect(output("Inherited layout")).toBe("Published default");
	});
}

function button(label: string) {
	const element = Array.from(container.querySelectorAll("button")).find(
		(candidate) => candidate.textContent === label,
	);
	if (!element) throw new Error(`Missing button: ${label}`);
	return element;
}

function output(label: string) {
	return container.querySelector(`output[aria-label="${label}"]`)?.textContent;
}

async function returnToWindow() {
	await act(async () => {
		vi.setSystemTime(Date.now() + 31_000);
		focusManager.setFocused(false);
		focusManager.setFocused(true);
		await new Promise((resolve) => setTimeout(resolve, 20));
	});
}

describe("HomePage synchronized layout queries", () => {
	it("pulls remote personal and default changes on focus despite the app's disabled focus default", async () => {
		await renderPage();
		expect(output("Rendered layout")).toBe("Original personal home");
		fixture.profile.home_layout = layout("Changed on another device");
		defaults.main = {
			id: "main",
			revision: "r2",
			layout: layout("Updated published default"),
		};
		await returnToWindow();
		await settled(() => {
			expect(output("Rendered layout")).toBe("Changed on another device");
			expect(output("Inherited layout")).toBe("Updated published default");
		});
		expect(fixture.getProfile).toHaveBeenCalledTimes(2);
		expect(fixture.getDefaults).toHaveBeenCalledTimes(2);
	});

	it("pauses focus refresh while editing and resumes it after the draft closes", async () => {
		await renderPage();
		await act(async () => button("Edit").click());
		fixture.profile.home_layout = layout("Changed on another device");
		await returnToWindow();
		expect(fixture.getProfile).toHaveBeenCalledOnce();
		expect(fixture.getDefaults).toHaveBeenCalledOnce();
		expect(output("Rendered layout")).toBe("Original personal home");
		await act(async () => button("Discard").click());
		await returnToWindow();
		await settled(() =>
			expect(output("Rendered layout")).toBe("Changed on another device"),
		);
		expect(fixture.getProfile).toHaveBeenCalledTimes(2);
	});

	it("keeps an acknowledged personal save visible when a later profile read fails", async () => {
		await renderPage();
		await act(async () => button("Edit").click());
		fixture.getProfile.mockRejectedValue(new Error("Connection unavailable"));
		await act(async () => button("Save").click());
		await settled(() =>
			expect(output("Rendered layout")).toBe("Saved changes"),
		);
		expect(fixture.getProfile).toHaveBeenCalledOnce();
		await returnToWindow();
		await settled(() => {
			expect(fixture.getProfile).toHaveBeenCalledTimes(2);
			expect(output("Rendered layout")).toBe("Saved changes");
		});
		expect(fixture.saveLayout).toHaveBeenCalledWith(
			fixture.pendingLayout,
			"personal-profile",
		);
		expect(fixture.saveError).toBeUndefined();
		expect(fixture.editorProps?.layoutSource).toBe("personal");
	});

	it("saves a null reset and keeps the inherited layout when a later read fails", async () => {
		await renderPage();
		await act(async () => button("Edit").click());
		fixture.getProfile.mockRejectedValue(new Error("Connection unavailable"));
		await act(async () => button("Use default").click());
		await settled(() =>
			expect(output("Rendered layout")).toBe("Published default"),
		);
		expect(fixture.getProfile).toHaveBeenCalledOnce();
		await returnToWindow();
		await settled(() => {
			expect(fixture.getProfile).toHaveBeenCalledTimes(2);
			expect(output("Rendered layout")).toBe("Published default");
		});
		expect(fixture.saveLayout).toHaveBeenCalledWith(null, "personal-profile");
		expect(fixture.saveError).toBeUndefined();
		expect(fixture.editorProps?.layoutSource).toBe("main");
	});

	it("ignores stale reads after saving and accepts a newer remote edit", async () => {
		await renderPage();
		const stale = structuredClone(fixture.profile);
		await act(async () => button("Edit").click());
		await act(async () => button("Save").click());
		await settled(() =>
			expect(output("Rendered layout")).toBe("Saved changes"),
		);
		expect(fixture.getProfile).toHaveBeenCalledOnce();
		fixture.getProfile.mockResolvedValue(stale);
		await returnToWindow();
		await settled(() => expect(fixture.getProfile).toHaveBeenCalledTimes(2));
		expect(output("Rendered layout")).toBe("Saved changes");
		fixture.getProfile.mockResolvedValue({
			...stale,
			updated: "2026-09-07T10:02:00Z",
			home_layout: layout("Newer remote edit"),
		});
		await returnToWindow();
		await settled(() =>
			expect(output("Rendered layout")).toBe("Newer remote edit"),
		);
	});

	it("cancels a read already in flight when the save is acknowledged", async () => {
		await renderPage();
		const stale = structuredClone(fixture.profile);
		let finishRead!: (value: typeof stale) => void;
		fixture.getProfile.mockImplementationOnce(
			() =>
				new Promise((resolve) => {
					finishRead = resolve;
				}),
		);
		await returnToWindow();
		await settled(() => expect(fixture.getProfile).toHaveBeenCalledTimes(2));
		await act(async () => button("Edit").click());
		await act(async () => button("Save").click());
		await settled(() =>
			expect(output("Rendered layout")).toBe("Saved changes"),
		);
		await act(async () => finishRead(stale));
		expect(output("Rendered layout")).toBe("Saved changes");
		expect(fixture.saveError).toBeUndefined();
	});
});
