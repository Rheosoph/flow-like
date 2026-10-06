"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowRight } from "lucide-react";
import { DvButton } from "../../../../settings/devices/primitives/dv-button";
import { cx } from "../../../../settings/devices/primitives/tone";
import type { RouteNav } from "../../contracts";

/** The button of the centred card (canvas `ctl.go`): 36 px high, 14 px at the sides, 13.5 px text. */
const CENTERED_BUTTON = "h-9 gap-1.75 px-3.5 text-[13.5px] has-[>svg]:px-3.5";

/** "Go to Support chat": the routes the form is configured with, offered after a finished run (through the leave guard). */
export function RoutesRow({
	routes,
	list,
	touch,
	centered,
}: Readonly<{
	routes: RouteNav;
	list: readonly string[];
	touch: boolean;
	centered: boolean;
}>) {
	const { t } = useTranslation("interfaces");
	if (list.length === 0) return null;
	return (
		<div
			className={cx(
				"flex flex-wrap gap-2",
				centered ? "mt-1 justify-center" : "pt-0.5",
			)}
		>
			{list.map((route) => (
				<DvButton
					key={route}
					size="md"
					className={cx(
						"whitespace-nowrap",
						centered && CENTERED_BUTTON,
						touch && "h-11 px-3.5",
					)}
					onClick={() => routes.go(route)}
				>
					{t("workbench.stage.routes.go", "Go to {{label}}", {
						label: routes.labels[route] ?? route,
					})}
					<ArrowRight aria-hidden className="size-3.5" />
				</DvButton>
			))}
		</div>
	);
}
