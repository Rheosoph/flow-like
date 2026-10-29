import type { ComponentType } from "react";
import type { ComponentProps } from "../../packages/ui/components/a2ui/ComponentRegistry";
import {
	A2UITable,
	A2UITableCell,
	A2UITableRow,
} from "../../packages/ui/components/a2ui/display/Table";
import { A2UIText } from "../../packages/ui/components/a2ui/display/Text";
import { A2UIWidgetInstance } from "../../packages/ui/components/a2ui/layout/A2UIWidgetInstance";
import { A2UIColumn } from "../../packages/ui/components/a2ui/layout/Column";

export type {
	ComponentProps,
	RenderChildFn,
} from "../../packages/ui/components/a2ui/ComponentRegistry";

// Restrict registration to this fixture's types while using their production renderers.
const renderers: Record<string, ComponentType<ComponentProps>> = {
	column: A2UIColumn as ComponentType<ComponentProps>,
	text: A2UIText as ComponentType<ComponentProps>,
	table: A2UITable as ComponentType<ComponentProps>,
	tableRow: A2UITableRow as ComponentType<ComponentProps>,
	tableCell: A2UITableCell as ComponentType<ComponentProps>,
	widgetInstance: A2UIWidgetInstance as ComponentType<ComponentProps>,
};

export const getComponentRenderer = (type: string) => renderers[type];
export const getRegisteredTypes = () => Object.keys(renderers);
