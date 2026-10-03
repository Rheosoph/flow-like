import type { AppInput } from "../../../../lib/device-management/model/app-plan";
import { parseUint8ArrayToJson } from "../../../../lib/uint8";
import type {
	IEventRegistration,
	ISinkState,
} from "../../../../state/backend-state/sink-state";

/*
 * This computer's own triggers: the events the desktop app runs itself while
 * Flow-Like is open (`backend.sinkState`, and `eventState.isEventSinkActive`).
 * A deploy asks for them before it moves a schedule or a bot to a device.
 *
 *   const fake = await createFakeWorkspace(seed, { platform: "desktop", localTriggers: ["evt_helper"] });
 *   fake.sinks.has("evt_helper");   // true until "Stop running it on this computer"
 *   fake.sinks.removed;             // ["evt_helper"] afterwards
 */

/** An event's config as the app's store keeps it (tokens included): the record's bytes, else its schedule. */
function storedConfig(
	event: AppInput["events"][number],
): Record<string, unknown> {
	const parsed = event.config?.length
		? parseUint8ArrayToJson([...event.config])
		: event.schedule;
	return parsed && typeof parsed === "object" && !Array.isArray(parsed)
		? { ...(parsed as Record<string, unknown>) }
		: {};
}

export class FakeLocalTriggers {
	private readonly running: Set<string>;
	/** Every trigger stopped here, oldest first. */
	readonly removed: string[] = [];
	/** Set to make every read fail, as a desktop command that errors. */
	failure: Error | null = null;

	constructor(
		private readonly apps: () => Readonly<Record<string, AppInput>>,
		private readonly now: () => number,
		eventIds: readonly string[] = [],
	) {
		this.running = new Set(eventIds);
	}

	/** This computer starts running the event's trigger. */
	run(eventId: string): void {
		this.running.add(eventId);
	}

	has(eventId: string): boolean {
		return this.running.has(eventId);
	}

	private event(eventId: string) {
		for (const app of Object.values(this.apps())) {
			const event = app.events.find((candidate) => candidate.id === eventId);
			if (event) return { app, event };
		}
		return undefined;
	}

	/** The registrations `sinkState.listEventSinks()` answers; an event no app knows is listed bare. */
	list(): IEventRegistration[] {
		const at = this.now();
		return [...this.running].map((eventId) => {
			const known = this.event(eventId);
			return {
				event_id: eventId,
				name: known?.event.name ?? eventId,
				type: known?.event.event_type ?? "unknown",
				created_at: at,
				updated_at: at,
				config: known ? storedConfig(known.event) : {},
				offline: known?.app.visibility === "Offline",
				app_id: known?.app.id ?? "",
			};
		});
	}

	private check() {
		if (this.failure) throw this.failure;
	}

	/** The desktop's `sinkState`, backed by this list. */
	state(): ISinkState {
		return {
			listEventSinks: async () => {
				this.check();
				return this.list();
			},
			removeEventSink: async (eventId: string) => {
				this.check();
				this.removed.push(eventId);
				this.running.delete(eventId);
			},
			isEventSinkActive: async (eventId: string) => {
				this.check();
				return this.running.has(eventId);
			},
		};
	}
}
