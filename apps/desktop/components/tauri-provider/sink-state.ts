import { invoke } from "@tauri-apps/api/core";
import type { IEventRegistration, ISinkState } from "@flow-like/flow-like-ui";

export class SinkState implements ISinkState {
	async listEventSinks(): Promise<IEventRegistration[]> {
		const registrations =
			await invoke<IEventRegistration[]>("list_event_sinks");
		return registrations.map((reg) => ({
			...reg,
			updated_at:
				typeof reg.updated_at === "object"
					? (reg.updated_at as { secs_since_epoch: number }).secs_since_epoch *
						1000
					: reg.updated_at,
			created_at:
				typeof reg.created_at === "object"
					? (reg.created_at as { secs_since_epoch: number }).secs_since_epoch *
						1000
					: reg.created_at,
		}));
	}

	async removeEventSink(eventId: string): Promise<void> {
		await invoke("remove_event_sink", { eventId });
	}

	async isEventSinkActive(eventId: string): Promise<boolean> {
		return await invoke<boolean>("is_event_sink_active", { eventId });
	}
}
