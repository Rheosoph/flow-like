"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowBigLeftDashIcon, HouseIcon, WorkflowIcon } from "lucide-react";
import { memo } from "react";
import { useInvoke } from "../../../hooks/use-invoke";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";

/**
 * Where the board goes when you leave it.
 *
 * The board owns the window, so the global sidebar — which held the only Home
 * row and the product's only cross-app board switcher (`getOpenBoards`) — is not
 * mounted here. Both live in this popover instead. It stays navigation: growing
 * it into a copy of the sidebar would rebuild inside the board the thing the
 * board removed.
 */
export const BoardNavMenu = memo(function BoardNavMenu({
	appHref,
	boardParent,
	boardId,
	onNavigate,
}: Readonly<{
	appHref: string;
	/** The route the board was opened from, when one registered itself. */
	boardParent?: string;
	boardId: string;
	onNavigate: (href: string) => void;
}>) {
	const { t } = useTranslation("flow");
	const backend = useBackend();
	const openBoards = useInvoke(
		backend.boardState.getOpenBoards,
		backend.boardState,
		[],
	);
	const others = openBoards.data?.filter(([, id]) => id !== boardId) ?? [];

	return (
		<div className="flex flex-col gap-1">
			<NavRow
				icon={<HouseIcon />}
				label={t("appFlows", "App flows")}
				onSelect={() => onNavigate(appHref)}
			/>
			{boardParent && boardParent !== appHref && (
				<NavRow
					icon={<ArrowBigLeftDashIcon />}
					label={t("backToApp", "Back to app")}
					onSelect={() => onNavigate(boardParent)}
				/>
			)}
			{others.length > 0 && (
				<>
					<span className="mt-1 px-2 text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
						{t("openFlows", "Open Flows")}
					</span>
					{others.map(([appId, id, name]) => (
						<NavRow
							key={id}
							icon={<WorkflowIcon />}
							label={name}
							onSelect={() => onNavigate(`/flow?id=${id}&app=${appId}`)}
						/>
					))}
				</>
			)}
		</div>
	);
});

const NavRow = memo(function NavRow({
	icon,
	label,
	onSelect,
}: Readonly<{ icon: React.ReactNode; label: string; onSelect: () => void }>) {
	return (
		<button
			type="button"
			onClick={onSelect}
			className={cn(
				"flex w-full items-center gap-2 rounded-sm px-2 py-1.5 text-left text-xs",
				"hover:bg-accent hover:text-accent-foreground",
			)}
		>
			<span className="[&>svg]:size-3.5 shrink-0 text-muted-foreground">
				{icon}
			</span>
			<span className="truncate">{label}</span>
		</button>
	);
});
