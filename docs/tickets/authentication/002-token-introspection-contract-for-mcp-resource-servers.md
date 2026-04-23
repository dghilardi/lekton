# Ticket 002: Token Introspection Contract for MCP Resource Servers

## Summary

Lekton needs a backend token-validation contract from the `authentication` service for validating bearer access tokens presented to the MCP server.

## Why this is needed

Lekton has decided not to validate JWT access tokens locally. The MCP server will delegate token validation to `authentication` through token introspection or an equivalent internal API.

## Decisions already taken

- Lekton will fail closed when validation is unavailable.
- Lekton will use short request timeouts and limited retry behavior.
- Lekton will normalize successful validation into an internal `McpPrincipal`.
- Lekton will support `pat`, `oauth2`, and `hybrid` modes. Only `oauth2` and `hybrid` depend on this contract.

## Requested deliverables from `authentication`

1. Provide a stable token introspection endpoint or equivalent internal validation API.
2. Document the authentication method required by that endpoint:
   - client secret
   - mTLS
   - internal network trust
   - bearer token
   - other
3. Return a stable response contract containing at least:
   - `active`
   - subject identifier
   - client identifier
   - scope list
   - token expiry
4. Define the response shape for:
   - expired token
   - revoked token
   - malformed token
   - unsupported token
   - auth service temporary failure
5. Clarify rate limits and expected latency budget for resource-server usage.

## Minimum response contract needed by Lekton

Lekton needs these fields, with stable semantics:

- `active: boolean`
- `sub: string`
- `client_id: string | null`
- `scope: string | array`
- `exp: unix timestamp | null`

Optional but useful:

- `iss`
- `aud`
- `token_type`
- `username`
- `email`
- custom grant data if officially supported

## Error handling expectations

Lekton will map the contract as follows:

- inactive or invalid token -> `401 Unauthorized`
- valid token with insufficient scope -> `403 Forbidden`
- introspection timeout or service failure -> secure failure, not anonymous fallback

## Questions to answer in this ticket

1. What endpoint should Lekton call for token validation?
2. How should Lekton authenticate to that endpoint?
3. What exact JSON schema should Lekton expect?
4. Is the endpoint suitable for per-request use by an MCP resource server?
5. Are there recommended cache semantics for positive or negative introspection results?

## Acceptance criteria

- Lekton can validate MCP bearer tokens without reimplementing token validation logic.
- The validation response is stable enough to build a typed principal.
- Failure modes are documented well enough to produce correct `401` and `403` responses.
