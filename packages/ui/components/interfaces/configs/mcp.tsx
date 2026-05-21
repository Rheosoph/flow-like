"use client";

import { Cloud, Info, Server } from "lucide-react";
import {
	Alert,
	AlertDescription,
	AlertTitle,
	Input,
	Label,
} from "../../ui";
import type { IConfigInterfaceProps } from "../interfaces";

type McpSink = {
	sink_type?: "mcp";
	prefix?: string;
};

export function McpConfig({
	config,
	onConfigUpdate,
	isEditing,
}: IConfigInterfaceProps) {
	const current = (config as McpSink) ?? {};
	const prefix = current.prefix ?? "";

	const setValue = (key: keyof McpSink, value: unknown) => {
		onConfigUpdate?.({
			...(config as McpSink),
			sink_type: "mcp",
			[key]: value,
		} as any);
	};

	return (
		<div className="w-full space-y-4">
			<Alert>
				<Server className="h-4 w-4" />
				<AlertTitle className="flex items-center gap-2">
					MCP Server
					<span className="inline-flex items-center gap-1 rounded-full bg-muted px-2 py-0.5 text-xs">
						<Cloud className="h-3 w-3" /> Remote only
					</span>
				</AlertTitle>
				<AlertDescription>
					Spins up a remote Model Context Protocol server. Exposed tools,
					schemas and authentication are declared inside the workflow board —
					the event itself only mounts the server.
				</AlertDescription>
			</Alert>

			<div className="space-y-2">
				<Label htmlFor="mcp_prefix">Prefix</Label>
				<Input
					id="mcp_prefix"
					placeholder="/mcp"
					disabled={!isEditing}
					value={prefix}
					onChange={(event) => setValue("prefix", event.target.value)}
				/>
				<p className="text-xs text-muted-foreground flex items-center gap-1">
					<Info className="h-3 w-3" />
					Path prefix the MCP server is mounted under (e.g. <code>/mcp</code>).
				</p>
			</div>
		</div>
	);
}
