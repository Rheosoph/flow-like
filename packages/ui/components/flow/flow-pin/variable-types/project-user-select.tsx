import { ChevronDown } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useBackend } from "../../../..";
import {
	Select,
	SelectContent,
	SelectGroup,
	SelectItem,
	SelectLabel,
	SelectTrigger,
} from "../../../../components/ui/select";
import type { IPin } from "../../../../lib/schema/flow/pin";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../../../lib/uint8";
import {
	userDisplayName,
	userSecondaryLabel,
} from "../../../../lib/user-display";
import type {
	IMember,
	IUserLookup,
} from "../../../../state/backend-state/types";

const TEAM_PAGE_SIZE = 100;
const MAX_TEAM_MEMBERS = 10000;

function normalizeSub(value: number[] | undefined | null): string {
	const parsed = parseUint8ArrayToJson(value);
	return typeof parsed === "string" ? parsed : "";
}

/** Falls back to the raw sub: picking a user by id stays a valid affordance here. */
function getUserDisplayName(
	user: IUserLookup | undefined,
	sub: string,
): string {
	return userDisplayName(user, sub || "Unknown User");
}

function getUserSecondaryLabel(
	user: IUserLookup | undefined,
	sub: string,
): string {
	return userSecondaryLabel(user) ?? sub;
}

export function ProjectUserSelect({
	pin,
	value,
	appId,
	setValue,
}: Readonly<{
	pin: IPin;
	value: number[] | undefined | null;
	appId: string;
	setValue: (value: number[] | undefined) => void;
}>) {
	const backend = useBackend();
	const [open, setOpen] = useState(false);
	const [members, setMembers] = useState<IMember[]>([]);
	const [usersBySub, setUsersBySub] = useState<Record<string, IUserLookup>>({});
	const [loading, setLoading] = useState(false);
	const [error, setError] = useState(false);
	const selectedSub = normalizeSub(value);

	useEffect(() => {
		if (!appId || (!open && !selectedSub)) return;

		let cancelled = false;

		async function loadProjectUsers() {
			setLoading(true);
			setError(false);

			try {
				const loadedMembers: IMember[] = [];
				let offset = 0;

				for (;;) {
					const page = await backend.teamState.getTeam(
						appId,
						offset,
						TEAM_PAGE_SIZE,
					);

					if (cancelled) return;

					loadedMembers.push(...page);

					if (page.length < TEAM_PAGE_SIZE) break;
					offset += TEAM_PAGE_SIZE;
					if (offset >= MAX_TEAM_MEMBERS) break;
				}

				const uniqueSubs = [
					...new Set(
						loadedMembers
							.map((member) => member.user_id)
							.concat(selectedSub ? [selectedSub] : [])
							.filter(Boolean),
					),
				];

				const lookupEntries = await Promise.all(
					uniqueSubs.map(async (sub) => {
						try {
							return [sub, await backend.userState.lookupUser(sub)] as const;
						} catch {
							return [sub, undefined] as const;
						}
					}),
				);

				if (cancelled) return;

				const nextUsersBySub: Record<string, IUserLookup> = {};
				for (const [sub, user] of lookupEntries) {
					if (user) nextUsersBySub[sub] = user;
				}

				setMembers(loadedMembers);
				setUsersBySub(nextUsersBySub);
			} catch {
				if (!cancelled) setError(true);
			} finally {
				if (!cancelled) setLoading(false);
			}
		}

		void loadProjectUsers();

		return () => {
			cancelled = true;
		};
	}, [appId, backend.teamState, backend.userState, open, selectedSub]);

	const projectMembers = useMemo(() => {
		const seen = new Set<string>();
		return members.filter((member) => {
			if (!member.user_id || seen.has(member.user_id)) return false;
			seen.add(member.user_id);
			return true;
		});
	}, [members]);

	const handleOpenChange = useCallback((isOpen: boolean) => {
		setOpen(isOpen);
	}, []);

	return (
		<div
			className="flex flex-row items-center justify-start max-w-full ml-1 overflow-hidden"
			onMouseDown={(e) => e.stopPropagation()}
			onPointerDown={(e) => e.stopPropagation()}
		>
			<Select
				open={open}
				onOpenChange={handleOpenChange}
				value={selectedSub || undefined}
				onValueChange={(sub) => setValue(convertJsonToUint8Array(sub))}
			>
				<SelectTrigger
					noChevron
					size="sm"
					className="w-fit! max-w-full! p-0 border-0 text-xs bg-card! text-start max-h-fit h-4 gap-0.5 flex-row items-center overflow-hidden"
				>
					<small className="text-start text-[10px] m-0! truncate">
						{selectedSub
							? getUserDisplayName(usersBySub[selectedSub], selectedSub)
							: "Select user"}
					</small>
					<ChevronDown className="size-2 min-w-2 min-h-2 text-card-foreground shrink-0" />
				</SelectTrigger>
				<SelectContent>
					<SelectGroup>
						<SelectLabel>{pin.friendly_name}</SelectLabel>
						{loading && projectMembers.length === 0 && (
							<SelectLabel>Loading users...</SelectLabel>
						)}
						{error && <SelectLabel>Could not load project users</SelectLabel>}
						{!loading && !error && projectMembers.length === 0 && (
							<SelectLabel>No project users found</SelectLabel>
						)}
						{projectMembers.map((member) => {
							const user = usersBySub[member.user_id];
							return (
								<SelectItem key={member.id} value={member.user_id}>
									<div className="flex min-w-0 flex-col items-start gap-0">
										<span className="max-w-48 truncate">
											{getUserDisplayName(user, member.user_id)}
										</span>
										<span className="max-w-48 truncate text-xs text-muted-foreground">
											{getUserSecondaryLabel(user, member.user_id)}
										</span>
									</div>
								</SelectItem>
							);
						})}
					</SelectGroup>
				</SelectContent>
			</Select>
		</div>
	);
}
