// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";

const state = vi.hoisted(() => ({
	getTemplate: vi.fn(),
	getMetadata: vi.fn(),
	setQueryParams: vi.fn(),
	canRead: true,
}));

vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));
vi.mock("../../../hooks", async () => import("../../../hooks/use-invoke"));
vi.mock("../../../hooks/use-app-permissions", () => ({
	useAppPermissions: () => ({ can: () => state.canRead, roleName: "Reader" }),
}));
vi.mock("../../../lib", () => ({
	useSetQueryParams: () => state.setQueryParams,
	formatRelativeTime: () => "Today",
}));
vi.mock("../../../state/backend-state", () => ({
	useBackend: () => ({
		templateState: {
			getTemplate: function getTemplate() {
				return state.getTemplate();
			},
			getTemplateMeta: function getTemplateMeta() {
				return state.getMetadata();
			},
		},
		boardState: {
			getBoardSummaries: async function getBoardSummaries() {
				return [];
			},
			getBoardVersions: async function getBoardVersions() {
				return [];
			},
		},
	}),
}));
vi.mock("../../flow", () => ({
	FlowPreview: () => <div data-preview="">Workflow preview</div>,
}));
vi.mock("../../ui", async () => ({
	...(await import("../../ui/breadcrumb")),
	TextEditor: ({ initialContent }: { initialContent: string }) => (
		<div>{initialContent}</div>
	),
}));
vi.mock("../permission/permission-gate", () => ({
	SectionLockedPanel: () => <div>Templates are locked</div>,
}));
vi.mock("../permission/permission-notice", () => ({
	PermissionNotice: () => <div>Read-only</div>,
}));

import { TemplatePreview } from "./template-preview";

const template = { nodes: {}, version: [1, 0, 0], stage: "Published" };
const metadata = {
	name: "Daily report",
	description: "Review the latest activity.",
	long_description: "Report details",
	tags: [],
};

function deferred<T>() {
	let resolve!: (value: T) => void;
	const promise = new Promise<T>((done) => {
		resolve = done;
	});
	return { promise, resolve };
}

let client: QueryClient;
let host: HTMLDivElement;
let root: Root;

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	state.getTemplate.mockReset();
	state.getMetadata.mockReset();
	state.setQueryParams.mockReset();
	state.canRead = true;
	client = new QueryClient({
		defaultOptions: { queries: { retry: false, gcTime: 0 } },
	});
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
});

afterEach(async () => {
	await act(async () => root.unmount());
	client.clear();
	host.remove();
	vi.restoreAllMocks();
});

async function settle() {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
}

async function mount(canEdit = false) {
	await act(async () =>
		root.render(
			<QueryClientProvider client={client}>
				<TemplatePreview appId="app" templateId="template" canEdit={canEdit} />
			</QueryClientProvider>,
		),
	);
	await settle();
}

test.each(["preview", "details"])(
	"the %s can arrive first while navigation and the other pane stay visible",
	async (first) => {
		const preview = deferred<typeof template>();
		const details = deferred<typeof metadata>();
		state.getTemplate.mockReturnValue(preview.promise);
		state.getMetadata.mockReturnValue(details.promise);
		await mount();
		expect(host.querySelectorAll('[aria-busy="true"]')).toHaveLength(2);
		const back = host.querySelector("nav button") as HTMLButtonElement;
		expect(back.textContent).toBe("Templates");
		await act(async () => back.click());
		expect(state.setQueryParams).toHaveBeenCalledWith("templateId", undefined);

		await act(async () => {
			if (first === "preview") preview.resolve(template);
			else details.resolve(metadata);
		});
		await settle();
		expect(host.querySelectorAll('[aria-busy="true"]')).toHaveLength(1);
		expect(Boolean(host.querySelector("[data-preview]"))).toBe(
			first === "preview",
		);
		expect(Boolean(host.querySelector("h1"))).toBe(first === "details");
		expect(host.querySelector("nav button")).toBe(back);

		await act(async () => {
			preview.resolve(template);
			details.resolve(metadata);
		});
		await settle();
		expect(host.querySelector("h1")?.textContent).toBe(metadata.name);
		expect(host.querySelector("[data-preview]")).not.toBeNull();
		expect(host.querySelector('[aria-busy="true"]')).toBeNull();
	},
);

