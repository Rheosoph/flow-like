/**
 * Observes elements and delivers a single coalesced callback on the next
 * animation frame.
 *
 * A ResizeObserver callback that writes layout — sizing the observed element,
 * remounting its children, setting a max-height on it — leaves the browser with
 * observations it could not deliver before the frame ended. The spec requires
 * the user agent to report that as an error event on `window`, which surfaces as
 * the stackless "ResizeObserver loop completed with undelivered notifications".
 * Deferring the write by one frame closes the delivery loop: the resize still
 * lands, the notice is never raised, and a run of rapid resizes collapses into a
 * single callback per frame.
 */
export function observeResize(
	targets: readonly (Element | null | undefined)[],
	callback: () => void,
): () => void {
	if (typeof ResizeObserver === "undefined") return () => undefined;
	const elements = targets.filter((target): target is Element =>
		Boolean(target),
	);
	if (elements.length === 0) return () => undefined;

	let frame: number | undefined;
	const observer = new ResizeObserver(() => {
		if (frame !== undefined) return;
		frame = requestAnimationFrame(() => {
			frame = undefined;
			callback();
		});
	});
	for (const element of elements) observer.observe(element);

	return () => {
		if (frame !== undefined) cancelAnimationFrame(frame);
		observer.disconnect();
	};
}
