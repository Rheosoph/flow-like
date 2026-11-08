"use client";
import { useTranslation } from "@flow-like/locales";
import Link from "next/link";
import { toast } from "sonner";
import { useUserAvailability } from "../../hooks/use-user-availability";
import { AVAILABILITY_STATUSES } from "../../lib/availability";
import {
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuSub,
	DropdownMenuSubContent,
	DropdownMenuSubTrigger,
} from "../ui/dropdown-menu";
import { AvailabilityBadge } from "./availability-badge";

export function AccountAvailabilityMenu() {
	const { t } = useTranslation("bots");
	const availability = useUserAvailability();
	return (
		<DropdownMenuSub>
			<DropdownMenuSubTrigger>
				{availability.data ? (
					<AvailabilityBadge value={availability.data} />
				) : (
					t("availability.title", "Availability and focus")
				)}
			</DropdownMenuSubTrigger>
			<DropdownMenuSubContent>
				{AVAILABILITY_STATUSES.map((status) => (
					<DropdownMenuItem
						key={status}
						disabled={!availability.data}
						onSelect={() => {
							void availability
								.update({ status, duration_minutes: 60 })
								.catch(() =>
									toast.error(
										t(
											"availability.failed",
											"Could not update availability. Try again.",
										),
									),
								);
						}}
					>
						{t(`availability.status.${status}`, status)}{" "}
						<span className="ml-auto pl-4 text-xs text-muted-foreground">
							{t("availability.oneHour", "1 hour")}
						</span>
					</DropdownMenuItem>
				))}
				<DropdownMenuSeparator />
				<DropdownMenuItem asChild>
					<Link href="/account#availability">
						{t("availability.manage", "Manage availability and calendars")}
					</Link>
				</DropdownMenuItem>
			</DropdownMenuSubContent>
		</DropdownMenuSub>
	);
}

export function AccountAvailabilityDot() {
	const availability = useUserAvailability();
	return (
		<AvailabilityBadge
			value={availability.data}
			dot
			className="absolute -bottom-0.5 -right-0.5"
		/>
	);
}
