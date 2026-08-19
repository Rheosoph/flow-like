"use client";

import { useTranslation } from "@flow-like/locales";
import { Input, Label, Switch } from "../../ui";
import type { IConfigInterfaceProps } from "../interfaces";

export function WebhookConfig({
	isEditing,
	appId,
	boardId,
	config,
	nodeId,
	node,
	onConfigUpdate,
}: IConfigInterfaceProps) {
	const { t } = useTranslation("interfaces");
	const setValue = (key: string, value: any) => {
		if (onConfigUpdate) {
			onConfigUpdate({
				...config,
				[key]: value,
			});
		}
	};

	return (
		<div className="w-full space-y-6">
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
					<Label htmlFor="allow_file_upload">
						{t("allowFileUpload", "Allow File Upload")}
					</Label>
				</div>
				<p className="text-sm text-muted-foreground">
					{t(
						"enableUsersToUploadFilesDuringChatConversations",
						"Enable users to upload files during chat conversations",
					)}
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
					<Label htmlFor="allow_voice_input">
						{t("allowVoiceInput", "Allow Voice Input")}
					</Label>
				</div>
				<p className="text-sm text-muted-foreground">
					{t(
						"enableUsersToUseVoiceInputForChatMessages",
						"Enable users to use voice input for chat messages",
					)}
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
					<Label htmlFor="allow_voice_mode">
						{t("allowVoiceMode", "Allow Voice Mode")}
					</Label>
				</div>
				<p className="text-sm text-muted-foreground">
					{t(
						"enableVoiceModeWithAutoSilenceDetection",
						"Enable voice mode with auto silence detection",
					)}
				</p>
			</div>

			<div className="space-y-3">
				<Label htmlFor="history_elements">
					{t("historyElements", "History Elements")}
				</Label>
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
					{t(
						"numberOfPreviousMessagesToIncludeInChatContext",
						"Number of previous messages to include in chat context",
					)}
				</p>
			</div>

			<div className="space-y-3">
				<Label htmlFor="tools">{t("availableTools", "Available Tools")}</Label>
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
							placeholder={t(
								"typeAToolNameAndPressEnter",
								"Type a tool name and press Enter",
							)}
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
								{t("noToolsConfigured", "No tools configured")}
							</div>
						)}
					</div>
				)}
				<p className="text-sm text-muted-foreground">
					{`Tools available for this chat. Press Enter to add a new tool.`}
				</p>
			</div>

			<div className="space-y-3">
				<Label htmlFor="default_tools">
					{t("defaultTools", "Default Tools")}
				</Label>
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
							placeholder={t(
								"typeAToolNameAndPressEnter",
								"Type a tool name and press Enter",
							)}
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
								{t("noDefaultTools", "No default tools")}
							</div>
						)}
					</div>
				)}
				<p className="text-sm text-muted-foreground">
					{`Tools enabled by default. Press Enter to add a new tool.`}
				</p>
			</div>

			<div className="space-y-3">
				<Label htmlFor="example_messages">
					{t("exampleMessages", "Example Messages")}
				</Label>
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
							placeholder={t(
								"typeAnExampleMessageAndPressEnter",
								"Type an example message and press Enter",
							)}
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
								{t("noExampleMessages", "No example messages")}
							</div>
						)}
					</div>
				)}
				<p className="text-sm text-muted-foreground">
					{`Example messages to show users. Press Enter to add a new message.`}
				</p>
			</div>
		</div>
	);
}
