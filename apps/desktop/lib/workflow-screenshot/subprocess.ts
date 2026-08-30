export interface SubprocessStatus {
	code: number | null;
	signal: string | null;
}

export function subprocessFailureMessage(
	label: string,
	status: SubprocessStatus,
	preferred?: string,
	stderr = "",
): string {
	const preferredMessage = preferred?.trim();
	if (preferredMessage) return preferredMessage;
	const stderrLine = stderr
		.split(/\r?\n/)
		.map((line) => line.trim())
		.filter(Boolean)
		.at(-1);
	if (stderrLine) return stderrLine;
	if (status.signal) return `${label} exited after signal ${status.signal}.`;
	return `${label} exited with code ${status.code ?? "unknown"}.`;
}
