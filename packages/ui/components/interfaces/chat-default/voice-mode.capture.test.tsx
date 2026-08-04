import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import type { VoiceConfig } from "../../voice";

interface SpeechResultEvent {
	resultIndex: number;
	results: ArrayLike<ArrayLike<{ transcript: string }> & { isFinal: boolean }>;
}

class FakeSpeechRecognition {
	static instances: FakeSpeechRecognition[] = [];

	continuous = true;
	interimResults = true;
	lang = "";
	startCalls = 0;
	stopCalls = 0;
	onresult: ((event: SpeechResultEvent) => void) | null = null;
	onerror: ((error: unknown) => void) | null = null;
	onend: (() => void) | null = null;

	constructor() {
		FakeSpeechRecognition.instances.push(this);
	}

	start() {
		this.startCalls += 1;
	}

	stop() {
		this.stopCalls += 1;
	}

	abort() {
		this.stopCalls += 1;
	}

	fail(error: unknown = new Error("speech unavailable")) {
		this.onerror?.(error);
	}

	finishWith(transcript: string, isFinal = true) {
		const result = Object.assign([{ transcript }], { isFinal });
		this.onresult?.({ resultIndex: 0, results: [result] });
		this.onend?.();
	}
}

interface FakeMediaRecorderOptions {
	mimeType?: string;
}

class FakeMediaRecorder {
	static instances: FakeMediaRecorder[] = [];
	static isTypeSupported = mock(() => true);

	state: "inactive" | "recording" = "inactive";
	mimeType: string;
	ondataavailable: ((event: { data: Blob }) => void) | null = null;
	onstart: (() => void) | null = null;
	onstop: (() => void) | null = null;

	constructor(
		readonly stream: MediaStream,
		options?: FakeMediaRecorderOptions,
	) {
		this.mimeType = options?.mimeType ?? "audio/webm";
		FakeMediaRecorder.instances.push(this);
	}

	start() {
		this.state = "recording";
		this.onstart?.();
	}

	stop() {
		this.state = "inactive";
		this.ondataavailable?.({
			data: new Blob(["recorded audio"], { type: this.mimeType }),
		});
		this.onstop?.();
	}
}

const voiceConfig = (mode: "stt" | "record"): VoiceConfig => ({
	mode,
	invoke: "manual",
	variant: "conservative",
	size: "md",
	color: "#8b5cf6",
	recordingColor: "#ef4444",
	playback: "text",
	maxDuration: 30,
	autoStop: false,
});

let browserWindow: Window;
let root: Root | null;
const voiceModeTestModule = "./VoiceMode.tsx?voice-mode-capture-test";

const setBrowserProperty = (
	target: object,
	key: PropertyKey,
	value: unknown,
) => {
	Object.defineProperty(target, key, {
		configurable: true,
		writable: true,
		value,
	});
};

const flushAsyncCapture = async () => {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
};

beforeEach(() => {
	browserWindow = new Window({ url: "http://localhost" });
	root = null;
	FakeSpeechRecognition.instances = [];
	FakeMediaRecorder.instances = [];
	FakeMediaRecorder.isTypeSupported.mockClear();

	Object.assign(globalThis, {
		Blob: browserWindow.Blob,
		document: browserWindow.document,
		Event: browserWindow.Event,
		File: browserWindow.File,
		HTMLElement: browserWindow.HTMLElement,
		KeyboardEvent: browserWindow.KeyboardEvent,
		MouseEvent: browserWindow.MouseEvent,
		Node: browserWindow.Node,
		navigator: browserWindow.navigator,
		requestAnimationFrame:
			browserWindow.requestAnimationFrame.bind(browserWindow),
		cancelAnimationFrame:
			browserWindow.cancelAnimationFrame.bind(browserWindow),
		window: browserWindow,
		IS_REACT_ACT_ENVIRONMENT: true,
	});
});

afterEach(async () => {
	if (root) await act(async () => root?.unmount());
	browserWindow.close();
});

