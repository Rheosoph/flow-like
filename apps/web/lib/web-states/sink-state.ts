import type { IEventRegistration, ISinkState } from "@flow-like/flow-like-ui";
import { type WebBackendRef, apiDelete, apiGet } from "./api-utils";

export class WebSinkState implements ISinkState {
	constructor(private readonly backend: WebBackendRef) {}

	async listEventSinks(): Promise<IEventRegistration[]> {
		return apiGet<IEventRegistration[]>("sinks", this.backend.auth);
	}

	async removeEventSink(eventId: string): Promise<void> {
		await apiDelete<void>(`sinks/${eventId}`, this.backend.auth);
	}

	async isEventSinkActive(eventId: string): Promise<boolean> {
		try {
			const result = await apiGet<{ active: boolean }>(
				`sinks/${eventId}/status`,
				this.backend.auth,
			);
			return result?.active ?? false;
		} catch {
			return false;
		}
	}
}
