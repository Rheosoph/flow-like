export type TurnModelRole = "speech" | "turn";

export const SILERO_VAD_URL =
	"https://huggingface.co/onnx-community/silero-vad/resolve/e71cae966052b992a7eca6b17738916ce0eca4ec/onnx/model.onnx";
export const SMART_TURN_URL =
	"https://huggingface.co/pipecat-ai/smart-turn-v3/resolve/f766f81d3cfdf7737ac64aad813d91bbfd56bf93/smart-turn-v3.2-cpu.onnx";
export const TURN_MODEL_CACHE = "flow-like-call-turn";

export const TURN_MODEL_URLS: Record<TurnModelRole, string> = {
	speech: SILERO_VAD_URL,
	turn: SMART_TURN_URL,
};

export type TurnWorkerRequest =
	| { type: "init"; role: TurnModelRole }
	| { type: "speech"; id: number; stream: string; samples: Float32Array }
	| { type: "reset"; stream: string }
	| { type: "turn"; id: number; samples: Float32Array }
	| { type: "disconnect" };

export type TurnWorkerResponse =
	| { type: "ready"; role: TurnModelRole }
	| { type: "failed"; role: TurnModelRole; message: string }
	| { type: "result"; id: number; probability: number | null }
	/** The worker freed its model and is closing; connect again when a call needs it. */
	| { type: "idle" };
