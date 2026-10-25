/**
 * Runs `callback` after this frame has rendered and before the next one paints. egui's frame
 * callback is always queued ahead of the host's, so a camera or geometry change made inside a
 * frame callback reaches the canvas a frame after the DOM. Made between frames, both draw it in
 * the same frame. Returns a cancel function.
 */
export function betweenFrames(callback: (time: number) => void): () => void {
	let timer: ReturnType<typeof setTimeout> | undefined;
	const frame = requestAnimationFrame(() => {
		timer = setTimeout(() => callback(performance.now()), 0);
	});
	return () => {
		cancelAnimationFrame(frame);
		clearTimeout(timer);
	};
}
