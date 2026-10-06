import { describe, expect, test } from "bun:test";
import {
	type RegisteredController,
	attachSessionController,
	claimSessionController,
	detachSessionController,
	getSessionController,
} from "./registry";

class FakeController implements RegisteredController {
	attached = 0;
	disposed = false;
	readonly listeners = new Set<() => void>();
	constructor(readonly name: string) {}
	attach() {
		this.attached += 1;
	}
	detach() {
		this.attached -= 1;
	}
	isAttached() {
		return this.attached > 0;
	}
	isDisposed() {
		return this.disposed;
	}
	onDispose(listener: () => void) {
		this.listeners.add(listener);
		return () => this.listeners.delete(listener);
	}
	dispose() {
		this.disposed = true;
		for (const listener of this.listeners) listener();
	}
}

function factoryOf() {
	const made: FakeController[] = [];
	const factory = () => {
		const controller = new FakeController(`controller-${made.length + 1}`);
		made.push(controller);
		return controller;
	};
	return { made, factory };
}

describe("claimSessionController", () => {
	test("the first mount claims the shared controller without attaching it", () => {
		const engine = {};
		const { made, factory } = factoryOf();
		const shared = getSessionController(engine, "app", "event", factory);
		const claimed = claimSessionController(
			engine,
			"app",
			"event",
			shared,
			factory,
		);
		expect(claimed).toBe(shared);
		expect(shared.isAttached()).toBe(false);
		expect(made).toHaveLength(1);
	});

	test("a second mount while the shared one is held claims one of its own; the shared one is left alone", () => {
		const engine = {};
		const { factory } = factoryOf();
		const shared = getSessionController(engine, "app", "event", factory, "p");
		attachSessionController(engine, "app", "event", shared, factory, "p");
		const second = claimSessionController(
			engine,
			"app",
			"event",
			shared,
			factory,
			"p",
		);
		expect(second).not.toBe(shared);
		expect(second.isAttached()).toBe(false);
		expect(shared.attached).toBe(1);
	});

	test("a mount that has its own keeps it; a disposed one falls back to the shared controller", () => {
		const engine = {};
		const { factory } = factoryOf();
		const shared = getSessionController(engine, "app", "event", factory);
		attachSessionController(engine, "app", "event", shared, factory);
		const own = attachSessionController(
			engine,
			"app",
			"event",
			shared,
			factory,
		);
		expect(claimSessionController(engine, "app", "event", own, factory)).toBe(
			own,
		);
		detachSessionController(shared);
		own.dispose();
		expect(claimSessionController(engine, "app", "event", own, factory)).toBe(
			shared,
		);
	});
});
