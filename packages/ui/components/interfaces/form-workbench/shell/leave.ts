import type { TFunction } from "i18next";
import type { FormSessionState, RunEntry } from "../contracts";

type InterfacesT = TFunction<"interfaces">;

/** What leaving the form would drop: runs that wait for their turn or their files, and next files. */
export interface LeaveCounts {
	readonly runs: number;
	readonly files: number;
}

const waitsForItsTurn = (run: RunEntry) =>
	run.origin === "session" &&
	(run.status === "queued" || run.status === "sending");

export function leaveCountsOf(
	state: Pick<FormSessionState, "runs" | "rail">,
): LeaveCounts {
	const files = Object.values(state.rail.nextFiles).reduce(
		(total, list) => total + list.length,
		0,
	);
	return { runs: state.runs.filter(waitsForItsTurn).length, files };
}

export const hasWaiting = (counts: LeaveCounts) =>
	counts.runs > 0 || counts.files > 0;

const runsPart = (t: InterfacesT, count: number) =>
	t("interfaces:workbench.shell.leave.runs", "{{count}} runs will not start", {
		count,
		defaultValue_one: "{{count}} run will not start",
	});

const filesPart = (t: InterfacesT, count: number) =>
	t(
		"interfaces:workbench.shell.leave.files",
		"{{count}} next files will be removed",
		{ count, defaultValue_one: "{{count}} next file will be removed" },
	);

/** "1 run will not start and 5 next files will be removed."; null when nothing waits. */
export function leaveBody(t: InterfacesT, counts: LeaveCounts) {
	const parts = [
		counts.runs > 0 ? runsPart(t, counts.runs) : null,
		counts.files > 0 ? filesPart(t, counts.files) : null,
	].filter((part): part is string => part !== null);
	if (parts.length === 2)
		return t(
			"interfaces:workbench.shell.leave.two",
			"{{first}} and {{second}}.",
			{
				first: parts[0],
				second: parts[1],
			},
		);
	if (parts.length === 1)
		return t("interfaces:workbench.shell.leave.one", "{{what}}.", {
			what: parts[0],
		});
	return null;
}