test.each(["preview", "details", "empty"])(
	"a %s failure offers a retry without hiding the other pane",
	async (failure) => {
		vi.spyOn(console, "error").mockImplementation(() => {});
		state.getTemplate.mockResolvedValue(template);
		state.getMetadata.mockResolvedValue(metadata);
		if (failure === "preview") {
			state.getTemplate.mockRejectedValueOnce(new Error("Unavailable"));
		} else if (failure === "details") {
			state.getMetadata.mockRejectedValueOnce(new Error("Unavailable"));
		} else {
			state.getMetadata.mockResolvedValueOnce(null);
		}
		await mount();
		await settle();
		expect(host.querySelector("[role=alert]")?.textContent).toContain(
			failure === "preview"
				? "Could not load the workflow preview."
				: "Could not load the template details.",
		);
		expect(host.querySelector('[aria-busy="true"]')).toBeNull();
		expect(host.querySelector("nav button")).not.toBeNull();
		if (failure === "preview") {
			expect(host.querySelector("h1")?.textContent).toBe(metadata.name);
		} else {
			expect(host.querySelector("[data-preview]")).not.toBeNull();
		}
		await act(async () => {
			(host.querySelector("[role=alert] button") as HTMLButtonElement).click();
		});
		await settle();
		expect(host.querySelector("[role=alert]")).toBeNull();
		expect(host.querySelector("h1")?.textContent).toBe(metadata.name);
		expect(host.querySelector("[data-preview]")).not.toBeNull();
	},
);

test("cached content stays mounted throughout a background refetch", async () => {
	state.getTemplate.mockResolvedValue(template);
	state.getMetadata.mockResolvedValue(metadata);
	await mount();
	const heading = host.querySelector("h1");
	const preview = host.querySelector("[data-preview]");
	const refresh = deferred<typeof metadata>();
	state.getMetadata.mockReturnValue(refresh.promise);
	let refetch!: Promise<unknown>;
	await act(async () => {
		refetch = client.refetchQueries({ queryKey: ["getTemplateMeta"] });
	});
	expect(host.querySelector("h1")).toBe(heading);
	expect(host.querySelector("[data-preview]")).toBe(preview);
	expect(host.querySelector('[aria-busy="true"]')).toBeNull();
	await act(async () => {
		refresh.resolve({ ...metadata, name: "Updated report" });
		await refetch;
	});
	await settle();
	expect(host.querySelector("h1")).toBe(heading);
	expect(heading?.textContent).toBe("Updated report");
});

test("loaded metadata remains editable while the workflow preview is pending", async () => {
	state.getTemplate.mockReturnValue(deferred<typeof template>().promise);
	state.getMetadata.mockResolvedValue(metadata);
	await mount(true);
	const edit = Array.from(host.querySelectorAll("button")).find(
		(button) => button.textContent === "Edit",
	);
	expect(edit?.disabled).toBe(false);
	await act(async () => edit?.click());
	expect(host.querySelector("input")?.value).toBe(metadata.name);
	expect(host.querySelector("textarea")?.value).toBe(metadata.description);
	expect(host.querySelector("nav button")).not.toBeNull();
});

test("denied template access stays locked without starting reads", async () => {
	state.canRead = false;
	await mount();
	expect(host.textContent).toBe("Templates are locked");
	expect(state.getTemplate).not.toHaveBeenCalled();
	expect(state.getMetadata).not.toHaveBeenCalled();
	expect(host.querySelector('[aria-busy="true"]')).toBeNull();
});
