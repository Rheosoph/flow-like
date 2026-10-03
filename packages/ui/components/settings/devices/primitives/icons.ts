import {
	Ban,
	CircleAlert,
	CircleCheck,
	CircleDashed,
	CircleDot,
	CirclePause,
	Clock,
	Gauge,
	History as HistoryIcon,
	Hourglass,
	Inbox,
	Info,
	KeyRound,
	Laptop,
	ListChecks,
	Lock,
	LockKeyhole,
	type LucideIcon,
	OctagonX,
	Radio,
	Server,
	ShieldAlert,
	ShieldCheck,
	Terminal,
	TriangleAlert,
	Undo2,
	User,
	Users,
	Wrench,
} from "lucide-react";
import type { Tone } from "./tone";

/* Vocabularies shared by the primitives. CA9: StampSource/StampAge are byte-identical to SourcePlane/AgeState in DM/model/types.ts. */

export type StampSource =
	| "hub"
	| "snap"
	| "saved"
	| "live"
	| "local"
	| "device";
export type StampAge =
	| "live"
	| "current"
	| "delayed"
	| "lastknown"
	| "snapshot"
	| "locked"
	| "notloaded"
	| "noaccess"
	| "unsupported"
	| "error";

/** SPEC §4.5 gate notice kinds; byte-identical to GateNoticeKind in DM/model/types.ts. */
export type GateKind =
	| "locked"
	| "nokeys"
	| "noaccess"
	| "owner"
	| "unsupported"
	| "policy"
	| "plan"
	| "role"
	| "platform"
	| "hub"
	| "live"
	| "busy";

export type StateKind =
	| "empty"
	| "notloaded"
	| "never"
	| "locked"
	| "noaccess"
	| "unsupported"
	| "error"
	| "loading"
	| "gate";

/** Same union as Severity in DM/model/types.ts. */
export type SeverityKind = "critical" | "warning" | "notice" | "info";

export type ConseqKind = "what" | "who" | "stays" | "when" | "undo" | "first";

export const SOURCE_ICON: Record<StampSource, LucideIcon> = {
	hub: Server,
	snap: LockKeyhole,
	saved: HistoryIcon,
	live: Radio,
	local: Laptop,
	device: Terminal,
};

export const GATE_ICON: Record<GateKind, LucideIcon> = {
	locked: Lock,
	nokeys: KeyRound,
	noaccess: Ban,
	owner: User,
	unsupported: Wrench,
	policy: ShieldAlert,
	plan: Gauge,
	role: Users,
	platform: Laptop,
	hub: Server,
	live: Radio,
	busy: Hourglass,
};

export const STATE_ICON: Record<
	Exclude<StateKind, "loading" | "gate">,
	LucideIcon
> = {
	empty: Inbox,
	notloaded: CircleDashed,
	never: CircleDashed,
	locked: Lock,
	noaccess: Ban,
	unsupported: Wrench,
	error: OctagonX,
};

export const SEVERITY_ICON: Record<SeverityKind, LucideIcon> = {
	critical: OctagonX,
	warning: TriangleAlert,
	notice: CircleAlert,
	info: Info,
};

export const SEVERITY_TONE: Record<SeverityKind, Tone> = {
	critical: "critical",
	warning: "warning",
	notice: "info",
	info: "unknown",
};

export const TONE_ICON: Record<Tone, LucideIcon> = {
	good: CircleCheck,
	warning: TriangleAlert,
	critical: OctagonX,
	info: Info,
	unknown: CircleDashed,
	paused: CirclePause,
	locked: Lock,
};

export const CONSEQ_ICON: Record<ConseqKind, LucideIcon> = {
	what: CircleDot,
	who: Users,
	stays: ShieldCheck,
	when: Clock,
	undo: Undo2,
	first: ListChecks,
};
