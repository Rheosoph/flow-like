import type { IApiState } from "../state/backend-state/api-state";
import type { IProfile } from "../types";

export async function requestGovernanceAssist<T>(
	apiState: Pick<IApiState, "stream">,
	profile: IProfile,
	path: string,
	data: Record<string, unknown>,
): Promise<T> {
	let response: T | undefined;
	await apiState.stream<unknown>(
		profile,
		path,
		{
			method: "POST",
			headers: { Accept: "text/event-stream" },
			body: JSON.stringify({ ...data, stream: true }),
		},
		(message) => {
			if (
				message &&
				typeof message === "object" &&
				"suggestion" in message &&
				"signals" in message &&
				"model" in message &&
				typeof message.model === "string"
			) {
				response = message as T;
			}
		},
	);

	if (response === undefined) {
		throw new Error(
			"The assessment assistant disconnected before returning a result. Please try again.",
		);
	}
	return response;
}
