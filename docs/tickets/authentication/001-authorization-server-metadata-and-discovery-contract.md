# Ticket 001: Authorization Server Metadata and Discovery Contract

## Summary

Lekton needs a stable OAuth2/OIDC discovery contract from the `authentication` service so its MCP server can advertise itself as a protected resource and direct MCP clients to the correct authorization server.

## Why this is needed

Lekton will add `/.well-known/oauth-protected-resource` on its own side. That metadata must point at one or more authorization servers that are discoverable and stable enough for standard MCP clients to complete OAuth flows without out-of-band configuration.

## Decisions already taken

- Lekton will treat the MCP client as the OAuth client.
- Lekton will not embed the OAuth client flow in the portal or in a gateway.
- Lekton will support one primary authorization server in local configuration.
- Lekton will publish `authorization_servers` as a list for protocol compatibility and future expansion.

## Requested deliverables from `authentication`

1. Expose a stable authorization-server metadata endpoint.
2. Confirm the canonical issuer URL to be used in development, staging, and production.
3. Ensure metadata includes at least:
   - issuer
   - authorization endpoint
   - token endpoint
   - supported grant types
   - PKCE support details
   - supported token introspection endpoint, if separate
4. Confirm whether the service is providing:
   - OpenID Connect discovery
   - OAuth 2.0 Authorization Server Metadata only
   - both
5. Confirm whether multiple issuers or environment-specific issuers are expected.

## Contract required by Lekton

Lekton expects:

- a stable issuer string
- stable discovery URLs
- `authorization_code` support
- PKCE support for public clients
- no hidden environment-specific values outside repository configuration

## Questions to answer in this ticket

1. What is the canonical metadata URL in each environment?
2. Is the metadata OIDC-compatible, OAuth-compatible, or both?
3. Are public clients with PKCE supported without client secret?
4. Are there any required custom parameters beyond standard OAuth fields?
5. Can Lekton safely treat the issuer as long-lived and cacheable?

## Acceptance criteria

- Lekton can configure its protected resource metadata with a stable `authorization_servers` value.
- A standard MCP client can discover the authorization server starting from the Lekton MCP endpoint.
- The development environment does not require undocumented manual steps.
