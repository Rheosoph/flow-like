// @vitest-environment happy-dom

import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { IBit } from "../lib/schema/bit/bit";

const mocks = vi.hoisted(() => ({
	addBit: vi.fn(),
	all: vi.fn(),
	getInfo: vi.fn(),
	getSettingsProfile: vi.fn(),
	signedIn: vi.fn(),
	invoke: vi.fn(),
}));

const ACTIVE = { hub_profile: { id: "active", bits: [] } };
const OTHER = { hub_profile: { id: "other", bits: [] } };

vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({ t: (key: string) => key }),
}));
vi.mock("@tanstack/react-query", () => ({
	useQueryClient: () => ({ invalidateQueries: vi.fn() }),
}));
vi.mock("sonner", () => ({ toast: { error: vi.fn() } }));
vi.mock("../state/backend-state", () => ({
	useSignedIn: mocks.signedIn,
	useBackend: () => ({
		bitState: { addBit: mocks.addBit, getBitSize: async () => 0 },
		userState: {
			getSettingsProfile: mocks.getSettingsProfile,
			getProfile: vi.fn(),
			getInfo: mocks.getInfo,
			getAllSettingsProfiles: mocks.all,
		},
	}),
}));
vi.mock("../state/download-manager", () => ({
	useDownloadManager: () => vi.fn(),
}));
vi.mock("../state/upgrade-dialog-state", () => ({
	handleUpgradeRequiredError: vi.fn(),
	openUpgradeDialogIfEnabled: vi.fn(),
}));
vi.mock("./use-hub", () => ({ useHub: () => ({ hub: undefined }) }));
vi.mock("./use-invoke", () => ({
	useInvalidateInvoke: () => async () => undefined,
	useInvoke: mocks.invoke,
}));

import { useAddModelToProfile } from "./use-add-model-to-profile";

const BIT = { id: "sonnet" } as IBit;
let root: Root;
let host: HTMLDivElement;
let add: ReturnType<typeof useAddModelToProfile>["add"];

function Probe({ profileId }: { profileId?: string }) {
	add = useAddModelToProfile(undefined, profileId).add;
	return null;
}

beforeEach(() => {
	(
		globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
	).IS_REACT_ACT_ENVIRONMENT = true;
	vi.clearAllMocks();
	mocks.signedIn.mockReturnValue(true);
	mocks.invoke.mockImplementation((fn) => ({
		data: fn === mocks.getSettingsProfile ? ACTIVE : undefined,
	}));
	mocks.all.mockResolvedValue([ACTIVE, OTHER]);
	host = document.createElement("div");
	root = createRoot(host);
});

afterEach(async () => {
	await act(async () => root.unmount());
});

async function mount(profileId?: string) {
	await act(async () => root.render(<Probe profileId={profileId} />));
}

test("without a target the model goes to the active profile", async () => {
	await mount();
	await act(async () => {
		await add(BIT);
	});
	expect(mocks.addBit).toHaveBeenCalledExactlyOnceWith(BIT, ACTIVE);
	expect(mocks.all).not.toHaveBeenCalled();
});

test("a bot's own profile gets the model, not the active one", async () => {
	await mount("other");
	await act(async () => {
		await add(BIT);
	});
	expect(mocks.addBit).toHaveBeenCalledExactlyOnceWith(BIT, OTHER);
});

test("signed-out profiles disable remote account info while allowing local model adds", async () => {
	mocks.signedIn.mockReturnValue(false);
	await mount();
	expect(mocks.invoke).toHaveBeenCalledWith(
		mocks.getInfo,
		expect.anything(),
		[],
		false,
		[],
	);
	expect(mocks.invoke).toHaveBeenCalledWith(
		mocks.getSettingsProfile,
		expect.anything(),
		[],
		true,
		[],
	);
	await act(async () => {
		expect(await add(BIT)).toBe(true);
	});
	expect(mocks.addBit).toHaveBeenCalledExactlyOnceWith(BIT, ACTIVE);
});

test("signing in enables account info for hosted model tier checks", async () => {
	mocks.signedIn.mockReturnValue(false);
	await mount();
	mocks.invoke.mockClear();
	mocks.signedIn.mockReturnValue(true);
	await mount();
	expect(mocks.invoke).toHaveBeenCalledWith(
		mocks.getInfo,
		expect.anything(),
		[],
		true,
		[],
	);
});
