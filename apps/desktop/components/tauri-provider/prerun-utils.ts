import { IExecutionMode } from "@tm9657/flow-like-ui";

type PrerunLike = {
	can_execute_locally: boolean;
	execution_mode: IExecutionMode;
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
			const remoteResult = await fetchRemote();

			if (remoteResult) {
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
