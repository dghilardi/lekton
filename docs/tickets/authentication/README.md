# Authentication Tickets for MCP OAuth2 Integration

These tickets capture the expected contracts and deliverables from the `authentication` service team so Lekton can expose its MCP server as a standard OAuth2 protected resource while keeping the current PAT-based mode available during migration.

## Decisions already taken on the Lekton side

- Lekton will support `pat`, `oauth2`, and `hybrid` MCP auth modes via configuration.
- Initial rollout target is `hybrid`:
  - existing PAT-based MCP clients keep working
  - standard OAuth2 MCP clients can start using bearer tokens issued by `authentication`
- The MCP client is responsible for the OAuth authorization flow.
- Lekton will expose protected resource metadata and standard `WWW-Authenticate` challenges.
- Lekton will not validate JWT access tokens locally.
- Lekton will use token introspection or an equivalent backend validation API exposed by `authentication`.
- OAuth scopes will gate MCP capabilities, while Lekton RBAC will continue to decide which documents/prompts the subject can actually access.
- Lekton will start with a single primary authorization server in configuration, but will publish `authorization_servers` as a list in protected resource metadata for forward compatibility.

## Ticket list

- [001 - Authorization Server Metadata and Discovery Contract](./001-authorization-server-metadata-and-discovery-contract.md)
- [002 - Token Introspection Contract for MCP Resource Servers](./002-token-introspection-contract-for-mcp-resource-servers.md)
- [003 - Subject Mapping and Authorization Model Contract](./003-subject-mapping-and-authorization-model-contract.md)
- [004 - OAuth Scope Contract for Lekton MCP Capabilities](./004-oauth-scope-contract-for-lekton-mcp-capabilities.md)
- [005 - End-to-End Development Smoke Flow Support](./005-end-to-end-development-smoke-flow-support.md)
