# Ticket 004: OAuth Scope Contract for Lekton MCP Capabilities

## Summary

Lekton needs a small, stable OAuth scope contract for MCP capabilities so authorization requests can be constructed predictably and insufficient-scope cases can be handled correctly.

## Why this is needed

Lekton already has internal RBAC for resource-level access, but MCP clients still need a standard OAuth scope set that represents the capabilities they are asking for.

## Decisions already taken

Lekton proposes this initial scope set:

- `mcp:read`
- `docs:read`
- `prompts:read`
- `feedback:write`

Lekton will enforce both:

- OAuth scope checks for capability access
- local RBAC checks for actual document and prompt visibility

## Proposed capability mapping

- enumerate and read MCP documentation resources -> `mcp:read` and `docs:read`
- use MCP prompt discovery and retrieval -> `mcp:read` and `prompts:read`
- create documentation feedback via MCP -> `mcp:read` and `feedback:write`

## Requested deliverables from `authentication`

1. Confirm whether the proposed scope names are acceptable.
2. If not, provide the authoritative scope names to be used.
3. Confirm whether scopes are returned space-delimited, array-based, or both in introspection responses.
4. Confirm whether scope downscoping by client is supported.
5. Confirm whether any additional reserved scopes are required for MCP usage.

## Decision proposal for v1

Lekton prefers capability-oriented scopes over document-level scopes.

Specifically:

- no per-document OAuth scopes
- no per-collection OAuth scopes
- no encoding of RBAC access levels into OAuth scope names

Those concerns remain inside Lekton RBAC, which is already implemented and tested.

## Questions to answer in this ticket

1. What exact scope values should Lekton request?
2. Are all proposed scopes valid for public clients using PKCE?
3. Are any scopes considered sensitive or admin-only?
4. How should `insufficient_scope` responses reference the required scope set?

## Acceptance criteria

- Lekton can publish recommended scopes in protected resource metadata.
- MCP clients can request predictable scopes during authorization.
- Lekton can distinguish valid-token-insufficient-scope from invalid-token cases.
