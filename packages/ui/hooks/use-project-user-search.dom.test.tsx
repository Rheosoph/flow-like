import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
	IProjectContactsPage,
	IUserLookup,
} from "../state/backend-state/types";
import { useProjectUserSearch } from "./use-project-user-search";

const fixture = vi.hoisted(() => ({
	viewer: "viewer",
	getProjectContacts: vi.fn(),
	searchUsers: vi.fn(),
}));
vi.mock("../state/backend-state", () => ({
	useBackend: () => ({ userState: fixture }),
}));
vi.mock("../lib/api-url", () => ({ getApiOrigin: () => "https://hub.test" }));
vi.mock("react-oidc-context", () => ({
	useAuth: () => ({ user: { profile: { sub: fixture.viewer } } }),
}));

const user = (id: string, name: string): IUserLookup => ({
	id,
	name,
	created_at: "",
});
let latest: ReturnType<typeof useProjectUserSearch>;
let root: Root;
let container: HTMLDivElement;
let client: QueryClient;

/** `null` searches without a target project. */
function Search({
	query,
	open,
	appId,
}: { query: string; open: boolean; appId: string | null }) {
	latest = useProjectUserSearch(appId ?? undefined, query, open);
	return <output>{latest.results.map(({ user }) => user.id).join(",")}</output>;
}

async function render(
	query = "",
	open = true,
	appId: string | null = "target",
) {
	await act(async () =>
		root.render(
			<QueryClientProvider client={client}>
				<Search query={query} open={open} appId={appId} />
			</QueryClientProvider>,
		),
	);
}

async function flush(ms = 1) {
	await act(async () => vi.advanceTimersByTimeAsync(ms));
}

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	vi.useFakeTimers();
	fixture.viewer = "viewer";
	fixture.getProjectContacts.mockReset().mockResolvedValue({
		users: [user("colleague", "Alice Adams")],
		next_cursor: null,
	});
	fixture.searchUsers.mockReset().mockResolvedValue([]);
	client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	client.clear();
	container.remove();
	vi.useRealTimers();
});

describe("project invitation search", () => {
	it("searches cached colleagues immediately and debounces only the directory", async () => {
		await render();
		await flush();
		expect(container.textContent).toBe("colleague");
		await render("Ali");
		expect(container.textContent).toBe("colleague");
		expect(fixture.searchUsers).not.toHaveBeenCalled();
		await flush(250);
		expect(fixture.searchUsers).toHaveBeenCalledWith("Ali", "target");
		expect(fixture.getProjectContacts).toHaveBeenCalledTimes(1);
	});

	it("loads all contact pages without fetching one project at a time", async () => {
		fixture.getProjectContacts.mockImplementation(
			async (_app: string, after?: string) => ({
				users: [
					user(
						after ? "later" : "first",
						after ? "Zelda Adams" : "Alice Adams",
					),
				],
				next_cursor: after ? null : "first",
			}),
		);
		await render("Zelda");
		await flush();
		await flush();
		expect(fixture.getProjectContacts.mock.calls).toEqual([
			["target", undefined],
			["target", "first"],
		]);
		expect(container.textContent).toBe("later");
		await render("Alice");
		expect(container.textContent).toBe("first");
		expect(fixture.getProjectContacts).toHaveBeenCalledTimes(2);
	});

	it("hides old directory results immediately and ignores late responses", async () => {
		let resolveAlice!: (users: IUserLookup[]) => void;
		fixture.searchUsers.mockImplementation((query: string) =>
			query === "Alice"
				? new Promise((resolve) => {
						resolveAlice = resolve;
					})
				: Promise.resolve([user("bob", "Bob Brown")]),
		);
		await render("Alice");
		await flush(250);
		await render("Bob");
		expect(container.textContent).toBe("");
		await flush(250);
		await flush();
		expect(container.textContent).toBe("bob");
		await act(async () => resolveAlice([user("old", "Alice Smith")]));
		await flush();
		expect(container.textContent).toBe("bob");
		await render("");
		expect(container.textContent).toBe("colleague");
	});

	it("keeps local results usable when the directory fails", async () => {
		fixture.searchUsers.mockRejectedValue(new Error("offline"));
		await render("Alice");
		await flush(250);
		await flush(1100);
		await flush();
		expect(latest.directoryError).toBeTruthy();
		expect(container.textContent).toBe("colleague");
	});

	it("stops paging while closed and resumes on reopen", async () => {
		let resolveFirst!: (page: IProjectContactsPage) => void;
		fixture.getProjectContacts.mockImplementationOnce(
			() =>
				new Promise((resolve) => {
					resolveFirst = resolve;
				}),
		);
		await render();
		await render("", false);
		await act(async () =>
			resolveFirst({
				users: [user("first", "Alice Adams")],
				next_cursor: "first",
			}),
		);
		await flush();
		expect(fixture.getProjectContacts).toHaveBeenCalledTimes(1);
		await render();
		await flush();
		expect(fixture.getProjectContacts).toHaveBeenCalledTimes(2);
	});

	it("retains earlier pages and retries a failed page on request", async () => {
		fixture.getProjectContacts.mockImplementation(
			async (_app: string, after?: string) => {
				if (after) throw new Error("offline");
				return { users: [user("first", "Alice Adams")], next_cursor: "first" };
			},
		);
		await render("Alice");
		await flush();
		await flush(1100);
		await flush();
		expect(latest.contactsError).toBeTruthy();
		expect(container.textContent).toBe("first");
		const attempts = fixture.getProjectContacts.mock.calls.length;
		await flush(5000);
		expect(fixture.getProjectContacts).toHaveBeenCalledTimes(attempts);
		fixture.getProjectContacts.mockResolvedValue({
			users: [user("second", "Alice Brown")],
			next_cursor: null,
		});
		await act(async () => {
			await latest.retryContacts();
		});
		await flush();
		expect(container.textContent).toBe("first,second");
		expect(latest.contactsError).toBeNull();
	});

	it("removes a sent invite from cached results before reopening", async () => {
		await render();
		await flush();
		await render("", false);
		await act(async () => {
			await latest.invalidate("colleague");
		});
		fixture.getProjectContacts.mockReturnValue(new Promise(() => {}));
		await render();
		expect(container.textContent).toBe("");
	});

	it("searches only the directory when there is no target project", async () => {
		fixture.searchUsers.mockResolvedValue([user("anyone", "Alice Adams")]);
		await render("", true, null);
		await flush();
		expect(container.textContent).toBe("");
		expect(latest.isLoadingContacts).toBe(false);
		await render("Ali", true, null);
		await flush(250);
		await flush();
		expect(fixture.searchUsers).toHaveBeenCalledWith("Ali", undefined);
		expect(container.textContent).toBe("anyone");
		expect(fixture.getProjectContacts).not.toHaveBeenCalled();
	});

	it("isolates cached users by account and target project", async () => {
		await render();
		await flush();
		expect(container.textContent).toBe("colleague");
		fixture.getProjectContacts.mockReturnValue(new Promise(() => {}));
		fixture.viewer = "other-viewer";
		await render();
		expect(container.textContent).toBe("");
		fixture.viewer = "viewer";
		await render("", true, "other-project");
		expect(container.textContent).toBe("");
	});
});
