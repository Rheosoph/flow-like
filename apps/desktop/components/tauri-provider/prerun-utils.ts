import { IExecutionMode } from "@flow-like/flow-like-ui";
import { asArray, isRecord } from "@flow-like/flow-like-ui/lib/response-shape";

type PrerunLike = {
	can_execute_locally: boolean;
	execution_mode: IExecutionMode;
	runtime_variables: readonly unknown[];
	oauth_requirements: readonly unknown[];
};

function toError(error: unknown): Error {
	if (error instanceof Error) {
		return error;
	}

	return new Error(String(error));
}

export async function resolveLocalFirstPrerun<T extends PrerunLike>({
	label,
	buildLocal,
	fetchRemote,
}: {
	label: string;
	buildLocal: () => Promise<T>;
	fetchRemote?: () => Promise<T | null | undefined>;
}): Promise<T> {
	let localError: unknown;

	try {
		return await buildLocal();
	} catch (error) {
		localError = error;
		console.warn(
			`[${label}] Local prerun unavailable, falling back to API:`,
			error,
		);
	}

	if (fetchRemote) {
		try {
			const fetched = await fetchRemote();

			if (isRecord(fetched)) {
				// Older hubs may omit the lists the run dialog iterates.
				const remoteResult = {
					...fetched,
					runtime_variables: asArray(fetched.runtime_variables),
					oauth_requirements: asArray(fetched.oauth_requirements),
				};
				if (
					remoteResult.can_execute_locally &&
					remoteResult.execution_mode !== IExecutionMode.Remote
				) {
					return {
						...remoteResult,
						can_execute_locally: false,
					};
				}

				return remoteResult;
			}
		} catch (error) {
			console.warn(
				`[${label}] API prerun failed after local prerun failed:`,
				error,
			);
		}
	}

	throw toError(localError);
}
