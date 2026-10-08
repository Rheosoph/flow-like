const PHASES = [
	"prepare",
	"connect",
	"open",
	"write",
	"verify",
	"cancel",
] as const;
const CODES = [
	"native_upload_failed",
	"connection_failed",
	"stream_failed",
	"cancelled",
] as const;

export type NativeArtifactUploadPhase = (typeof PHASES)[number];
export type NativeArtifactUploadCode = (typeof CODES)[number];

/** Safe desktop upload facts; native paths and transport error messages stay on the desktop. */
export class NativeArtifactUploadError extends Error {
	constructor(
		readonly phase: NativeArtifactUploadPhase | undefined,
		readonly code: NativeArtifactUploadCode,
	) {
		super("The desktop app did not confirm the project file upload.");
		this.name = "NativeArtifactUploadError";
	}
}

export function nativeArtifactUploadError(
	error: unknown,
): NativeArtifactUploadError {
	if (error && typeof error === "object") {
		const value = error as Record<string, unknown>;
		const phase = PHASES.find((phase) => phase === value.phase);
		const code = CODES.find((code) => code === value.code);
		if (phase && code) return new NativeArtifactUploadError(phase, code);
	}
	return new NativeArtifactUploadError(undefined, "native_upload_failed");
}
