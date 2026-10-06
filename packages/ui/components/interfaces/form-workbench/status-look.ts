import type { TFunction } from "i18next";
import {
	CircleCheck,
	CircleDashed,
	CirclePause,
	CircleQuestionMark,
	CircleStop,
	LoaderCircle,
	type LucideIcon,
	OctagonX,
} from "lucide-react";
import type { RunStatus } from "./contracts";

/** One icon per run status, always shown with its word (statusWord) and tone (RUN_STATUS_TONE). */
export const RUN_STATUS_ICON: Readonly<Record<RunStatus, LucideIcon>> = {
	sending: LoaderCircle,
	queued: CircleDashed,
	starting: LoaderCircle,
	asking: CirclePause,
	running: LoaderCircle,
	streaming: LoaderCircle,
	done: CircleCheck,
	empty: CircleCheck,
	failed: OctagonX,
	stopped: CircleStop,
	notStarted: CircleDashed,
	unknown: CircleQuestionMark,
};

/** Statuses whose icon spins (`animate-spin motion-reduce:animate-none`). */
export const RUN_STATUS_SPINS: Readonly<Record<RunStatus, boolean>> = {
	sending: true,
	queued: false,
	starting: true,
	asking: false,
	running: true,
	streaming: true,
	done: false,
	empty: false,
	failed: false,
	stopped: false,
	notStarted: false,
	unknown: false,
};

type InterfacesT = TFunction<"interfaces">;
type StatusCopy = (t: InterfacesT) => string;

const running: StatusCopy = (t) =>
	t("interfaces:workbench.status.running", "Running");
const done: StatusCopy = (t) => t("interfaces:workbench.status.done", "Done");

/** Literal keys, one per status, so the extractor keeps them (PLAN §9). */
const STATUS_WORD: Readonly<Record<RunStatus, StatusCopy>> = {
	sending: (t) => t("interfaces:workbench.status.sending", "Sending"),
	queued: (t) => t("interfaces:workbench.status.queued", "Queued"),
	starting: (t) => t("interfaces:workbench.status.starting", "Starting"),
	asking: (t) =>
		t("interfaces:workbench.status.asking", "Waiting for your answer"),
	running,
	streaming: running,
	done,
	empty: done,
	failed: (t) => t("interfaces:workbench.status.failed", "Failed"),
	stopped: (t) => t("interfaces:workbench.status.stopped", "Stopped"),
	notStarted: (t) => t("interfaces:workbench.status.notStarted", "Not started"),
	unknown: (t) => t("interfaces:workbench.status.unknown", "Result unknown"),
};

/** The word for a run status (tabs, run bar, Runs list). */
export function statusWord(t: InterfacesT, status: RunStatus) {
	return STATUS_WORD[status](t);
}
