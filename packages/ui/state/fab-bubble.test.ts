import { beforeEach, describe, expect, it } from "bun:test";
import { useFabBubbleStore } from "./fab-bubble";

const visible = () => {
	const { requests, suppressions } = useFabBubbleStore.getState();
	return requests > 0 && suppressions === 0;
};

describe("fab bubble visibility", () => {
	beforeEach(() => {
		useFabBubbleStore.setState({ requests: 0, suppressions: 0 });
	});

	it("stays hidden until a surface asks for it", () => {
		expect(visible()).toBe(false);
		const release = useFabBubbleStore.getState().acquireRequest();
		expect(visible()).toBe(true);
		release();
		expect(visible()).toBe(false);
	});

	it("keeps showing while any requester is still mounted", () => {
		const releaseBoard = useFabBubbleStore.getState().acquireRequest();
		const releaseBuilder = useFabBubbleStore.getState().acquireRequest();
		releaseBoard();
		expect(visible()).toBe(true);
		releaseBuilder();
		expect(visible()).toBe(false);
	});

	it("lets a suppressor outrank a request", () => {
		const releaseRequest = useFabBubbleStore.getState().acquireRequest();
		const releaseSuppression = useFabBubbleStore
			.getState()
			.acquireSuppression();
		expect(visible()).toBe(false);
		releaseSuppression();
		expect(visible()).toBe(true);
		releaseRequest();
	});

	it("never lets a double release drive a count negative", () => {
		const release = useFabBubbleStore.getState().acquireRequest();
		release();
		release();
		expect(useFabBubbleStore.getState().requests).toBe(0);
		useFabBubbleStore.getState().acquireRequest();
		expect(visible()).toBe(true);
	});
});
