import {
	ExternalLinkIcon,
	EyeIcon,
	Globe,
	InfoIcon,
	KeyRound,
	Lock,
	type LucideIcon,
	SettingsIcon,
	Shield,
	ShieldIcon,
} from "lucide-react";
import { IAppVisibility } from "../../../types";

/** Everything that can be published behind a reviewed visibility gate. */
export type IVisibilityEntityNoun = "app" | "suite";

export interface IVisibilityMeta {
	/** Headline used by the switcher and its toasts. */
	title: string;
	/** Compact wording used by store badges. */
	badgeLabel: string;
	/** Dot / accent utility class. */
	color: string;
	description: string;
	/** Switcher icon. */
	Icon: LucideIcon;
	/** Store badge icon. */
	BadgeIcon: LucideIcon;
	/** Copy for the app-card hover tooltip. */
	tooltip: string;
}

export const VISIBILITY_META: Record<IAppVisibility, IVisibilityMeta> = {
	[IAppVisibility.Offline]: {
		title: "Offline",
		badgeLabel: "Offline",
		color: "bg-slate-500",
		description: "Only local, no syncing across devices",
		Icon: ShieldIcon,
		BadgeIcon: Lock,
		tooltip: "App is currently offline",
	},
	[IAppVisibility.Private]: {
		title: "Private",
		badgeLabel: "Private",
		color: "bg-blue-500",
		description: "Synced for your account only",
		Icon: EyeIcon,
		BadgeIcon: Lock,
		tooltip: "Private access only",
	},
	[IAppVisibility.Prototype]: {
		title: "Prototype",
		badgeLabel: "Prototype",
		color: "bg-yellow-500",
		description: "Development phase, invite collaborators",
		Icon: SettingsIcon,
		BadgeIcon: Shield,
		tooltip: "Experimental prototype",
	},
	[IAppVisibility.PublicRequestAccess]: {
		title: "Public Request",
		badgeLabel: "Request access",
		color: "bg-orange-500",
		description: "Visible, people can request to join",
		Icon: InfoIcon,
		BadgeIcon: KeyRound,
		tooltip: "Public with access request",
	},
	[IAppVisibility.Public]: {
		title: "Public",
		badgeLabel: "Public",
		color: "bg-emerald-500",
		description: "Everyone can join, visible in store",
		Icon: ExternalLinkIcon,
		BadgeIcon: Globe,
		tooltip: "Publicly available",
	},
};

export function visibilityMeta(
	visibility: IAppVisibility,
): IVisibilityMeta | undefined {
	return VISIBILITY_META[visibility];
}

const TRANSITIONS: Record<IAppVisibility, IAppVisibility[]> = {
	[IAppVisibility.Offline]: [],
	[IAppVisibility.Private]: [IAppVisibility.Prototype],
	[IAppVisibility.Prototype]: [
		IAppVisibility.Private,
		IAppVisibility.PublicRequestAccess,
		IAppVisibility.Public,
	],
	[IAppVisibility.PublicRequestAccess]: [
		IAppVisibility.Public,
		IAppVisibility.Prototype,
	],
	[IAppVisibility.Public]: [
		IAppVisibility.PublicRequestAccess,
		IAppVisibility.Prototype,
	],
};

export function getVisibilityTransitions(
	current: IAppVisibility,
): IAppVisibility[] {
	return TRANSITIONS[current] ?? [];
}

export interface IVisibilityTransitionWarning {
	title: string;
	message: string;
	severity: "warning" | "danger" | "info";
}

function isPublicLevel(visibility: IAppVisibility): boolean {
	return (
		visibility === IAppVisibility.Public ||
		visibility === IAppVisibility.PublicRequestAccess
	);
}

export function getVisibilityTransitionWarning(
	from: IAppVisibility,
	to: IAppVisibility,
	noun: IVisibilityEntityNoun = "app",
): IVisibilityTransitionWarning {
	if (from === IAppVisibility.Prototype && to === IAppVisibility.Private) {
		return {
			title: "Remove All Collaborators",
			message:
				noun === "suite"
					? "Switching to Private hides the suite everywhere outside the anchor app's team. The member apps keep their own visibility and access."
					: "Switching to Private will remove all collaborators from your project. They will lose access immediately.",
			severity: "warning",
		};
	}

	if (isPublicLevel(from) && isPublicLevel(to)) {
		return {
			title: "Change Access Mode",
			message:
				to === IAppVisibility.Public
					? `Everyone will be able to join directly. Your ${noun} stays listed in the store.`
					: `Visitors will have to request access before joining. Your ${noun} stays listed in the store.`,
			severity: "info",
		};
	}

	if (from === IAppVisibility.Prototype && isPublicLevel(to)) {
		return {
			title: "Submit for Review",
			message: `Your ${noun} will be submitted for central revision. This process may take 1-3 business days. You'll be notified once the review is complete.`,
			severity: "info",
		};
	}

	if (isPublicLevel(from) && to === IAppVisibility.Prototype) {
		return {
			title: "Return to Development",
			message: `Your ${noun} will be removed from public visibility and submitted for central revision to return to prototype status. This may take 1-3 business days.`,
			severity: "warning",
		};
	}

	return {
		title: "Change Visibility",
		message: "Are you sure you want to change the visibility status?",
		severity: "info",
	};
}

const WIRE_VISIBILITY: Record<IAppVisibility, string> = {
	[IAppVisibility.Offline]: "OFFLINE",
	[IAppVisibility.Private]: "PRIVATE",
	[IAppVisibility.Prototype]: "PROTOTYPE",
	[IAppVisibility.Public]: "PUBLIC",
	[IAppVisibility.PublicRequestAccess]: "PUBLIC_REQUEST_ACCESS",
};

/**
 * Suite endpoints speak SCREAMING_SNAKE while the app enum is PascalCase —
 * `PublicRequestAccess` would otherwise be rejected by the strict parser.
 */
export function toWireVisibility(visibility: IAppVisibility): string {
	return WIRE_VISIBILITY[visibility] ?? "PRIVATE";
}

export function fromWireVisibility(value?: string | null): IAppVisibility {
	switch ((value ?? "").toUpperCase()) {
		case "PUBLIC":
			return IAppVisibility.Public;
		case "PUBLIC_REQUEST_ACCESS":
			return IAppVisibility.PublicRequestAccess;
		case "PROTOTYPE":
			return IAppVisibility.Prototype;
		case "OFFLINE":
			return IAppVisibility.Offline;
		default:
			return IAppVisibility.Private;
	}
}
