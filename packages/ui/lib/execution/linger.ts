export const STREAM_LINGER_MS = 60_000;

/** How long a caller that reads the run's logs next waits for the server to end the stream. */
export const STREAM_CLOSE_WAIT_MS = 10_000;

/**
 * The server settles run logs and quota after its terminal event and only then ends the stream;
 * closing the connection earlier can stop that work, so the reader stays open until the server
 * ends the stream or the cap lapses. Whatever arrives meanwhile is discarded. Resolves once the
 * reader is done, which is when the run's logs can be read.
 */
export function lingerUntilClosed(
	reader: ReadableStreamDefaultReader<Uint8Array>,
	capMs = STREAM_LINGER_MS,
): Promise<void> {
	const timer = setTimeout(() => {
		void reader.cancel().catch(() => undefined);
	}, capMs);
	return (async () => {
		try {
			while (!(await reader.read()).done);
		} catch {
			/* The run is already finished; a broken tail changes nothing. */
		} finally {
			clearTimeout(timer);
			try {
				reader.releaseLock();
			} catch {
				/* Already released. */
			}
		}
	})();
}

/** Waits for `closed`, but no longer than `waitMs` when the server keeps the stream open. */
export function closedWithin(
	closed: Promise<void>,
	waitMs = STREAM_CLOSE_WAIT_MS,
): Promise<void> {
	return new Promise((resolve) => {
		const timer = setTimeout(resolve, waitMs);
		void closed.finally(() => {
			clearTimeout(timer);
			resolve();
		});
	});
}
