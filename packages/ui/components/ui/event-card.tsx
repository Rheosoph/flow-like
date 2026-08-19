"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Activity,
	Calendar,
	Clock,
	Edit2,
	ExternalLinkIcon,
	Hash,
	Sparkles,
	Trash2,
	Zap,
} from "lucide-react";
import type { IEvent } from "../../lib";
import { Badge } from "./badge";
import { Button } from "./button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "./card";

interface EventCardProps {
	event: IEvent;
	onEdit: () => void;
	onDelete: () => void;
	navigateToNode: (nodeId: string) => void;
}

export function EventCard({
	event,
	onEdit,
	onDelete,
	navigateToNode,
}: Readonly<EventCardProps>) {
	const { t } = useTranslation("common");
	const formatDate = (systemTime: { secs_since_epoch: number }) => {
		return new Date(systemTime.secs_since_epoch * 1000).toLocaleDateString();
	};

	const getEventTypeFromBoardId = (boardId: string) => {
		if (boardId.includes("chat"))
			return { type: "Chat", icon: <Hash className="h-3 w-3" /> };
		if (boardId.includes("mail"))
			return { type: "Mail", icon: <Calendar className="h-3 w-3" /> };
		if (boardId.includes("api"))
			return { type: "API", icon: <Zap className="h-3 w-3" /> };
		return { type: "Generic", icon: <Activity className="h-3 w-3" /> };
	};

	const eventType = getEventTypeFromBoardId(event.board_id);

	return (
		<Card className="relative group transition-all duration-300 hover:scale-[1.03] bg-card">
			{/* Gradient overlay for active events */}
			{event.active && (
				<div className="absolute inset-0 bg-linear-to-r from-primary/5 via-transparent to-primary/5 opacity-0 group-hover:opacity-100 transition-opacity duration-300" />
			)}

			<div className="relative">
				<CardHeader className="pb-4">
					<div className="flex items-start justify-between">
						<div className="flex items-center gap-3">
							<Badge
								variant="outline"
								className="gap-2 font-medium bg-muted/50 hover:bg-muted transition-colors"
							>
								{eventType.icon}
								{eventType.type}
							</Badge>
							<Badge
								variant={event.active ? "default" : "secondary"}
								className={`gap-2 transition-all duration-200 ${
									event.active ? "bg-primary" : "bg-muted"
								}`}
							>
								<div
									className={`w-2 h-2 rounded-full transition-all duration-200 ${
										event.active
											? "bg-primary-foreground animate-pulse duration-1000"
											: "bg-muted-foreground"
									}`}
								/>
								{event.active ? "Active" : "Inactive"}
							</Badge>
						</div>
						{event.active && (
							<Sparkles className="h-4 w-4 text-primary opacity-60 animate-pulse" />
						)}
					</div>

					<CardTitle className="text-xl font-bold pt-2 bg-linear-to-r from-foreground to-foreground/80 bg-clip-text">
						{event.name}
					</CardTitle>
					<CardDescription className="leading-relaxed text-muted-foreground/90">
						{event.description || t('noDescriptionProvided', 'No description provided')}
					</CardDescription>
				</CardHeader>

				<CardContent className="space-y-6">
					{/* Metadata section */}
					<div className="flex items-center justify-between p-3 rounded-lg bg-muted/20 border">
						<div className="flex items-center gap-4">
							<div className="flex items-center gap-2 text-sm">
								<Activity className="h-4 w-4 text-primary" />
								<Badge variant="outline" className="font-mono text-xs">
									v{event.event_version.join(".")}
								</Badge>
							</div>
							<div className="w-px h-4 bg-border" />
							<div className="flex items-center gap-2 text-sm text-muted-foreground">
								<Clock className="h-4 w-4" />
								<span className="text-xs">{formatDate(event.updated_at)}</span>
							</div>
						</div>
					</div>

					{/* Action buttons with enhanced styling */}
					<div className="overflow-hidden transition-all duration-300 group-hover:max-h-16 max-h-0">
						<div className="flex items-center justify-between pt-4 border-t opacity-0 group-hover:opacity-100 transition-opacity duration-300 delay-75">
							<div className="flex items-center gap-2">
								<Button
									variant="outline"
									size="sm"
									onClick={onEdit}
									className="gap-2 hover:bg-primary/10 hover:border-primary/30 hover:text-primary transition-all duration-200"
								>
									<Edit2 className="h-3 w-3" />
									{t('edit', 'Edit')}
								</Button>
								<Button
									variant="destructive"
									size="sm"
									onClick={onDelete}
									className="gap-2"
								>
									<Trash2 className="h-3 w-3" />
									{t('delete', 'Delete')}
								</Button>
							</div>
							<Button
								variant="outline"
								size="icon"
								onClick={() => navigateToNode(event.node_id)}
							>
								<ExternalLinkIcon className="h-3 w-3" />
							</Button>
						</div>
					</div>
				</CardContent>
			</div>
		</Card>
	);
}
