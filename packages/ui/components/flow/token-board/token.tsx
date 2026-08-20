"use client";

import { useDraggable } from "@dnd-kit/core";
import { useTranslation } from "@flow-like/locales";
import type { CSSProperties, PointerEvent as ReactPointerEvent } from "react";
import { useMemo, useRef } from "react";
import type { ILayer, IVariable } from "../../../lib/schema/flow/board";
import { IVariableType } from "../../../lib/schema/flow/node";
import { IValueType } from "../../../lib/schema/flow/pin";
import { cn } from "../../../lib/utils";
import type { IDraggable } from "./model";
import { TOKEN_GLYPH, containerFormClass, tokenColor, tokenInk } from "./model";

interface ITokenShellProps {
	className?: string;
	style?: CSSProperties;
	selected?: boolean;
	dead?: boolean;
	tint?: boolean;
	label: string;
	title: string;
	tabIndex: number;
	itemId: string;
	onOpen: () => void;
	children: React.ReactNode;
	draggable: {
		setNodeRef: (element: HTMLElement | null) => void;
		listeners?: IDraggable["listeners"];
		attributes: IDraggable["attributes"];
		isDragging: boolean;
	};
}

function TokenShell({
	className,
	style,
	selected,
	dead,
	tint = true,
	label,
	title,
	tabIndex,
	itemId,
	onOpen,
	children,
	draggable,
}: Readonly<ITokenShellProps>) {
	const { t } = useTranslation("flow");
	const shellRef = useRef<HTMLSpanElement | null>(null);

	return (
		<span
			ref={(element) => {
				shellRef.current = element;
				draggable.setNodeRef(element);
			}}
			data-token-id={itemId}
			className={cn(
				"fl-tok",
				className,
				!tint && "fl-tok--flat",
				dead && "fl-tok--dead",
				selected && "fl-tok--selected",
				draggable.isDragging && "fl-tok--dragging",
			)}
			style={style}
		>
			<button
				type="button"
				className="fl-tok-btn"
				aria-label={label}
				title={title}
				onClick={onOpen}
				{...draggable.attributes}
				{...draggable.listeners}
				// After the spread: dnd-kit sets its own tabIndex, and the board's
				// roving focus has to win or arrow keys land on every chip in turn.
				tabIndex={tabIndex}
			>
				<span className="fl-tok-in">{children}</span>
			</button>
			<button
				type="button"
				className="fl-tok-caret"
				aria-label={t("actionsForName", "Actions for {{name}}", {
					name: label,
				})}
				onPointerDown={(event: ReactPointerEvent) => event.stopPropagation()}
				onClick={(event) => {
					event.stopPropagation();
					// The caret opens the same menu as right-click: replaying the event on
					// the shell keeps one menu implementation instead of two.
					const rect = event.currentTarget.getBoundingClientRect();
					shellRef.current?.dispatchEvent(
						new MouseEvent("contextmenu", {
							bubbles: true,
							cancelable: true,
							clientX: rect.left,
							clientY: rect.bottom,
						}),
					);
				}}
			>
				⌄
			</button>
		</span>
	);
}

function TokenSlug({
	type,
	cell,
}: Readonly<{ type: IVariableType | "Function"; cell?: "key" }>) {
	const glyph = type === "Function" ? "ƒ" : (TOKEN_GLYPH[type] ?? "?");
	return (
		<span
			className="fl-tok-slug"
			data-wide={glyph.length > 1 ? "true" : "false"}
			data-role={cell}
			aria-hidden="true"
		>
			{glyph}
		</span>
	);
}

export interface IVariableTokenProps {
	variable: IVariable;
	uses: number;
	tint?: boolean;
	selected?: boolean;
	focused?: boolean;
	onOpen: () => void;
}

