export class FlowLikeError extends Error {
	constructor(
		message: string,
		public readonly statusCode?: number,
		public readonly body?: unknown,
	) {
		super(message);
		this.name = "FlowLikeError";
	}
}

export class AuthError extends FlowLikeError {
	constructor(message: string) {
		super(message, 401);
		this.name = "AuthError";
	}
}

export class NotFoundError extends FlowLikeError {
	constructor(message: string) {
		super(message, 404);
		this.name = "NotFoundError";
	}
}

export class ValidationError extends FlowLikeError {
	constructor(message: string) {
		super(message, 400);
		this.name = "ValidationError";
	}
}
