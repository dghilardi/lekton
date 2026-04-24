# Ticket 005: End-to-End Development Smoke Flow Support

## Summary

Lekton needs a repeatable end-to-end development flow against the real `authentication` service so the MCP OAuth integration can be demonstrated and debugged without hidden setup.

## Why this is needed

The final acceptance criteria for the MCP integration require a real authorization flow, not only mocked validation. Without a reproducible dev flow, the protocol integration is likely to fail late.

## Decisions already taken

- Lekton will support `hybrid` mode during rollout so PAT-based clients keep working.
- OAuth validation success must be testable independently from PAT mode.
- The end-to-end flow should use `authorization_code` with PKCE.

## Requested deliverables from `authentication`

1. Provide a development-ready authorization server environment.
2. Provide one or more test clients suitable for MCP-style usage.
3. Provide test user credentials or a documented local bootstrap flow.
4. Document the exact sequence for:
   - discovery
   - authorize
   - code exchange
   - token validation by resource server
5. Clarify how revoked or expired tokens can be simulated in development.

## Smoke flow expected by Lekton

The dev flow should make it possible to test:

1. Protected resource metadata discovery from Lekton.
2. Authorization server discovery from `authentication`.
3. Browser-based user authorization with PKCE.
4. Token exchange.
5. MCP request accepted with a valid access token.
6. MCP request rejected with missing or invalid token.
7. MCP request rejected with insufficient scope.
8. MCP request rejected after expiry or revocation.

## Questions to answer in this ticket

1. What environment should Lekton use for local and preprod smoke testing?
2. What client registration should be used for MCP testing?
3. Is there a standard dev user bootstrap path?
4. How can revocation and expiry be tested deterministically?
5. Is there an existing smoke script or Postman collection the teams can reuse?

## Acceptance criteria

- Both teams can run the same documented smoke flow.
- The flow covers success, invalid token, insufficient scope, and revoked or expired token cases.
- No environment-critical setup remains implicit or tribal knowledge.