export function VariableToken({
	variable,
	uses,
	tint = true,
	selected,
	focused,
	onOpen,
}: Readonly<IVariableTokenProps>) {
	const { t } = useTranslation("flow");
	const { setNodeRef, listeners, attributes, isDragging } = useDraggable({
		id: variable.id,
		data: variable,
	});

	const flags = useMemo(() => {
		const out: Array<{ mark: string; title: string }> = [];
		if (variable.exposed)
			out.push({
				mark: "E",
				title: t(
					"exposedInTheAppConfiguration",
					"Exposed in the app configuration",
				),
			});
		if (variable.secret)
			out.push({
				mark: "S",
				title: t("secretValueMasked", "Secret — value masked"),
			});
		if (variable.runtime_configured)
			out.push({
				mark: "R",
				title: t(
					"runtimeConfiguredPerUser",
					"Runtime-configured, set per user",
				),
			});
		if (!variable.editable)
			out.push({
				mark: "L",
				title: t(
					"readOnlySystemVariable",
					"Read-only — provided by the runtime",
				),
			});
		return out;
	}, [
		variable.exposed,
		variable.secret,
		variable.runtime_configured,
		variable.editable,
		t,
	]);

	const isMap = variable.value_type === IValueType.HashMap;
	const containerLabel =
		variable.value_type === IValueType.Normal
			? ""
			: ` ${String(variable.value_type)}`;

	return (
		<TokenShell
			className={containerFormClass(variable.value_type)}
			style={
				{
					"--fl-tok-color": tokenColor(variable.data_type),
					"--fl-tok-ink": tokenInk(variable.data_type),
					"--fl-tok-key-color": tokenColor(IVariableType.String),
					"--fl-tok-key-ink": tokenInk(IVariableType.String),
				} as CSSProperties
			}
			selected={selected}
			dead={uses === 0}
			tint={tint}
			label={`${variable.name} — ${variable.data_type}${containerLabel}`}
			title={t("nameTypeUses", "{{name}} · {{type}} · {{uses}} uses", {
				name: variable.name,
				type: `${variable.data_type}${containerLabel}`,
				uses,
			})}
			tabIndex={focused ? 0 : -1}
			itemId={variable.id}
			onOpen={onOpen}
			draggable={{ setNodeRef, listeners, attributes, isDragging }}
		>
			{isMap && <TokenSlug type={IVariableType.String} cell="key" />}
			<TokenSlug type={variable.data_type} />
			<span className="fl-tok-name">{variable.name}</span>
			{flags.length > 0 && (
				<span className="fl-tok-tail">
					{flags.map((flag) => (
						<b key={flag.mark} title={flag.title}>
							{flag.mark}
						</b>
					))}
				</span>
			)}
		</TokenShell>
	);
}

export interface IFunctionTokenProps {
	layer: ILayer;
	calls: number;
	inputs: number;
	outputs: number;
	tint?: boolean;
	selected?: boolean;
	focused?: boolean;
	onOpen: () => void;
}

export function FunctionToken({
	layer,
	calls,
	inputs,
	outputs,
	tint = true,
	selected,
	focused,
	onOpen,
}: Readonly<IFunctionTokenProps>) {
	const { t } = useTranslation("flow");
	const { setNodeRef, listeners, attributes, isDragging } = useDraggable({
		id: `function-${layer.id}`,
		data: { type: "function-layer", layerId: layer.id },
	});

	return (
		<TokenShell
			className="fl-tok--fn"
			style={
				{
					"--fl-tok-color": tokenColor("Function"),
					"--fl-tok-ink": tokenInk("Function"),
				} as CSSProperties
			}
			selected={selected}
			dead={calls === 0}
			tint={tint}
			label={t(
				"functionNameSignature",
				"{{name}} — function, {{in}} in {{out}} out",
				{
					name: layer.name,
					in: inputs,
					out: outputs,
				},
			)}
			title={t(
				"functionNameCalls",
				"{{name}} · {{in}} in / {{out}} out · {{calls}} call sites",
				{
					name: layer.name,
					in: inputs,
					out: outputs,
					calls,
				},
			)}
			tabIndex={focused ? 0 : -1}
			itemId={layer.id}
			onOpen={onOpen}
			draggable={{ setNodeRef, listeners, attributes, isDragging }}
		>
			<TokenSlug type="Function" />
			<span className="fl-tok-name">{layer.name}</span>
			<span className="fl-tok-tail">
				{inputs}→{outputs} <b>×{calls}</b>
				{layer.cache?.enabled ? (
					<b title={t("resultCached", "Result cached")}>⟳</b>
				) : null}
			</span>
		</TokenShell>
	);
}
