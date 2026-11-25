import { useTranslation } from "@flow-like/locales";
import {
	Building2Icon,
	FolderKanbanIcon,
	type LucideIcon,
	MapPinIcon,
	PackageIcon,
	UserRoundIcon,
} from "lucide-react";
import { useCallback } from "react";
import type { BotEntity } from "../../state/global-chat/bot-memory-facts";

export interface EntityKindStyle {
	icon: LucideIcon;
	/** Tinted badge: background and text. */
	color: string;
	/** Text color for strokes and glyphs drawn in `currentColor`. */
	accent: string;
}

export type EntityKind =
	| "person"
	| "place"
	| "organization"
	| "project"
	| "thing";

const styles: Record<EntityKind, EntityKindStyle> = {
	person: {
		icon: UserRoundIcon,
		color: "bg-sky-500/10 text-sky-700 dark:text-sky-300",
		accent: "text-sky-600 dark:text-sky-400",
	},
	place: {
		icon: MapPinIcon,
		color: "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
		accent: "text-emerald-600 dark:text-emerald-400",
	},
	organization: {
		icon: Building2Icon,
		color: "bg-violet-500/10 text-violet-700 dark:text-violet-300",
		accent: "text-violet-600 dark:text-violet-400",
	},
	project: {
		icon: FolderKanbanIcon,
		color: "bg-amber-500/10 text-amber-700 dark:text-amber-300",
		accent: "text-amber-600 dark:text-amber-400",
	},
	thing: {
		icon: PackageIcon,
		color: "bg-rose-500/10 text-rose-700 dark:text-rose-300",
		accent: "text-rose-600 dark:text-rose-400",
	},
};

export function entityKindStyle(kind: string): EntityKindStyle {
	return Object.hasOwn(styles, kind)
		? styles[kind as EntityKind]
		: styles.thing;
}

export function entityStyle(entity: BotEntity): EntityKindStyle {
	return entity.owner
		? {
				...styles.person,
				color: "bg-primary/10 text-primary",
				accent: "text-primary",
			}
		: entityKindStyle(entity.kind);
}

export function useEntityKindLabel() {
	const { t } = useTranslation("bots");
	return useCallback(
		(kind: string) => {
			switch (kind) {
				case "person":
					return t("memory.entities.kinds.person", "People");
				case "place":
					return t("memory.entities.kinds.place", "Places");
				case "organization":
					return t("memory.entities.kinds.organization", "Organizations");
				case "project":
					return t("memory.entities.kinds.project", "Projects");
				default:
					return t("memory.entities.kinds.thing", "Things");
			}
		},
		[t],
	);
}

/** The singular name for one entity's kind, as in the kind select. */
export function useEntityKindName() {
	const { t } = useTranslation("bots");
	return useCallback(
		(kind: string) => {
			switch (kind) {
				case "person":
					return t("memory.entities.kindNames.person", "Person");
				case "place":
					return t("memory.entities.kindNames.place", "Place");
				case "organization":
					return t("memory.entities.kindNames.organization", "Organization");
				case "project":
					return t("memory.entities.kindNames.project", "Project");
				default:
					return t("memory.entities.kindNames.thing", "Thing");
			}
		},
		[t],
	);
}
