import { createContext } from "react";

/** What a chat offers a failed turn's card once its cause is fixed. */
export interface ChatErrorRecovery {
	/** Sends the question the failed answer belongs to again, once at most per failed answer. */
	resend: (answerId: string) => void;
}

export const ChatErrorRecoveryContext = createContext<ChatErrorRecovery | null>(
	null,
);
