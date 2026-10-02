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
 * Test support for the Access screens: the fake workspace plus an account
 * directory, so people render by name. Import after `installDom()`.
 */

export const PEOPLE: Readonly<Record<string, string>> = {
	[SAMPLE_ME]: "Felix Schultz",
	[SAMPLE_PEOPLE.mira]: "Mira Novak",
	[SAMPLE_PEOPLE.jonas]: "Jonas Weber",
	[SAMPLE_PEOPLE.partner]: "Partner Org Admin",
};

/** No snake_case wire value, gate code or pre-flight code may reach the screen (R3). */
export const MACHINE_WORDS = /\b[a-z]+_[a-z_]+\b|\bG\d{1,2}\b|\bD\d\b/;

export interface MountAccessOptions extends MountDevicesOptions {
	/** Account id → display name; the sample people by default. */
	people?: Readonly<Record<string, string>>;
}

function userState(
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

/** `mountDevices` with names for the sample people; `seed` and the fake options pass through. */
export async function mountAccess(
	node: ReactNode,
	options: MountAccessOptions = {},
	seed?: DeviceSeed,
): Promise<MountedDevices> {
	const [{ mountDevices }, { createFakeWorkspace }] = await Promise.all([
		import("../testing/mount-devices"),
		import("../testing/fake-workspace"),
	]);
	const fake =
		options.fake ?? (await createFakeWorkspace(seed ?? options.seed, options));
	return mountDevices(node, {
		...options,
		fake,
		backend: {
			userState: userState(fake, options.people ?? PEOPLE),
			...options.backend,
		},
	});
}

/** A request file as a person's app saves it for one device. */
export function requestFile(
	deviceId: string,
	userId: string,
	keySeed: string,
	grantId?: string,
): File {
	return new File(
		[
			JSON.stringify([
				{
					user_id: userId,
					controller_key: {
						kty: "OKP",
						crv: "Ed25519",
						x: keySeed.padEnd(43, "A").slice(0, 43),
					},
					...(grantId ? { grant_id: grantId } : {}),
				},
			]),
		],
		`device-access-${deviceId}.json`,
		{ type: "application/json" },
	);
}
