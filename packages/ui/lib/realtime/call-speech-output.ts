/** Mixes generated speech into the outgoing microphone stream and plays it locally. */
export function createCallSpeechOutput(
	context: AudioContext,
	microphone: MediaStream,
	onSpeaking: (botId: string | null) => void,
) {
	const destination = context.createMediaStreamDestination();
	const input = context.createMediaStreamSource(microphone);
	input.connect(destination);
	let generation = 0;
	let disposed = false;
	let finish: (() => void) | null = null;

	const stop = () => {
		generation++;
		finish?.();
	};

	return {
		stream: destination.stream,
		async play(blob: Blob, botId: string): Promise<void> {
			stop();
			if (disposed) return;
			const current = generation;
			const buffer = await context.decodeAudioData(await blob.arrayBuffer());
			if (disposed || current !== generation) return;
			await context.resume();
			if (disposed || current !== generation) return;
			if (context.state !== "running")
				throw new Error("Call audio playback is unavailable");
			await new Promise<void>((resolve, reject) => {
				const source = context.createBufferSource();
				source.buffer = buffer;
				let ended = false;
				const done = () => {
					if (ended) return;
					ended = true;
					source.onended = null;
					try {
						source.stop();
					} catch {}
					source.disconnect();
					finish = null;
					onSpeaking(null);
					resolve();
				};
				finish = done;
				source.onended = done;
				try {
					source.connect(destination);
					source.connect(context.destination);
					onSpeaking(botId);
					source.start();
				} catch (cause) {
					// Reject before cleanup resolves the promise so callers can show the text fallback.
					reject(cause);
					done();
				}
			});
		},
		stop,
		dispose() {
			if (disposed) return;
			disposed = true;
			stop();
			input.disconnect();
			destination.disconnect();
			for (const track of destination.stream.getTracks()) track.stop();
		},
	};
}
