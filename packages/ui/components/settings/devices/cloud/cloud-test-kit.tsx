import type { ReactNode } from "react";
import {
	SAMPLE_ME,
	SAMPLE_PEOPLE,
} from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import type { IBackendState } from "../../../../state/backend-state";
import type { IUserLookup } from "../../../../state/backend-state/types";
import type { DeviceSeed } from "../testing/fake-device-api";
import type { FakeWorkspace } from "../testing/fake-workspace";
import type {
	MountDevicesOptions,
	MountedDevices,
} from "../testing/mount-devices";

/*
 * Test support for the cloud screens: the fake workspace plus an account
 * directory, a model catalogue and app roles. Import after `installDom()`.
 */

export const PEOPLE: Readonly<Record<string, string>> = {
	[SAMPLE_ME]: "Felix Schultz",
	[SAMPLE_PEOPLE.mira]: "Mira Novak",
	[SAMPLE_PEOPLE.jonas]: "Jonas Weber",
	[SAMPLE_PEOPLE.partner]: "Partner Org Admin",
};

export const MODELS: Readonly<Record<string, string>> = {
	"bge-m3": "BGE-M3 embeddings",
	"gpt-4.1-mini": "GPT-4.1 mini",
	"mistral-small-3": "Mistral Small 3",
};

/** No snake_case wire value, gate code or pre-flight code may reach the screen (R3). */
export const MACHINE_WORDS = /\b[a-z]+_[a-z_]+\b|\bG\d{1,2}\b|\bD\d\b/;

/** Owner with every permission, as `GET /apps/{id}/roles/me` answers. */
export const OWNER_ROLE = {
	role_id: "role-owner",
	role_name: "Owner",
	permissions: 1,
	is_owner: true,
	can_leave: false,
};
/** An admin who doesn't own the app: may approve models, not project files. The hub answers `is_owner: true` for an Admin too. */
export const ADMIN_ROLE = {
	role_id: "role-admin",
	role_name: "Admin",
	permissions: 2,
	is_owner: true,
	can_leave: true,
};
/** A member who may read boards only. */
export const MEMBER_ROLE = {
	role_id: "role-member",
	role_name: "Member",
	permissions: 256,
	is_owner: false,
	can_leave: true,
};

export interface MountCloudOptions extends MountDevicesOptions {
	people?: Readonly<Record<string, string>>;
	models?: Readonly<Record<string, string>>;
	/** App id → model ids the app uses. */
	appModels?: Readonly<Record<string, readonly string[]>>;
	/** App id → the viewer's role; apps without an entry have no readable role. */
	roles?: Readonly<Record<string, typeof OWNER_ROLE>>;
}

function directory(
	fake: FakeWorkspace,
	people: Readonly<Record<string, string>>,
): IBackendState["userState"] {
	const lookup = (id: string): IUserLookup | undefined =>
		people[id]
			? { id, name: people[id], created_at: "2026-01-01T00:00:00Z" }
			: undefined;
	return {
		getProfile: async () => fake.profile,
		getInfo: async () => ({ id: fake.hub.me, dev_mode: false }),
		updateUser: async () => undefined,
		lookupUser: async (id: string) => {
			const user = lookup(id);
			if (!user) throw new Error("No such account");
			return user;
		},
		lookupUsers: async (ids: string[]) => ids.flatMap((id) => lookup(id) ?? []),
	} as unknown as IBackendState["userState"];
}

function catalogue(
	models: Readonly<Record<string, string>>,
): IBackendState["bitState"] {
	return {
		getBit: async (id: string) => {
			const name = models[id];
			if (!name) throw new Error("No such model");
			return {
				id,
				type: id.startsWith("bge") ? "Embedding" : "Llm",
				meta: { en: { name } },
			};
		},
	} as unknown as IBackendState["bitState"];
}

function roleState(
	roles: Readonly<Record<string, typeof OWNER_ROLE>>,
): IBackendState["roleState"] {
	return {
		getOwnRole: async (appId: string) => {
			const role = roles[appId];
			if (!role) throw new Error("No role");
			return role;
		},
	} as unknown as IBackendState["roleState"];
}

/** `mountDevices` with names for people and models; `seed` and the fake options pass through. */
export async function mountCloud(
	node: ReactNode,
	options: MountCloudOptions = {},
	seed?: DeviceSeed,
): Promise<MountedDevices> {
	const [{ mountDevices, fakeBackend }, { createFakeWorkspace }] =
		await Promise.all([
			import("../testing/mount-devices"),
			import("../testing/fake-workspace"),
		]);
	const fake =
		options.fake ?? (await createFakeWorkspace(seed ?? options.seed, options));
	const base = fakeBackend(fake, options.apps ?? fake.hub.apps);
	const appModels = options.appModels ?? {};
	return mountDevices(node, {
		...options,
		fake,
		backend: {
			userState: directory(fake, options.people ?? PEOPLE),
			bitState: catalogue(options.models ?? MODELS),
			roleState: roleState(options.roles ?? {}),
			appState: {
				getApps: () => base.appState.getApps(),
				getAppMeta: (appId: string) => base.appState.getAppMeta(appId),
				getApp: async (appId: string) => ({
					...(await base.appState.getApp(appId)),
					bits: [...(appModels[appId] ?? [])],
				}),
			} as unknown as IBackendState["appState"],
			...options.backend,
		},
	});
}

/** Hub requests that change an approval or a spending limit. */
export const cloudWrites = (fake: FakeWorkspace) =>
	fake.api
		.writes()
		.filter(([, path]) => /resource-grants|billing-grants/.test(path));

export const textOf = (root: ParentNode = document.body) =>
	(root.textContent ?? "").replace(/\s+/g, " ");

/** The copy a person reads: everything but the settings reference, which is wire text by design. */
export function copyOf(root: HTMLElement): string {
	const copy = root.cloneNode(true) as HTMLElement;
	for (const wire of copy.querySelectorAll("pre")) wire.remove();
	return textOf(copy);
}

export const primaries = (root: ParentNode) =>
	root.querySelectorAll("[data-dv-primary]").length;
