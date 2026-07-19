import type { IVoiceConfig } from "./event-payload-chat";

export interface IEventPayload {
	ai_disclosure?: string | null;
	allow_file_upload?: boolean | null;
	attach_widget_snapshots?: boolean | null;
	allow_voice_input?: boolean | null;
	allow_voice_output?: boolean | null;
	allow_voice_mode?: boolean | null;
	background_image?: string | null;
	color_scheme?: "system" | "light" | "dark" | null;
	custom_css?: string | null;
	default_tools?: string[] | null;
	example_messages?: string[] | null;
	history_elements?: number | null;
	navigate_to_routes?: string[] | null;
	tools?: string[] | null;
	voice?: IVoiceConfig | null;
	imap_port?: number | null;
	imap_server?: null | string;
	imap_username?: null | string;
	mail?: null | string;
	secret_imap_password?: null | string;
	secret_smtp_password?: null | string;
	sender_name?: null | string;
	smtp_port?: number | null;
	smtp_server?: null | string;
	smtp_username?: null | string;
	method?: null | string;
	path_suffix?: null | string;
	public_endpoint?: boolean | null;
	[property: string]: any;
}
