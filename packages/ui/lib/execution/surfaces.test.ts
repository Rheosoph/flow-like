import { afterEach, expect, mock, test } from "bun:test";
import type { A2UIServerMessage } from "../../components/a2ui/types";
import type { IIntercomEvent } from "../schema/events/intercom-event";
import {
	deliverExecutionSurfaceEvent,
	deliverExecutionSurfaceEvents,
	executionElementSource,
	registerExecutionSurface,
} from "./surfaces";
import type { ExecutionTask } from "./types";

const cleanups: (() => void)[] = [];
afterEach(() => {
	for (const close of cleanups.splice(0)) close();
});

function event(type = "setPageState"): IIntercomEvent {
	return {
		event_id: crypto.randomUUID(),
		event_type: "a2ui",
		payload: { type, page_id: "page", key: "count", value: 1 },
		timestamp: { secs_since_epoch: 0, nanos_since_epoch: 0 },
	};
}

function task(events: IIntercomEvent[]): ExecutionTask {
	return {
		appId: crypto.randomUUID(),
		pageId: "page",
		taskId: "task",
		streamId: "stream",
		scope: "scope",
		eventId: "workflow",
		title: "Update page",
		startedAt: new Date().toISOString(),
		status: "running",
		events,
		pendingInteractions: [],
	};
}

function mount(
	target: Pick<ExecutionTask, "appId" | "pageId" | "widgetInstanceId">,
	onMessage = mock((_message: A2UIServerMessage) => {}),
) {
	if (!target.pageId) throw new Error("Test surface requires a page");
	const source = {
		surfaceId: target.pageId,
		components: {},
		storedValues: {},
		widgetScope: target.widgetInstanceId
			? { instanceId: target.widgetInstanceId, components: [] }
			: undefined,
	};
	const close = registerExecutionSurface({
		appId: target.appId,
		pageId: target.pageId,
		source: () => source,
		onMessage,
	});
	cleanups.push(close);
	return { close, source, onMessage };
}

test("shared snapshots and the original callback apply the same output once", () => {
	const run = task([event(), event("upsertElement")]);
	const page = mount(run);
	deliverExecutionSurfaceEvents(run);
	expect(deliverExecutionSurfaceEvent(run, run.events[0])).toBe(true);
	deliverExecutionSurfaceEvents(run);
	expect(page.onMessage.mock.calls.map(([message]) => message.type)).toEqual([
		"setPageState",
		"upsertElement",
	]);
});

test("output missed before mount catches up once while the run is active", () => {
	const run = task([event()]);
	expect(deliverExecutionSurfaceEvent(run, run.events[0])).toBe(false);
	const page = mount(run);
	deliverExecutionSurfaceEvents(run);
	expect(page.onMessage).toHaveBeenCalledTimes(1);
	page.close();
	const remounted = mount(run);
	deliverExecutionSurfaceEvents(run);
	expect(remounted.onMessage).not.toHaveBeenCalled();
});

test("terminal history cannot overwrite a mounted page", () => {
	for (const status of ["completed", "failed", "cancelled"] as const) {
		const run = { ...task([event()]), status };
		const page = mount(run);
		deliverExecutionSurfaceEvents(run);
		expect(page.onMessage).not.toHaveBeenCalled();
	}
});

test("the original callback records output applied after its page unmounts", () => {
	const run = task([event()]);
	const original = mock((_message: A2UIServerMessage) => {});
	expect(deliverExecutionSurfaceEvent(run, run.events[0], original)).toBe(true);
	expect(original).toHaveBeenCalledTimes(1);
	const page = mount(run);
	deliverExecutionSurfaceEvents(run);
	expect(page.onMessage).not.toHaveBeenCalled();
	page.close();
	deliverExecutionSurfaceEvent(run, run.events[0], original);
	expect(original).toHaveBeenCalledTimes(1);
});

test("page and widget output requires the exact app, page, and instance", () => {
	const run = task([event()]);
	const otherApp = mount({ ...run, appId: "another-app" });
	const otherPage = mount({ ...run, pageId: "another-page" });
	const widget = mount({ ...run, widgetInstanceId: "widget" });
	expect(deliverExecutionSurfaceEvent(run, run.events[0])).toBe(false);
	expect(executionElementSource(run.appId, run.pageId)).toBeNull();
	const page = mount(run);
	deliverExecutionSurfaceEvents({ ...run, widgetInstanceId: "widget" });
	expect(widget.onMessage).toHaveBeenCalledTimes(1);
	expect(page.onMessage).not.toHaveBeenCalled();
	deliverExecutionSurfaceEvents(run);
	expect(page.onMessage).toHaveBeenCalledTimes(1);
	expect(otherApp.onMessage).not.toHaveBeenCalled();
	expect(otherPage.onMessage).not.toHaveBeenCalled();
});

test("event dedupe is scoped to its app and page", () => {
	const first = task([event()]);
	const second = { ...first, appId: "second-app" };
	const third = { ...first, pageId: "second-page" };
	const pages = [mount(first), mount(second), mount(third)];
	for (const run of [first, second, third]) deliverExecutionSurfaceEvents(run);
	for (const page of pages) expect(page.onMessage).toHaveBeenCalledTimes(1);
});

test("catchup does not replay navigation, dialogs, or requests", () => {
	const run = task([
		event("navigateTo"),
		event("openDialog"),
		event("requestElements"),
		event("deviceCommand"),
	]);
	const page = mount(run);
	deliverExecutionSurfaceEvents(run);
	expect(page.onMessage).not.toHaveBeenCalled();
});

test("a failed output remains retryable and cannot masquerade as delivered", () => {
	const run = task([event()]);
	const onMessage = mock((_message: A2UIServerMessage) => {
		throw new Error("Renderer failed");
	});
	const page = mount(run, onMessage);
	expect(() => deliverExecutionSurfaceEvents(run)).toThrow("Renderer failed");
	page.close();
	const replacement = mount(run);
	deliverExecutionSurfaceEvents(run);
	expect(replacement.onMessage).toHaveBeenCalledTimes(1);
});
