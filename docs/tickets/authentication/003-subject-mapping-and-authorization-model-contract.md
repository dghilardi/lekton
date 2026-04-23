# Ticket 003: Subject Mapping and Authorization Model Contract

## Summary

Lekton needs a stable identity contract so an OAuth2-authenticated subject can be mapped to Lekton's existing RBAC model for document and prompt access.

## Why this is needed

Lekton already has working application-side authorization based on user records and per-access-level permissions. The missing piece is how an access token subject issued by `authentication` maps into that existing model.

## Decisions already taken

- Lekton will separate authentication from authorization.
- OAuth scopes will enable MCP capabilities.
- Existing Lekton RBAC will still decide which concrete documents and prompts the subject can access.
- The first implementation will prefer static subject mapping over dynamic remote grant resolution.

## Requested deliverables from `authentication`

1. Define the stable subject identifier field Lekton should trust.
2. Confirm whether that identifier is immutable across the user lifecycle.
3. Clarify whether `client_id` identifies:
   - the calling MCP client application
   - an installation
   - another entity
4. Clarify whether user identity claims such as email are available and stable enough for diagnostics only.
5. Confirm whether authorization data beyond scope is expected to travel:
   - inside the token
   - via introspection response
   - via a separate grants API

## Decision proposal for initial rollout

Lekton proposes the following contract:

- `sub` is the only stable subject key used for identity mapping
- Lekton stores or resolves a local user record keyed by provider subject
- `client_id` is treated as client metadata, not as the user identity
- OAuth scopes gate MCP capabilities only
- no dynamic external grant resolution in v1

This keeps the first integration simple and avoids coupling document authorization to a second remote dependency on every MCP request.

## Fallback behavior expected by Lekton

- if the token is valid but the subject is unknown to Lekton, Lekton may reject access or auto-provision a minimal local user, depending on final local policy
- if external grants are required for correctness, that must be explicit before implementation starts

## Questions to answer in this ticket

1. Which field is the canonical identity key for a human user?
2. Can `sub` change over time or across environments?
3. Is email informational or identity-grade?
4. Will authorization grants beyond scope be required in v1?
5. If yes, what API should Lekton call and what is the expected latency/reliability model?

## Acceptance criteria

- Lekton can reliably map a validated token to a local application principal.
- The mapping does not depend on unstable or presentation-oriented claims.
- The first release can ship without ambiguous identity semantics.
