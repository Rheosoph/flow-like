import { describe, expect, test } from "bun:test";
import type { IUserLookup } from "../state/backend-state/types";
import { __testing } from "./use-user-lookup";

const { UserLookupBatcher } = __testing;

const SUB = "42c52474-5081-70d7-2b23-4bd8c38d8fb0";
const OTHER_SUB = "32a5a414-a001-70d1-7b23-570b1c9d4e2f";

function account(id: string, name: string): IUserLookup {
	return { id, name, created_at: "" };
}

/** Records what each lookup was asked for, so coalescing can be asserted. */
function source(
	overrides: Partial<{
		users: (userIds: string[]) => Promise<IUserLookup[]>;
		user: (userId: string) => Promise<IUserLookup>;
	}> = {},
) {
	const batches: string[][] = [];
	const singles: string[] = [];

	return {
		batches,
		singles,
		lookupUsers: async (userIds: string[]) => {
			batches.push(userIds);
			if (overrides.users) return overrides.users(userIds);
			return userIds.map((id) => account(id, `Name ${id.slice(0, 4)}`));
		},
		lookupUser: async (userId: string) => {
			singles.push(userId);
			if (overrides.user) return overrides.user(userId);
			return account("signed-in-sub", "You");
		},
	};
}

describe("UserLookupBatcher", () => {
	test("collapses ids queued together into one request", async () => {
		const backend = source();
		const batcher = new UserLookupBatcher(backend);

		const [first, second, repeat] = await Promise.all([
			batcher.load(SUB),
			batcher.load(OTHER_SUB),
			batcher.load(SUB),
		]);

		expect(backend.batches).toEqual([[SUB, OTHER_SUB]]);
		expect(first?.id).toBe(SUB);
		expect(second?.id).toBe(OTHER_SUB);
		expect(repeat).toBe(first);
	});

	test("resolves an id the directory omits to null rather than throwing", async () => {
		const backend = source({ users: async () => [account(SUB, "Felix")] });
		const batcher = new UserLookupBatcher(backend);

		const [found, missing] = await Promise.all([
			batcher.load(SUB),
			batcher.load(OTHER_SUB),
		]);

		expect(found?.name).toBe("Felix");
		expect(missing).toBeNull();
	});

	test("hands the failure to every waiter when the request itself fails", async () => {
		const backend = source({
			users: async () => {
				throw new Error("offline");
			},
		});
		const batcher = new UserLookupBatcher(backend);

		const [outcome] = await Promise.allSettled([batcher.load(SUB)]);
		expect(outcome.status).toBe("rejected");
		expect((outcome as PromiseRejectedResult).reason).toMatchObject({
			message: "offline",
		});
	});

	test("resolves the local placeholder on its own, never through the batch", async () => {
		const backend = source();
		const batcher = new UserLookupBatcher(backend);

		const [local, sub] = await Promise.all([
			batcher.load("local"),
			batcher.load(SUB),
		]);

		expect(backend.singles).toEqual(["local"]);
		expect(backend.batches).toEqual([[SUB]]);
		expect(local?.name).toBe("You");
		expect(sub?.id).toBe(SUB);
	});

	test("a failing local lookup does not take the batch down with it", async () => {
		const backend = source({
			user: async () => {
				throw new Error("no session");
			},
		});
		const batcher = new UserLookupBatcher(backend);

		const results = await Promise.allSettled([
			batcher.load("local"),
			batcher.load(SUB),
		]);

		expect(results[0].status).toBe("rejected");
		expect(results[1]).toMatchObject({
			status: "fulfilled",
			value: { id: SUB },
		});
	});

	test("starts a fresh window once the previous one has been sent", async () => {
		const backend = source();
		const batcher = new UserLookupBatcher(backend);

		await batcher.load(SUB);
		await batcher.load(OTHER_SUB);

		expect(backend.batches).toEqual([[SUB], [OTHER_SUB]]);
	});
});
