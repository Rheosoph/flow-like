"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "./card";
import { Input } from "./input";
import { Label } from "./label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "./select";
import { Switch } from "./switch";

interface EventTypeConfigProps {
	type: string;
	config: any;
	onChange: (config: any) => void;
}

export function EventTypeConfig({
	type,
	config,
	onChange,
}: EventTypeConfigProps) {
	const { t } = useTranslation("common");
	const handleConfigChange = (field: string, value: any) => {
		onChange({ ...config, [field]: value });
	};

	const renderChatConfig = () => (
		<Card>
			<CardHeader>
				<CardTitle className="text-lg">
					{t("chatConfiguration", "Chat Configuration")}
				</CardTitle>
				<CardDescription>
					{t(
						"configureChatspecificSettings",
						"Configure chat-specific settings",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-4">
				<div className="flex items-center space-x-2">
					<Switch
						id="allow_file_upload"
						checked={config.allow_file_upload ?? false}
						onCheckedChange={(checked) =>
							handleConfigChange("allow_file_upload", checked)
						}
					/>
					<Label htmlFor="allow_file_upload">
						{t("allowFileUpload", "Allow File Upload")}
					</Label>
				</div>

				<div className="space-y-2">
					<Label htmlFor="history_elements">
						{t("historyElements", "History Elements")}
					</Label>
					<Input
						id="history_elements"
						type="number"
						value={config.history_elements || ""}
						onChange={(e) =>
							handleConfigChange(
								"history_elements",
								Number.parseInt(e.target.value) || null,
							)
						}
						placeholder={t(
							"numberOfHistoryElementsToKeep",
							"Number of history elements to keep",
						)}
					/>
				</div>
			</CardContent>
		</Card>
	);

	const renderMailConfig = () => (
		<Card>
			<CardHeader>
				<CardTitle className="text-lg">
					{t("mailConfiguration", "Mail Configuration")}
				</CardTitle>
				<CardDescription>
					{t(
						"configureEmailProcessingSettings",
						"Configure email processing settings",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-4">
				<div className="grid grid-cols-2 gap-4">
					<div className="space-y-2">
						<Label htmlFor="imap_server">
							{t("imapServer", "IMAP Server")}
						</Label>
						<Input
							id="imap_server"
							value={config.imap_server || ""}
							onChange={(e) =>
								handleConfigChange("imap_server", e.target.value)
							}
							placeholder="imap.example.com"
						/>
					</div>

					<div className="space-y-2">
						<Label htmlFor="imap_port">{t("imapPort", "IMAP Port")}</Label>
						<Input
							id="imap_port"
							type="number"
							value={config.imap_port || ""}
							onChange={(e) =>
								handleConfigChange(
									"imap_port",
									Number.parseInt(e.target.value) || null,
								)
							}
							placeholder="993"
						/>
					</div>
				</div>

				<div className="space-y-2">
					<Label htmlFor="imap_username">
						{t("imapUsername", "IMAP Username")}
					</Label>
					<Input
						id="imap_username"
						value={config.imap_username || ""}
						onChange={(e) =>
							handleConfigChange("imap_username", e.target.value)
						}
						placeholder="username@example.com"
					/>
				</div>

				<div className="grid grid-cols-2 gap-4">
					<div className="space-y-2">
						<Label htmlFor="smtp_server">
							{t("smtpServer", "SMTP Server")}
						</Label>
						<Input
							id="smtp_server"
							value={config.smtp_server || ""}
							onChange={(e) =>
								handleConfigChange("smtp_server", e.target.value)
							}
							placeholder="smtp.example.com"
						/>
					</div>

					<div className="space-y-2">
						<Label htmlFor="smtp_port">{t("smtpPort", "SMTP Port")}</Label>
						<Input
							id="smtp_port"
							type="number"
							value={config.smtp_port || ""}
							onChange={(e) =>
								handleConfigChange(
									"smtp_port",
									Number.parseInt(e.target.value) || null,
								)
							}
							placeholder="587"
						/>
					</div>
				</div>

				<div className="space-y-2">
					<Label htmlFor="sender_name">{t("senderName", "Sender Name")}</Label>
					<Input
						id="sender_name"
						value={config.sender_name || ""}
						onChange={(e) => handleConfigChange("sender_name", e.target.value)}
						placeholder={t("yourName", "Your Name")}
					/>
				</div>

				<div className="space-y-2">
					<Label htmlFor="mail">{t("emailAddress", "Email Address")}</Label>
					<Input
						id="mail"
						type="email"
						value={config.mail || ""}
						onChange={(e) => handleConfigChange("mail", e.target.value)}
						placeholder="email@example.com"
					/>
				</div>
			</CardContent>
		</Card>
	);

	const renderApiConfig = () => (
		<Card>
			<CardHeader>
				<CardTitle className="text-lg">
					{t("apiConfiguration", "API Configuration")}
				</CardTitle>
				<CardDescription>
					{t("configureApiEndpointSettings", "Configure API endpoint settings")}
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-4">
				<div className="space-y-2">
					<Label htmlFor="method">{t("httpMethod", "HTTP Method")}</Label>
					<Select
						value={config.method || ""}
						onValueChange={(value) => handleConfigChange("method", value)}
					>
						<SelectTrigger>
							<SelectValue
								placeholder={t("selectHttpMethod", "Select HTTP method")}
							/>
						</SelectTrigger>
						<SelectContent>
							<SelectItem value="GET">GET</SelectItem>
							<SelectItem value="POST">{`POST`}</SelectItem>
							<SelectItem value="PUT">PUT</SelectItem>
							<SelectItem value="DELETE">{`DELETE`}</SelectItem>
							<SelectItem value="PATCH">{`PATCH`}</SelectItem>
						</SelectContent>
					</Select>
				</div>

				<div className="space-y-2">
					<Label htmlFor="path_suffix">{t("pathSuffix", "Path Suffix")}</Label>
					<Input
						id="path_suffix"
						value={config.path_suffix || ""}
						onChange={(e) => handleConfigChange("path_suffix", e.target.value)}
						placeholder="/api/endpoint"
					/>
				</div>

				<div className="flex items-center space-x-2">
					<Switch
						id="public_endpoint"
						checked={config.public_endpoint ?? false}
						onCheckedChange={(checked) =>
							handleConfigChange("public_endpoint", checked)
						}
					/>
					<Label htmlFor="public_endpoint">
						{t("publicEndpoint", "Public Endpoint")}
					</Label>
				</div>
			</CardContent>
		</Card>
	);

	switch (type) {
		case "chat":
			return renderChatConfig();
		case "mail":
			return renderMailConfig();
		case "api":
			return renderApiConfig();
		default:
			return null;
	}
}
