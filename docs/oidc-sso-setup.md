# Setting up OIDC SSO login with `exchange-code-for-tokens` + `inspect-id-token`

These two steps are generic across any OIDC provider (Microsoft Entra, Google, Okta, Auth0,
etc.) — nothing about them is provider-specific, so every value below comes from your own
provider's app registration, not from these steps.

## Configuration values you'll need

All of these are genuinely independent, provider-supplied values — there's no way to derive
one from another while staying provider-agnostic, so plan on configuring all of them per
provider you integrate with:

- **Token endpoint**, **Authorization endpoint**, **Issuer** — three separate URLs from your
  provider's OIDC configuration (for Entra, its own `.well-known/openid-configuration` document
  lists all three for your tenant).
- **Client ID** / **Client Secret** — from your app registration. Client ID is also what you use
  for `inspect-id-token`'s "Expected Audience" — per the OIDC spec, an ID token's `aud` claim is
  always the requesting client's ID, so this is one Configuration value reused in two places, not
  two separate ones.
- **Redirect URI** — your Betty Blocks app's own callback URL, registered with the provider.
- **Scopes** — used when building the authorization redirect URL (typically `openid profile
  email` or similar).

## The `claims` output: finding out what fields your provider actually sends

`inspect-id-token`'s `claims` output can be typed via a **Schema Model** so later steps can bind
directly to fields like `claims.email` instead of going through a JSON Path step. But which
fields actually exist depends entirely on your provider, your requested scopes, and (for Entra)
any optional claims configured in the app registration — there's no universal answer.

**The fastest way to find out:** add a **Log Message** step right after `inspect-id-token` in
your flow, log the `claims` output, and run the login flow once for real. The log will show you
the exact JSON your provider actually returned — build your Schema Model from that, not from
guessing.

### Starter reference: common Microsoft Entra ID token claims

Confirmed from a real Entra ID token (tenant with `openid profile email` scopes, default
optional claims) — a reasonable starting point for an Entra Schema Model, not a guarantee your
own tenant/scope/optional-claims configuration produces the exact same set:

| Claim | Type | Notes |
|---|---|---|
| `email` | Text | User's email address |
| `preferred_username` | Text | Usually the same as `email`, but not guaranteed to be |
| `name` | Text | Display name |
| `sub` | Text | Stable, unique identifier for this user *for this app* — good for a primary key |
| `oid` | Text | The user's object ID in the tenant — stable across apps, unlike `sub` |
| `tid` | Text | Tenant ID |
| `groups` | Array (Text) | Group *object IDs* (GUIDs), not names — resolving to human-readable group names needs a separate Microsoft Graph call, it's not in the token |

Also present but rarely useful for application logic: `aio`, `rh`, `uti` (internal/opaque
tokens), `sid` (session ID), `ver` (token version), `iat`/`nbf` (issued-at/not-before
timestamps). `iss` and `aud` are already checked by the step itself and don't need to be in your
Schema Model.
