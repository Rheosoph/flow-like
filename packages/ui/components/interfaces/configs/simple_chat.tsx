"use client";

import { useMemo } from "react";
import { useInvoke } from "../../../hooks/use-invoke";
import { useBackend } from "../../../state/backend-state";
import type { IRouteMapping } from "../../../state/backend-state/route-state";
import { Checkbox, Input, Label, ScrollArea, Switch } from "../../ui";
import type { IConfigInterfaceProps } from "../interfaces";

export function SimpleChatConfig({
	isEditing,
	appId,
	boardId,
	config,
	nodeId,
	node,
	onConfigUpdate,
}: IConfigInterfaceProps) {
	const backend = useBackend();
	const routesQuery = useInvoke<IRouteMapping[], [appId: string]>(
		backend.routeState.getRoutes,
		backend.routeState,
		[appId],
		!!appId,
		[appId],
	);

	const routes = useMemo(() => {
		const list = routesQuery.data ?? [];
		return list.slice().sort((a, b) => a.path.localeCompare(b.path));
	}, [routesQuery.data]);

	const setValue = (key: string, value: any, deleteKeys: string[] = []) => {
		if (!onConfigUpdate) return;
		const next = { ...(config as any) };
		for (const k of deleteKeys) {
			delete next[k];
		}
		next[key] = value;
		onConfigUpdate(next);
	};

	const normalizeRoute = (value: string): string => {
		const trimmed = value.trim();
		if (!trimmed) return "";
		return trimmed.startsWith("/") ? trimmed : `/${trimmed}`;
	};

	const selectedRoutes = useMemo(() => {
		const rawArray = (config as any)?.navigate_to_routes;
		const raw: string[] = Array.isArray(rawArray) ? rawArray : [];
		const normalized = raw
			.map((r) => normalizeRoute(String(r)))
			.filter((r) => !!r);
		return Array.from(new Set(normalized));
	}, [config]);

	return (
		<div className="w-full space-y-6">
			<div className="space-y-3">
				<Label>Navigate To</Label>
				{isEditing ? (
					<div className="rounded-md border border-input bg-background">
						<ScrollArea className="max-h-48">
							<div className="p-2 space-y-1">
								{routesQuery.isLoading ? (
									<div className="px-2 py-2 text-sm text-muted-foreground">
										Loading routes…
									</div>
								) : routes.length === 0 ? (
									<div className="px-2 py-2 text-sm text-muted-foreground">
										No routes configured.
									</div>
								) : (
									routes
										.filter(
											(r) => typeof r.path === "string" && r.path.length > 0,
										)
										.map((route) => {
											const checked = selectedRoutes.includes(route.path);
											const label = route.path;
											return (
												<label
													key={route.path}
													className="flex items-center gap-2 px-2 py-1.5 rounded-md hover:bg-muted cursor-pointer"
												>
													<Checkbox
														checked={checked}
														onCheckedChange={(nextChecked) => {
															const normalized = normalizeRoute(route.path);
															const next = new Set(selectedRoutes);
															if (nextChecked) {
																next.add(normalized);
															} else {
																next.delete(normalized);
															}
															const nextArr = Array.from(next).filter(Boolean);
															setValue(
																"navigate_to_routes",
																nextArr.length > 0 ? nextArr : null,
															);
														}}
													/>
													<span className="text-sm">{label}</span>
												</label>
											);
										})
								)}
							</div>
						</ScrollArea>
						<div className="border-t border-input p-2">
							<button
								type="button"
								className="text-xs text-muted-foreground hover:text-foreground"
								onClick={() => setValue("navigate_to_routes", null)}
							>
								Clear selection
							</button>
						</div>
					</div>
				) : (
					<div className="flex min-h-10 w-full rounded-md border border-input bg-muted px-3 py-2 text-sm">
						{selectedRoutes.length > 0
							? selectedRoutes.join(", ")
							: "No destinations"}
					</div>
				)}
				<p className="text-sm text-muted-foreground">
					Optional route destinations this chat can navigate to.
				</p>
			</div>

			<div className="space-y-4">
				<div className="flex items-center space-x-2">
					<Switch
						disabled={!isEditing}
						id="allow_file_upload"
						checked={config?.allow_file_upload ?? true}
						onCheckedChange={(checked) => {
							setValue("allow_file_upload", checked);
						}}
					/>
					<Label htmlFor="allow_file_upload">Allow File Upload</Label>
				</div>
				<p className="text-sm text-muted-foreground">
					Enable users to upload files during chat conversations
				</p>
			</div>

			<div className="space-y-4">
				<div className="flex items-center space-x-2">
					<Switch
						disabled={!isEditing}
						id="allow_voice_input"
						checked={config?.allow_voice_input ?? false}
						onCheckedChange={(checked) => {
							setValue("allow_voice_input", checked);
						}}
					/>
					<Label htmlFor="allow_voice_input">Allow Voice Input</Label>
				</div>
				<p className="text-sm text-muted-foreground">
					Enable users to use voice input for chat messages
				</p>
			</div>

			<div className="space-y-4">
				<div className="flex items-center space-x-2">
					<Switch
						disabled={!isEditing}
						id="allow_voice_mode"
						checked={config?.allow_voice_mode ?? false}
						onCheckedChange={(checked) => {
							setValue("allow_voice_mode", checked);
						}}
					/>
					<Label htmlFor="allow_voice_mode">Allow Voice Mode</Label>
				</div>
				<p className="text-sm text-muted-foreground">
					Enable voice mode with auto silence detection, similar to OpenAI voice
					mode
				</p>
			</div>

			<div className="space-y-3">
				<Label htmlFor="history_elements">History Elements</Label>
				{isEditing ? (
					<Input
						value={config?.history_elements ?? 5}
						onChange={(e) => {
							const value = e.target.value
								? Number.parseInt(e.target.value, 10)
								: 5;
							setValue("history_elements", value);
						}}
						type="number"
						id="history_elements"
						placeholder="5"
						min="1"
						max="100"
					/>
				) : (
					<div className="flex h-10 w-full rounded-md border border-input bg-muted px-3 py-2 text-sm">
						{config?.history_elements ?? 5}
					</div>
				)}
				<p className="text-sm text-muted-foreground">
					Number of previous messages to include in chat context
				</p>
			</div>

			<div className="space-y-3">
				<Label htmlFor="tools">Available Tools</Label>
				{isEditing ? (
					<div className="space-y-2">
						<div className="flex flex-wrap gap-2">
							{(config?.tools ?? []).map((tool, index) => (
								<div
									key={index + tool}
									className="inline-flex items-center gap-1 bg-secondary text-secondary-foreground px-2 py-1 rounded-md text-sm"
								>
									<span>{tool}</span>
									<button
										type="button"
										onClick={() => {
											const newTools = [...(config?.tools ?? [])];
											newTools.splice(index, 1);
											setValue("tools", newTools);
										}}
										className="text-secondary-foreground/70 hover:text-secondary-foreground"
									>
										×
									</button>
								</div>
							))}
						</div>
						<Input
							placeholder="Type a tool name and press Enter"
							onKeyDown={(e) => {
								if (e.key === "Enter" && e.currentTarget.value.trim()) {
									e.preventDefault();
									const newTool = e.currentTarget.value.trim();
									const currentTools = config?.tools ?? [];
									if (!currentTools.includes(newTool)) {
										setValue("tools", [...currentTools, newTool]);
									}
									e.currentTarget.value = "";
								}
							}}
						/>
					</div>
				) : (
					<div className="space-y-2">
						{(config?.tools ?? []).length > 0 ? (
							<div className="flex flex-wrap gap-2">
								{(config?.tools ?? []).map((tool, index) => (
									<div
										key={index + tool}
										className="inline-flex items-center bg-muted text-muted-foreground px-2 py-1 rounded-md text-sm"
									>
										{tool}
									</div>
								))}
							</div>
						) : (
							<div className="flex h-10 w-full rounded-md border border-input bg-muted px-3 py-2 text-sm">
								No tools configured
							</div>
						)}
					</div>
				)}
				<p className="text-sm text-muted-foreground">
					Tools available for this chat. Press Enter to add a new tool.
				</p>
			</div>

			<div className="space-y-3">
				<Label htmlFor="default_tools">Default Tools</Label>
				{isEditing ? (
					<div className="space-y-2">
						<div className="flex flex-wrap gap-2">
							{(config?.default_tools ?? []).map((tool, index) => (
								<div
									key={index + tool}
									className="inline-flex items-center gap-1 bg-primary text-primary-foreground px-2 py-1 rounded-md text-sm"
								>
									<span>{tool}</span>
									<button
										type="button"
										onClick={() => {
											const newTools = [...(config?.default_tools ?? [])];
											newTools.splice(index, 1);
											setValue("default_tools", newTools);
										}}
										className="text-primary-foreground/70 hover:text-primary-foreground"
									>
										×
									</button>
								</div>
							))}
						</div>
						<Input
							placeholder="Type a tool name and press Enter"
							onKeyDown={(e) => {
								if (e.key === "Enter" && e.currentTarget.value.trim()) {
									e.preventDefault();
									const newTool = e.currentTarget.value.trim();
									const currentTools = config?.default_tools ?? [];
									if (!currentTools.includes(newTool)) {
										setValue("default_tools", [...currentTools, newTool]);
									}
									e.currentTarget.value = "";
								}
							}}
						/>
					</div>
				) : (
					<div className="space-y-2">
						{(config?.default_tools ?? []).length > 0 ? (
							<div className="flex flex-wrap gap-2">
								{(config?.default_tools ?? []).map((tool, index) => (
									<div
										key={index + tool}
										className="inline-flex items-center bg-muted text-muted-foreground px-2 py-1 rounded-md text-sm"
									>
										{tool}
									</div>
								))}
							</div>
						) : (
							<div className="flex h-10 w-full rounded-md border border-input bg-muted px-3 py-2 text-sm">
								No default tools
							</div>
						)}
					</div>
				)}
				<p className="text-sm text-muted-foreground">
					Tools enabled by default. Press Enter to add a new tool.
				</p>
			</div>

			<div className="space-y-3">
				<Label htmlFor="example_messages">Example Messages</Label>
				{isEditing ? (
					<div className="space-y-2">
						<div className="flex flex-wrap gap-2">
							{(config?.example_messages ?? []).map((message, index) => (
								<div
									key={index + message}
									className="inline-flex items-center gap-1 bg-secondary text-secondary-foreground px-2 py-1 rounded-md text-sm max-w-xs"
								>
									<span className="truncate">{message}</span>
									<button
										type="button"
										onClick={() => {
											const newMessages = [...(config?.example_messages ?? [])];
											newMessages.splice(index, 1);
											setValue("example_messages", newMessages);
										}}
										className="text-secondary-foreground/70 hover:text-secondary-foreground shrink-0"
									>
										×
									</button>
								</div>
							))}
						</div>
						<Input
							placeholder="Type an example message and press Enter"
							onKeyDown={(e) => {
								if (e.key === "Enter" && e.currentTarget.value.trim()) {
									e.preventDefault();
									const newMessage = e.currentTarget.value.trim();
									const currentMessages = config?.example_messages ?? [];
									if (!currentMessages.includes(newMessage)) {
										setValue("example_messages", [
											...currentMessages,
											newMessage,
										]);
									}
									e.currentTarget.value = "";
								}
							}}
						/>
					</div>
				) : (
					<div className="space-y-2">
						{(config?.example_messages ?? []).length > 0 ? (
							<div className="flex flex-wrap gap-2">
								{(config?.example_messages ?? []).map((message, index) => (
									<div
										key={index + message}
										className="inline-flex items-center bg-muted text-muted-foreground px-2 py-1 rounded-md text-sm max-w-xs"
									>
										<span className="truncate">{message}</span>
									</div>
								))}
							</div>
						) : (
							<div className="flex h-10 w-full rounded-md border border-input bg-muted px-3 py-2 text-sm">
								No example messages
							</div>
						)}
					</div>
				)}
				<p className="text-sm text-muted-foreground">
					Example messages to show users. Press Enter to add a new message.
				</p>
			</div>
		</div>
	);
}
