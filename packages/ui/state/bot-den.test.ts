import { beforeEach, describe, expect, it } from "bun:test";
import { useBotDenStore } from "./bot-den";

const latest = () => useBotDenStore.getState().requests.at(-1) ?? null;

describe("bot den requests", () => {
	beforeEach(() => {
		useBotDenStore.setState({ requests: [] });
	});

	it("stays hidden until a surface asks for it", () => {
		expect(latest()).toBeNull();
		const release = useBotDenStore.getState().acquire(null);
		expect(latest()).not.toBeNull();
		release();
		expect(latest()).toBeNull();
	});

	it("follows the newest surface and falls back when it leaves", () => {
		const board = { current: null };
		const builder = { current: null };
		const releaseBoard = useBotDenStore.getState().acquire(board);
		const releaseBuilder = useBotDenStore.getState().acquire(builder);
		expect(latest()?.anchor).toBe(builder);
		releaseBuilder();
		expect(latest()?.anchor).toBe(board);
		releaseBoard();
		expect(latest()).toBeNull();
	});

	it("ignores a second release", () => {
		const release = useBotDenStore.getState().acquire(null);
		const other = useBotDenStore.getState().acquire(null);
		release();
		release();
		expect(useBotDenStore.getState().requests).toHaveLength(1);
		other();
		expect(latest()).toBeNull();
	});
});