async function renderVoiceMode(
	mode: "stt" | "record",
	onSend: (content: string, audioFile?: File) => void | Promise<void>,
) {
	const { VoiceMode } = await import(voiceModeTestModule);
	const container = browserWindow.document.createElement("div");
	browserWindow.document.body.append(container);
	root = createRoot(container as unknown as Element);

	await act(async () => {
		root?.render(
			<VoiceMode
				open
				onClose={() => {}}
				onSend={onSend}
				voice={voiceConfig(mode)}
			/>,
		);
	});
}

describe("VoiceMode capture", () => {
	test("starts on-device speech recognition and submits its transcript without recording", async () => {
		setBrowserProperty(
			browserWindow,
			"SpeechRecognition",
			FakeSpeechRecognition,
		);
		setBrowserProperty(browserWindow, "MediaRecorder", FakeMediaRecorder);
		setBrowserProperty(globalThis, "MediaRecorder", FakeMediaRecorder);
		const getUserMedia = mock(async () => {
			throw new Error("recording should not be requested in STT mode");
		});
		setBrowserProperty(browserWindow.navigator, "mediaDevices", {
			getUserMedia,
		});
		const onSend = mock(
			async (_content: string, _audioFile?: File): Promise<void> => {},
		);

		await renderVoiceMode("stt", onSend);

		expect(FakeSpeechRecognition.instances).toHaveLength(1);
		const recognition = FakeSpeechRecognition.instances[0];
		expect(recognition.startCalls).toBe(1);
		expect(recognition.continuous).toBe(false);
		const sendOrb = [
			...browserWindow.document.getElementsByTagName("button"),
		].find((button) => button.getAttribute("aria-label") === "Tap to send");
		expect(sendOrb).toBeDefined();
		expect(FakeMediaRecorder.instances).toHaveLength(0);
		expect(getUserMedia).not.toHaveBeenCalled();

		await act(async () => {
			recognition.finishWith("send this from the device");
		});

		expect(onSend).toHaveBeenCalledTimes(1);
		expect(onSend).toHaveBeenCalledWith("send this from the device", undefined);
		expect(FakeMediaRecorder.instances).toHaveLength(0);
	});

	test("submits the displayed interim transcript when recognition ends before a final result", async () => {
		setBrowserProperty(
			browserWindow,
			"SpeechRecognition",
			FakeSpeechRecognition,
		);
		const onSend = mock(
			async (_content: string, _audioFile?: File): Promise<void> => {},
		);

		await renderVoiceMode("stt", onSend);
		const recognition = FakeSpeechRecognition.instances[0];

		await act(async () => {
			recognition.finishWith("send the words I can see", false);
		});

		expect(onSend).toHaveBeenCalledTimes(1);
		expect(onSend).toHaveBeenCalledWith("send the words I can see", undefined);
	});

	test("switches the voice overlay to recording fallback after STT fails", async () => {
		setBrowserProperty(
			browserWindow,
			"SpeechRecognition",
			FakeSpeechRecognition,
		);
		const onSend = mock(
			async (_content: string, _audioFile?: File): Promise<void> => {},
		);
		const originalConsoleError = console.error;
		console.error = mock(() => {});

		try {
			await renderVoiceMode("stt", onSend);
			await act(async () => {
				FakeSpeechRecognition.instances[0].fail();
			});

			expect(browserWindow.document.body.textContent).toContain(
				"Speech recognition is unavailable",
			);
			const retryOrb = [
				...browserWindow.document.getElementsByTagName("button"),
			].find((button) => button.getAttribute("aria-label") === "Tap to talk");
			expect(retryOrb).toBeDefined();
			expect(onSend).not.toHaveBeenCalled();
		} finally {
			console.error = originalConsoleError;
		}
	});

	test("cancels the old speech session when voice mode closes and reopens", async () => {
		setBrowserProperty(
			browserWindow,
			"SpeechRecognition",
			FakeSpeechRecognition,
		);
		const onSend = mock(
			async (_content: string, _audioFile?: File): Promise<void> => {},
		);
		await renderVoiceMode("stt", onSend);
		const staleRecognition = FakeSpeechRecognition.instances[0];
		const { VoiceMode } = await import(voiceModeTestModule);

		await act(async () => {
			root?.render(
				<VoiceMode
					open={false}
					onClose={() => {}}
					onSend={onSend}
					voice={voiceConfig("stt")}
				/>,
			);
		});
		await act(async () => {
			root?.render(
				<VoiceMode
					open
					onClose={() => {}}
					onSend={onSend}
					voice={voiceConfig("stt")}
				/>,
			);
		});

		expect(FakeSpeechRecognition.instances).toHaveLength(2);
		expect(staleRecognition.onend).toBeNull();
		await act(async () => {
			staleRecognition.finishWith("stale words");
			FakeSpeechRecognition.instances[1].finishWith("fresh words");
		});
		expect(onSend).toHaveBeenCalledTimes(1);
		expect(onSend).toHaveBeenCalledWith("fresh words", undefined);
	});

	test("falls back to MediaRecorder when STT is unavailable and submits the audio file", async () => {
		setBrowserProperty(browserWindow, "SpeechRecognition", undefined);
		setBrowserProperty(browserWindow, "webkitSpeechRecognition", undefined);
		setBrowserProperty(browserWindow, "MediaRecorder", FakeMediaRecorder);
		setBrowserProperty(globalThis, "MediaRecorder", FakeMediaRecorder);

		const stopTrack = mock(() => {});
		const stream = {
			getTracks: () => [{ stop: stopTrack }],
		} as unknown as MediaStream;
		const getUserMedia = mock(async () => stream);
		setBrowserProperty(browserWindow.navigator, "mediaDevices", {
			getUserMedia,
		});

		const closeAudioContext = mock(async () => {});
		class FakeAudioContext {
			resume = mock(async () => {});
			close = closeAudioContext;
			createMediaStreamSource() {
				return { connect: mock(() => {}) };
			}
			createAnalyser() {
				return {
					fftSize: 2048,
					frequencyBinCount: 1024,
					getFloatTimeDomainData: mock(() => {}),
				};
			}
		}
		setBrowserProperty(browserWindow, "AudioContext", FakeAudioContext);
		setBrowserProperty(globalThis, "AudioContext", FakeAudioContext);
		const onSend = mock(
			async (_content: string, _audioFile?: File): Promise<void> => {},
		);

		await renderVoiceMode("stt", onSend);
		await flushAsyncCapture();

		expect(getUserMedia).toHaveBeenCalledTimes(1);
		expect(FakeMediaRecorder.instances).toHaveLength(1);
		const recorder = FakeMediaRecorder.instances[0];
		expect(recorder.state).toBe("recording");

		await act(async () => recorder.stop());

		expect(onSend).toHaveBeenCalledTimes(1);
		const [content, audioFile] = onSend.mock.calls[0];
		expect(content).toBe("");
		expect(audioFile).toBeInstanceOf(browserWindow.File);
		expect(audioFile?.name).toEndWith(".webm");
		expect(audioFile?.size).toBeGreaterThan(0);
		expect(stopTrack).toHaveBeenCalledTimes(1);
		expect(closeAudioContext).toHaveBeenCalledTimes(1);
	});

	test("shows a microphone permission error when recording cannot start", async () => {
		setBrowserProperty(browserWindow, "SpeechRecognition", undefined);
		setBrowserProperty(browserWindow, "webkitSpeechRecognition", undefined);
		setBrowserProperty(browserWindow, "MediaRecorder", FakeMediaRecorder);
		setBrowserProperty(globalThis, "MediaRecorder", FakeMediaRecorder);
		setBrowserProperty(globalThis, "AudioContext", class {});
		setBrowserProperty(browserWindow.navigator, "mediaDevices", {
			getUserMedia: mock(async () => {
				throw new Error("permission denied");
			}),
		});
		const onSend = mock(
			async (_content: string, _audioFile?: File): Promise<void> => {},
		);
		const originalConsoleError = console.error;
		console.error = mock(() => {});

		try {
			await renderVoiceMode("record", onSend);
			await flushAsyncCapture();

			expect(browserWindow.document.body.textContent).toContain(
				"Microphone access failed",
			);
			expect(onSend).not.toHaveBeenCalled();
		} finally {
			console.error = originalConsoleError;
		}
	});
});
