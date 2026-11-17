import type { AppPointedAtKind } from "../pointed-at";
import { denTargetProps } from "./targets";

export interface AppDenThing {
	kind: AppPointedAtKind;
	appId?: string | null;
	id?: string | null;
	label?: string | null;
	boardId?: string | null;
}

/** Spread on the element of a thing inside an app; empty until the app, id and name are known. */
export function appDenProps({
	kind,
	appId,
	id,
	label,
	boardId,
}: AppDenThing): Record<string, string> {
	const name = label?.trim();
	if (!appId || !id || !name) return {};
	return denTargetProps({
		kind,
		app_id: appId,
		id,
		label: name,
		...(boardId ? { board_id: boardId } : {}),
	});
}
