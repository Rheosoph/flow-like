import { getApiUrl } from "@flow-like/flow-like-ui/lib/api-url";
import { createMeetingClient } from "@flow-like/flow-like-ui/lib/meetings/client";
import { createMeetingTransport } from "@flow-like/flow-like-ui/lib/meetings/transport";
import { type WebBackendRef, apiFetch } from "./api-utils";

export function createWebMeetingClient(backend: WebBackendRef) {
	return createMeetingClient({
		request: createMeetingTransport({
			accountRequest: (path, init) => apiFetch(path, init, backend.auth),
			apiUrl: (path) => getApiUrl(null, path),
		}),
	});
}
