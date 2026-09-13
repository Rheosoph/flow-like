import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
	NotificationIcon,
	NotificationIconArtwork,
} from "../notifications/notification-icon";

const fixture = vi.hoisted(() => ({
	profile: { id: "profile", hub: "hub.example" },
	viewer: "viewer",
	loading: false,
	getProfile: vi.fn(),
	getApps: vi.fn(),
}));
vi.mock("../../state/backend-state", () => ({
	useBackend: () => ({
		profile: fixture.profile,
		userState: { getProfile: fixture.getProfile },
		appState: { getApps: fixture.getApps },
	}),
	useBackendReady: () => true,
}));
vi.mock("react-oidc-context", () => ({
	useAuth: () => ({
		isLoading: fixture.loading,
		isAuthenticated: true,
		user: { profile: { sub: fixture.viewer } },
	}),
}));
vi.mock("lucide-react/dynamic", () => ({
	DynamicIcon: ({ name }: { name: string }) => <svg data-icon={name} />,
}));

let root: Root;
let container: HTMLDivElement;
let client: QueryClient;
beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	vi.useFakeTimers();
	fixture.profile = { id: "profile", hub: "hub.example" };
	fixture.viewer = "viewer";
	fixture.loading = false;
	fixture.getProfile
		.mockReset()
		.mockResolvedValue({ id: "profile", apps: [{ app_id: "app" }] });
	fixture.getApps.mockReset().mockResolvedValue([
		[{ id: "app" }, { icon: "/source-app.webp" }],
		[{ id: "outside-profile" }, { icon: "/hidden.webp" }],
	]);
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
async function render(content: React.ReactNode) {
	await act(async () =>
		root.render(
			<QueryClientProvider client={client}>{content}</QueryClientProvider>,
		),
	);
	await act(async () => vi.advanceTimersByTimeAsync(1));
}
const image = () => container.querySelector("img");

describe("shared notification icons", () => {
	it("falls back from a failed custom image through the app icon to the logo", async () => {
		await render(
			<NotificationIconArtwork icon="/custom.png" appIcon="/app.webp" />,
		);
		expect(image()?.getAttribute("src")).toBe("/custom.png");
		await act(async () => image()?.dispatchEvent(new Event("error")));
		expect(image()?.getAttribute("src")).toBe("/app.webp");
		await act(async () => image()?.dispatchEvent(new Event("error")));
		expect(image()?.getAttribute("src")).toBe("/app-logo.webp");
	});
	it("renders Lucide and emoji artwork without image requests", async () => {
		await render(<NotificationIconArtwork icon="shopping-bag" />);
		expect(container.querySelector("svg")?.getAttribute("data-icon")).toBe(
			"shopping-bag",
		);
		expect(image()).toBeNull();
		await render(<NotificationIconArtwork icon="🌍" />);
		expect(container.textContent).toBe("🌍");
	});
	it("shares one metadata query across rows and filters apps outside the profile", async () => {
		await render(
			<>
				<NotificationIcon appId="app" />
				<NotificationIcon appId="app" />
				<NotificationIcon appId="outside-profile" />
			</>,
		);
		expect(fixture.getApps).toHaveBeenCalledTimes(1);
		expect(fixture.getProfile).toHaveBeenCalledTimes(1);
		expect(
			[...container.querySelectorAll("img")].map((item) =>
				item.getAttribute("src"),
			),
		).toEqual(["/source-app.webp", "/source-app.webp", "/app-logo.webp"]);
	});
	it.each(["viewer", "profile", "hub"])(
		"does not display late artwork from the previous %s",
		async (change) => {
			let settle: (apps: unknown) => void = () => {};
			fixture.getApps.mockImplementationOnce(
				() =>
					new Promise((resolve) => {
						settle = resolve;
					}),
			);
			await render(<NotificationIcon appId="app" />);
			if (change === "viewer") fixture.viewer = "other";
			if (change === "profile")
				fixture.profile = { ...fixture.profile, id: "other" };
			if (change === "hub")
				fixture.profile = { ...fixture.profile, hub: "other.example" };
			fixture.getProfile.mockResolvedValue({
				id: fixture.profile.id,
				apps: [{ app_id: "app" }],
			});
			fixture.getApps.mockResolvedValue([
				[{ id: "app" }, { icon: "/new.webp" }],
			]);
			await render(<NotificationIcon appId="app" />);
			expect(image()?.getAttribute("src")).toBe("/new.webp");
			await act(async () => settle([[{ id: "app" }, { icon: "/old.webp" }]]));
			expect(image()?.getAttribute("src")).toBe("/new.webp");
		},
	);
	it("hides cached app artwork while authentication is changing", async () => {
		await render(<NotificationIcon appId="app" />);
		expect(image()?.getAttribute("src")).toBe("/source-app.webp");
		fixture.loading = true;
		await render(<NotificationIcon appId="app" />);
		expect(image()?.getAttribute("src")).toBe("/app-logo.webp");
	});
});
