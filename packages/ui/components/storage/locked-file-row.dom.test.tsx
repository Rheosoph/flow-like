import { type ComponentProps, type ReactNode, act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
	ILockedItem,
	ILockedItemDetail,
	ILockedItemVersion,
} from "../../state/backend-state/locked-storage-state";
import { LockedFileRow } from "./locked-file-row";
import { LockedVersionList } from "./locked-version-list";

const fixture = vi.hoisted(() => ({
	detail: undefined as ILockedItemDetail | undefined,
	invokeArgs: [] as unknown[][],
}));

vi.mock("../..", () => ({
	useInvoke: (_fn: unknown, _context: unknown, args: unknown[]) => {
		fixture.invokeArgs.push(args);
		return { data: fixture.detail, isError: false };
	},
}));

vi.mock("../ui", () => {
	const Wrapper = ({ children }: { children?: ReactNode }) => <>{children}</>;
	return {
		Badge: Wrapper,
		Button: ({ children, ...props }: ComponentProps<"button">) => (
			<button type="button" {...props}>
				{children}
			</button>
		),
		DropdownMenu: Wrapper,
		DropdownMenuContent: () => null,
		DropdownMenuItem: Wrapper,
		DropdownMenuLabel: Wrapper,
		DropdownMenuSeparator: () => null,
		DropdownMenuTrigger: Wrapper,
		Skeleton: () => <div data-skeleton />,
		UserInlineTag: ({ userId }: { userId: string }) => (
			<span data-user={userId} />
		),
	};
});

const version = (
	number: number,
	status: ILockedItemVersion["status"],
): ILockedItemVersion => ({
	id: `version-${number}`,
	item_id: "item-1",
	version: number,
	status,
	file_name: "Übersicht (2)#1.pdf",
	content_type: "application/pdf",
	size: 1024 * number,
	sha256: "ab".repeat(32),
	checksum_verified: true,
	provider_version_id: `p-${number}`,
	retention_mode: "compliance",
	retain_until: "2036-10-21T12:00:00Z",
	enforcement: "object",
	legal_hold: number === 2,
	delete_after_retention: false,
	comment: null,
	metadata: null,
	uploaded_by: "user-1",
	created_at: "2026-10-21T12:00:00Z",
	locked_at: "2026-10-21T12:00:05Z",
	purged_at: status === "purged" ? "2036-10-22T12:00:00Z" : null,
	purged_by: status === "purged" ? "admin-1" : null,
	purgeable: false,
});

const versions = [
	version(3, "purged"),
	version(2, "locked"),
	version(1, "locked"),
];

const item: ILockedItem = {
	id: "item-1",
	app_id: "app-a",
	scope: "project",
	owner_id: null,
	name: "Übersicht (2)#1.pdf",
	folder: "Akten/Übersicht",
	content_type: "application/pdf",
	latest_version: 3,
	latest_size: 2048,
	live_versions: 2,
	live_size: 3072,
	metadata: null,
	created_by: "user-1",
	created_at: "2026-10-21T12:00:00Z",
	updated_at: "2026-10-21T12:00:00Z",
	latest: versions[1] ?? null,
};

let root: Root;
let container: HTMLDivElement;

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	fixture.detail = { item, versions };
	fixture.invokeArgs = [];
	container = document.createElement("div");
	document.body.appendChild(container);
	root = createRoot(container);
});

afterEach(() => {
	act(() => root.unmount());
	container.remove();
});

function renderRow(onDownload = vi.fn(), onOpenDetails = vi.fn()) {
	act(() =>
		root.render(
			<LockedFileRow
				item={item}
				highlight={false}
				actions={{
					onOpen: vi.fn(),
					onDownload: vi.fn(),
					onShowVersions: vi.fn(),
					renderVersions: (target) => (
						<LockedVersionList
							appId="app-a"
							item={target}
							queries={{ lockedItemDetail: vi.fn() }}
							onDownload={onDownload}
							onOpenDetails={onOpenDetails}
						/>
					),
				}}
			/>,
		),
	);
	return { onDownload, onOpenDetails };
}

const toggle = () =>
	container.querySelector<HTMLButtonElement>("button[aria-expanded]");

describe("Locked file row", () => {
	it("badges the newest stored version, not a purged higher number", () => {
		renderRow();
		expect(container.textContent).toContain("v2");
		expect(container.textContent).not.toContain("v3");
	});

	it("expands every version inline and downloads the one picked", () => {
		const { onDownload, onOpenDetails } = renderRow();
		expect(container.querySelector("[data-locked-version]")).toBeNull();
		expect(fixture.invokeArgs).toEqual([]);

		act(() => toggle()?.click());

		expect(toggle()?.getAttribute("aria-expanded")).toBe("true");
		expect(fixture.invokeArgs.at(-1)).toEqual(["app-a", "item-1"]);
		const rows = [...container.querySelectorAll("[data-locked-version]")];
		expect(rows.map((row) => row.getAttribute("data-locked-version"))).toEqual([
			"version-3",
			"version-2",
			"version-1",
		]);
		expect(rows[0]?.querySelector("button")).toBeNull();

		act(() => rows[2]?.querySelector("button")?.click());
		expect(onDownload).toHaveBeenCalledWith(item, 1);

		const details = [...container.querySelectorAll("button")].find((button) =>
			button.textContent?.includes("Checksums and retention details"),
		);
		act(() => details?.click());
		expect(onOpenDetails).toHaveBeenCalledWith("item-1");

		act(() => toggle()?.click());
		expect(container.querySelector("[data-locked-version]")).toBeNull();
	});
});
