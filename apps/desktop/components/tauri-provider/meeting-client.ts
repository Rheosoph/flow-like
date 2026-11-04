import { getApiUrl } from "@flow-like/flow-like-ui/lib/api-url";
import { createMeetingClient } from "@flow-like/flow-like-ui/lib/meetings/client";
import { createMeetingTransport } from "@flow-like/flow-like-ui/lib/meetings/transport";
import { fetcher } from "../../lib/api";
import type { TauriBackend } from "../tauri-provider";

export function createNativeMeetingClient(backend: TauriBackend) {
	return createMeetingClient({
		request: createMeetingTransport({
			accountRequest: (path, init) => {
				if (!backend.profile)
					throw new Error("Sign in to schedule or join a room meeting.");
				return fetcher(backend.profile, path, init, backend.auth);
			},
			apiUrl: (path) => getApiUrl(backend.profile, path),
		}),
	});
}
