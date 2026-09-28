export class ConfigurationError extends Error {
  override name = "ConfigurationError";
}
export class AuthenticationError extends Error {
  override name = "AuthenticationError";
}
export class AuthorizationRedirectError extends Error {
  override name = "AuthorizationRedirectError";
}
export class CloudTransportError extends Error {
  override name = "CloudTransportError";
  constructor(
    message: string,
    readonly status?: number,
    readonly retryable = false,
  ) {
    super(message);
  }
}
export class CloudConflictError extends Error {
  override name = "CloudConflictError";
  constructor(
    readonly remote: { version: number; createdAt: string; createdBy: string },
    message: string,
  ) {
    super(message);
  }
}
