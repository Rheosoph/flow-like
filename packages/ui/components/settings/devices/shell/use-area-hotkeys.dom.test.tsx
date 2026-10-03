import { afterAll, afterEach, expect, test } from "bun:test";
import { fire, installDom } from "../testing/dom-harness";

const dom = installDom();
const { isFilterHotkey, useAreaHotkeys } = await import("./use-area-hotkeys");

afterEach(dom.cleanup);
afterAll(dom.restore);

function Area({
	onFilter,
	enabled,
}: Readonly<{ onFilter(): boolean; enabled?: boolean }>) {
	useAreaHotkeys({ onFilter, enabled });
	return (
		<div>
			<button type="button" data-testid="plain">
				Refresh
			</button>
			<input aria-label="Device name" />
			<textarea aria-label="Reason" />
			{/* biome-ignore lint/a11y/useSemanticElements: the markup Radix renders for a sheet */}
			<div role="dialog" data-testid="sheet">
				<button type="button">Cancel</button>
			</div>
			{/* biome-ignore lint/a11y/useSemanticElements: the markup Radix renders for a sheet */}
			<div role="dialog" data-testid="rail">
				<input data-rail-filter="" aria-label="Filter devices" />
				<a href="/settings/devices">Fleet overview</a>
			</div>
		</div>
	);
}

interface Modifiers {
	metaKey?: boolean;
	ctrlKey?: boolean;
	altKey?: boolean;
}

function slash(init: Modifiers = {}): KeyboardEvent {
	return new dom.window.KeyboardEvent("keydown", {
		key: "/",
		bubbles: true,
		cancelable: true,
		...init,
	}) as unknown as KeyboardEvent;
}

async function press(selector: string, init?: Modifiers) {
	const target = dom.document.querySelector(selector) as HTMLElement;
	const event = slash(init);
	await fire(target, event);
	return event;
}

test("`/` outside a field asks for the filter and is consumed when it was taken", async () => {
	let calls = 0;
	await dom.render(
		<Area
			onFilter={() => {
				calls++;
				return true;
			}}
		/>,
	);
	const event = await press('[data-testid="plain"]');
	expect(calls).toBe(1);
	expect(event.defaultPrevented).toBe(true);
});

test("the key is left alone when no filter took it", async () => {
	await dom.render(<Area onFilter={() => false} />);
	const event = await press('[data-testid="plain"]');
	expect(event.defaultPrevented).toBe(false);
});

test("typing a slash in a field never moves the focus", async () => {
	let calls = 0;
	await dom.render(
		<Area
			onFilter={() => {
				calls++;
				return true;
			}}
		/>,
	);
	await press('input[aria-label="Device name"]');
	await press("textarea");
	await press("[data-rail-filter]");
	expect(calls).toBe(0);
});

test("shortcuts with a modifier and keys inside a sheet are not taken", async () => {
	let calls = 0;
	await dom.render(
		<Area
			onFilter={() => {
				calls++;
				return true;
			}}
		/>,
	);
	await press('[data-testid="plain"]', { metaKey: true });
	await press('[data-testid="plain"]', { ctrlKey: true });
	await press('[data-testid="plain"]', { altKey: true });
	await press('[data-testid="sheet"] button');
	expect(calls).toBe(0);
});

test("inside the open rail the key still reaches its filter", async () => {
	let calls = 0;
	await dom.render(
		<Area
			onFilter={() => {
				calls++;
				return true;
			}}
		/>,
	);
	await press('[data-testid="rail"] a');
	expect(calls).toBe(1);
});

test("a disabled area listens to nothing, and unmounting removes the listener", async () => {
	let calls = 0;
	const onFilter = () => {
		calls++;
		return true;
	};
	const view = await dom.render(<Area onFilter={onFilter} enabled={false} />);
	await press('[data-testid="plain"]');
	expect(calls).toBe(0);

	await view.rerender(<Area onFilter={onFilter} />);
	await press('[data-testid="plain"]');
	expect(calls).toBe(1);

	await view.unmount();
	await fire(dom.document.body, slash());
	expect(calls).toBe(1);
});

test("an event another handler already took is not a filter hotkey", () => {
	const event = slash();
	event.preventDefault();
	expect(isFilterHotkey(event)).toBe(false);
	expect(isFilterHotkey(slash())).toBe(true);
	expect(
		isFilterHotkey(
			new dom.window.KeyboardEvent("keydown", {
				key: "k",
			}) as unknown as KeyboardEvent,
		),
	).toBe(false);
});
