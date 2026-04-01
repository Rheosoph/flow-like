import { INodePermission } from "../schema/flow/board";

export const NODE_PERMISSION_LABELS: Record<
	INodePermission,
	{ label: string; icon: string }
> = {
	[INodePermission.NetworkHttp]: { label: "Network Access (HTTP)", icon: "🌐" },
	[INodePermission.NetworkWebsocket]: {
		label: "WebSocket Connections",
		icon: "🔌",
	},
	[INodePermission.NetworkTcp]: { label: "TCP Sockets", icon: "🔗" },
	[INodePermission.NetworkUdp]: { label: "UDP Sockets", icon: "📶" },
	[INodePermission.NetworkDns]: { label: "DNS Lookups", icon: "🔍" },
	[INodePermission.StorageRead]: { label: "Storage Read", icon: "📖" },
	[INodePermission.StorageWrite]: { label: "Storage Write", icon: "💾" },
	[INodePermission.Variables]: { label: "Flow Variables", icon: "🔀" },
	[INodePermission.Cache]: { label: "Execution Cache", icon: "⚡" },
	[INodePermission.Streaming]: { label: "Streaming Output", icon: "📡" },
	[INodePermission.Models]: { label: "AI Model Access", icon: "🤖" },
	[INodePermission.A2ui]: { label: "Dynamic UI (A2UI)", icon: "🖼️" },
	[INodePermission.OAuth]: { label: "OAuth Authentication", icon: "🔑" },
	[INodePermission.Functions]: { label: "Function Calls", icon: "⚙️" },
};
