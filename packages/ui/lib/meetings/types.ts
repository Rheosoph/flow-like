import type { IRealtimeAccess } from "../realtime/types";

export interface Meeting {
	id: string;
	conversation_id: string;
	title: string;
	host_id: string;
	starts_at: string;
	ends_at: string;
	ended_at: string | null;
	guests_allowed: boolean;
}

export type MeetingPreview = Pick<
	Meeting,
	"id" | "title" | "starts_at" | "ends_at" | "ended_at" | "guests_allowed"
>;

export interface MeetingParticipant {
	id: string;
	display_name: string;
	guest: boolean;
	status: "waiting" | "admitted" | "left" | "removed";
	is_host: boolean;
}

export interface MeetingMessage {
	id: string;
	participant_id: string;
	display_name: string;
	body: string;
	created_at: string;
}

export interface MeetingSession {
	meeting: Meeting;
	me: MeetingParticipant;
	participants: MeetingParticipant[];
	messages: MeetingMessage[];
	server_time: string;
	room_epoch: number;
}

export interface MeetingCredentials {
	session_token: string;
	participant_id: string;
}

export interface MeetingRealtimeAccess extends IRealtimeAccess {
	room_id: string;
	expires_at: number;
	expires_in: number;
	server_time: string;
	signaling_servers?: string[];
}

export interface CreateMeeting {
	conversation_id: string;
	title: string;
	starts_at: string;
	ends_at: string;
	guests_allowed: boolean;
}

export interface MeetingClient {
	list(conversationId: string): Promise<Meeting[]>;
	create(
		input: CreateMeeting,
	): Promise<{ meeting: Meeting; invite_token: string }>;
	invite(meetingId: string): Promise<{ invite_token: string }>;
	rotateInvite(meetingId: string): Promise<{ invite_token: string }>;
	preview(meetingId: string, inviteToken?: string): Promise<MeetingPreview>;
	join(meetingId: string): Promise<MeetingCredentials>;
	joinGuest(
		meetingId: string,
		inviteToken: string,
		name: string,
	): Promise<MeetingCredentials>;
	session(meetingId: string, token: string): Promise<MeetingSession>;
	realtime(meetingId: string, token: string): Promise<MeetingRealtimeAccess>;
	send(meetingId: string, token: string, body: string): Promise<void>;
	leave(meetingId: string, token: string): Promise<void>;
	admit(meetingId: string, participantId: string, token: string): Promise<void>;
	remove(
		meetingId: string,
		participantId: string,
		token: string,
	): Promise<void>;
	end(meetingId: string, token?: string): Promise<void>;
}
