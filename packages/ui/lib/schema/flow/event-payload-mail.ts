export interface IEventPayloadMail {
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
	[property: string]: any;
}
