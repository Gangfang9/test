# Hong Kong membership deployment

The desktop client calls only `https://www.jxzs.top`. Nginx forwards five POST
routes to the original membership gateway. The gateway uses the original U验证
installation to authenticate accounts, redeem recharge cards, and read VIP
expiry. Database, Redis, U验证, and gateway ports are not public endpoints.

## Migration status — 2026-09-28

- Imported the original MySQL database and gateway state after stopping the old
  gateway and U验证 services. Old sessions were cleared; members must log in again.
- HTTPS certificate installed, with automatic renewal and Nginx reload hook.
- A dedicated account and recharge card passed live HTTPS checks: registration,
  login without membership, rejection of an invalid card, U验证 card redemption,
  membership heartbeat, rejection of reused card and mismatched device, logout,
  and rejection of the logged-out session. U验证 records the redemption in
  `u_cdk_user`; `u_cdk_kami` belongs to the other card-login mode.
- Public admin and raw U验证 routes return 404.
- Cloudflare onboarding is pending. DNS currently exposes the origin. Do not
  describe this deployment as hiding the origin until proxying and origin access
  restrictions have been verified.

## Acceptance script

`acceptance.py` runs as root on the server. It creates a marked one-day recharge
card directly in the existing account-mode table and registers an account through
the public endpoint. It does not grant VIP directly: redemption and VIP state are
decided by U验证. Credentials remain in a mode-600 private root file, outside Git.
This validates user redemption, not the admin card-generation UI.

The script is intended for one initial run. `--resume` is only for a reviewed
failure before redemption; a redeemed card cannot rerun the initial checks.

## Client protections and limits

The native application checks membership before protected APIs, WebSocket
control, and projection startup. Cloud heartbeats refresh access; failed checks
invalidate membership. Frontend edits alone do not change the native session.
U验证 administration secrets are not shipped to the client. Redirects from the
membership endpoint are rejected.

These checks cannot guarantee resistance to a modified native executable.
Features implemented entirely on an attacker-controlled computer can be patched.
Critical operations requiring stronger enforcement must execute on a server that
independently checks membership.

## Cloudflare follow-up

Proxy the `www` record, select Full (strict), and verify the nameserver delegation
and edge certificate. Keep membership responses uncached. Once proxying works,
configure trusted Cloudflare client-IP headers and restrict origin HTTP/S ingress
to Cloudflare ranges or use an outbound tunnel. Do not apply that restriction
before proxying works. Recheck certificate renewal after firewall changes.

Historical DNS may retain the previous origin address; proxying does not erase it.
Never commit migration archives, SQL dumps, account credentials, or tokens.
