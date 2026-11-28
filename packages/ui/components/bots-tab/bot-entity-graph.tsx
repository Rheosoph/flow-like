"use client";

import { useTranslation } from "@flow-like/locales";
import { useMemo } from "react";
import { cn } from "../../lib/utils";
import type { BotEntity } from "../../state/global-chat/bot-memory-facts";
import {
	type EgoGraph,
	edgeWidth,
	entityKey,
	entityLabel,
	layoutEgo,
} from "./bot-memory-entities-model";
import { entityStyle } from "./bot-memory-entity-kinds";

const NODE_RADIUS = 19;
const CENTER_RADIUS = 25;

function clip(text: string, length: number) {
	return text.length > length ? `${text.slice(0, length - 1)}…` : text;
}

function EntityGlyph({
	entity,
	x,
	y,
	radius,
}: Readonly<{ entity: BotEntity; x: number; y: number; radius: number }>) {
	const Icon = entityStyle(entity).icon;
	const size = radius * 0.95;
	return (
		<Icon
			x={x - size / 2}
			y={y - size / 2}
			width={size}
			height={size}
			strokeWidth={1.6}
			aria-hidden
		/>
	);
}

/** The selected entity in the middle, the entities its facts connect it to around it. */
export function BotEntityGraph({
	center,
	graph,
	onSelect,
}: Readonly<{
	center: BotEntity;
	graph: EgoGraph;
	onSelect: (entity: BotEntity) => void;
}>) {
	const { t } = useTranslation("bots");
	const you = t("memory.entities.you", "You");
	const layout = useMemo(
		() => layoutEgo(graph.neighbours.length),
		[graph.neighbours.length],
	);
	const centerLabel = entityLabel(center, you);

	return (
		<figure className="min-w-0">
			<svg
				viewBox={`0 0 ${layout.width} ${layout.height}`}
				// biome-ignore lint/a11y/useSemanticElements: a <fieldset> cannot be an SVG child
				role="group"
				aria-label={t("memory.entities.graph", "Connections of {{name}}", {
					name: centerLabel,
				})}
				className="mx-auto block h-auto max-h-72 w-full max-w-md"
			>
				<defs>
					<marker
						id="entity-arrow"
						viewBox="0 0 8 8"
						refX="6.5"
						refY="4"
						markerWidth="6"
						markerHeight="6"
						orient="auto-start-reverse"
					>
						<path d="M0 0 L8 4 L0 8 z" className="fill-muted-foreground/60" />
					</marker>
				</defs>
				{graph.neighbours.map(({ entity, edge }, index) => {
					const node = layout.nodes[index];
					const angle = Math.atan2(
						node.y - layout.center.y,
						node.x - layout.center.x,
					);
					const [from, to] = edge.outgoing
						? [
								[
									layout.center.x + Math.cos(angle) * CENTER_RADIUS,
									layout.center.y + Math.sin(angle) * CENTER_RADIUS,
								],
								[
									node.x - Math.cos(angle) * (NODE_RADIUS + 3),
									node.y - Math.sin(angle) * (NODE_RADIUS + 3),
								],
							]
						: [
								[
									node.x - Math.cos(angle) * (NODE_RADIUS + 3),
									node.y - Math.sin(angle) * (NODE_RADIUS + 3),
								],
								[
									layout.center.x + Math.cos(angle) * (CENTER_RADIUS + 3),
									layout.center.y + Math.sin(angle) * (CENTER_RADIUS + 3),
								],
							];
					const midX = (layout.center.x + node.x) / 2;
					const midY = (layout.center.y + node.y) / 2;
					return (
						<g key={entityKey(entity)}>
							<line
								x1={from[0]}
								y1={from[1]}
								x2={to[0]}
								y2={to[1]}
								strokeWidth={edgeWidth(edge.count)}
								strokeLinecap="round"
								markerEnd="url(#entity-arrow)"
								className="stroke-border"
							/>
							{edge.relation && (
								<text
									x={midX}
									y={midY}
									textAnchor="middle"
									dominantBaseline="central"
									paintOrder="stroke"
									strokeWidth={4}
									strokeLinejoin="round"
									className="fill-muted-foreground stroke-card text-[9px]"
								>
									{clip(edge.relation, 16)}
								</text>
							)}
						</g>
					);
				})}
				{graph.neighbours.map(({ entity, edge }, index) => {
					const node = layout.nodes[index];
					const style = entityStyle(entity);
					const label = entityLabel(entity, you);
					return (
						<g
							key={entityKey(entity)}
							// biome-ignore lint/a11y/useSemanticElements: an SVG node cannot be a <button>
							role="button"
							tabIndex={0}
							aria-label={t("memory.entities.openNeighbour", "Open {{name}}", {
								name: label,
							})}
							onClick={() => onSelect(entity)}
							onKeyDown={(event) => {
								if (event.key === "Enter" || event.key === " ") {
									event.preventDefault();
									onSelect(entity);
								}
							}}
							className="cursor-pointer outline-none [&:focus-visible>circle]:stroke-ring [&:hover>circle]:stroke-foreground/40"
						>
							<circle
								cx={node.x}
								cy={node.y}
								r={NODE_RADIUS}
								strokeWidth={1.5}
								className="fill-card stroke-border transition-colors"
							/>
							<g className={style.accent}>
								<EntityGlyph
									entity={entity}
									x={node.x}
									y={node.y}
									radius={NODE_RADIUS}
								/>
							</g>
							<text
								x={node.x}
								y={
									node.y < layout.center.y - 1
										? node.y - NODE_RADIUS - 5
										: node.y + NODE_RADIUS + 11
								}
								textAnchor="middle"
								className="fill-foreground text-[10px] font-medium"
							>
								{clip(label, 14)}
							</text>
						</g>
					);
				})}
				<g aria-hidden>
					<circle
						cx={layout.center.x}
						cy={layout.center.y}
						r={CENTER_RADIUS}
						strokeWidth={2}
						className="fill-primary/10 stroke-primary"
					/>
					<g className="text-primary">
						<EntityGlyph
							entity={center}
							x={layout.center.x}
							y={layout.center.y}
							radius={CENTER_RADIUS}
						/>
					</g>
					<text
						x={layout.center.x}
						y={layout.center.y + CENTER_RADIUS + 12}
						textAnchor="middle"
						paintOrder="stroke"
						strokeWidth={4}
						strokeLinejoin="round"
						className="fill-foreground stroke-card text-[11px] font-semibold"
					>
						{clip(centerLabel, 18)}
					</text>
				</g>
			</svg>
			{graph.hidden > 0 && (
				<figcaption
					className={cn("mt-1 text-center text-[11px] text-muted-foreground")}
				>
					{t("memory.entities.moreConnections", "+{{count}} more connections", {
						count: graph.hidden,
						defaultValue_one: "+{{count}} more connection",
					})}
				</figcaption>
			)}
		</figure>
	);
}
