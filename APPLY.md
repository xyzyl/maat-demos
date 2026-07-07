# Slice 11 — Demos reorganization + email-outreach + deploy

> **Status: applied.** This migration playbook has been fully executed —
> the demos live in `maat-demos/`, the old `demo-store`/`demo-agent`
> crates are gone from the gateway workspace, and the demo databases
> are provisioned. Kept for historical reference.

This slice does two things in one go:

1. Reorganizes the demos out of `maat-gateway/crates/demo-{store,agent}`
   into a new top-level `maat-demos/` workspace, the way third-party
   integrators would consume the protocol.
2. Adds two new demos exercising parts of the protocol grammar the
   commerce demo doesn't reach: `MaxRate` + `DomainAllow` + content-hash
   anchor binding (email-outreach), and `RequireAnchorFreshness` +
   hierarchical scope + commit-hash anchor binding (deploy).

## Prerequisite final layout

```
maatFolder/
├── maat/                  # protocol library (0.4.0)
├── maat-gateway-slice8/   # rename to maat-gateway later if desired
├── maat-sdk-slice9/       # rename to maat-sdk later if desired
└── maat-demos/            # ← NEW from this slice
```

(The path deps in `maat-demos/Cargo.toml` reference
`../maat-sdk-slice9/crates/...` to match the user's current naming.
If you rename to `maat-sdk`, update those four path entries.)

## Phase 1 — drop in maat-demos/

Unpack the tarball at the maatFolder level. You should now have
`maatFolder/maat-demos/` alongside the existing top-level crates.

Verify the workspace builds (this also verifies all SDK + protocol
references resolve correctly):

```sh
cd maatFolder/maat-demos
cargo build --workspace
```

Expected: clean build. No tests need to pass yet; we haven't run any
demos against a live gateway.

## Phase 2 — migrate the commerce demo files

The existing `maat-gateway-slice8/crates/demo-store/` and
`maat-gateway-slice8/crates/demo-agent/` source trees are correct
post-slice-10a (after the build fixes). Move their `src/` directories
into the new locations, keeping the existing `.rs` files exactly as
they are:

```sh
cp -r maat-gateway-slice8/crates/demo-store/src/    maat-demos/commerce/store/src/
cp -r maat-gateway-slice8/crates/demo-store/migrations/  maat-demos/commerce/store/migrations/ 2>/dev/null || true
cp -r maat-gateway-slice8/crates/demo-agent/src/    maat-demos/commerce/agent/src/
```

(The `Cargo.toml` files at the new locations are already in place from
the tarball — use the new ones, not the old. They use workspace deps
from `maat-demos/Cargo.toml` rather than direct path deps.)

In the commerce-store and commerce-agent source files, you may need
to update the `#[lib]` and `#[[bin]]` names if `demo_store` /
`demo-store` is referenced anywhere — the new crate names are
`commerce_store` / `commerce-store` and `commerce_agent` /
`commerce-agent`. A quick search:

```sh
cd maat-demos/commerce
grep -rn "demo_store\|demo_agent\|demo-store\|demo-agent" store/src agent/src
```

For each hit, replace `demo_store` → `commerce_store`, `demo_agent` →
`commerce_agent`, etc.

Then remove the old crates from the gateway workspace:

In `maat-gateway-slice8/Cargo.toml`, remove `"crates/demo-store"` and
`"crates/demo-agent"` from the `members` list. Delete the directories:

```sh
rm -rf maat-gateway-slice8/crates/demo-store
rm -rf maat-gateway-slice8/crates/demo-agent
```

Verify the gateway still builds without them:

```sh
cd maat-gateway-slice8
cargo build --workspace
```

Then build the demos workspace:

```sh
cd ../maat-demos
cargo build --workspace
```

Both should be clean.

## Phase 3 — provision databases for the new demos

Each new demo's resource has its own Postgres database (separate from
the gateway's config database, separate from each other):

```sh
createdb outreach_mailer
createdb deploy_runner

psql outreach_mailer -f maat-demos/email-outreach/mailer/migrations/0001_initial.sql
psql deploy_runner   -f maat-demos/deploy/runner/migrations/0001_initial.sql
```

## Phase 4 — run a new demo end-to-end (smoke test)

Pick email-outreach to start. Five processes total: gateway, KMS,
dashboard (already running from earlier slices), plus mailer + agent.

```sh
# Mailer.
$env:OUTREACH_MAILER_BIND="0.0.0.0:8090"
$env:OUTREACH_MAILER_DATABASE_URL="postgres://user:pass@localhost/outreach_mailer"
$env:MAAT_GATEWAY_URL="http://localhost:8080"
$env:MAAT_API_KEY="<your tenant API key>"
$env:OUTREACH_ALLOWED_DOMAINS="acme.com,example.com"
cargo run -p outreach-mailer

# Agent (separate terminal).
$env:OUTREACH_AGENT_BIND="0.0.0.0:9090"
$env:MAAT_GATEWAY_URL="http://localhost:8080"
$env:MAAT_API_KEY="<your tenant API key>"
$env:OUTREACH_MAILER_URL="http://localhost:8090"
cargo run -p outreach-agent
```

Then in a browser:

1. Open `http://localhost:9090` (the agent UI).
2. Copy the agent's public key from the "Identity" box.
3. In the dashboard, create a delegation:
   - Agent pubkey: paste from step 2.
   - Scope grant: `messaging:email:send`
   - Constraints:
     - `DomainAllow`: `["acme.com"]`
     - `MaxRate`: 5 invocations per 60 seconds
     - `RequireAnchorFreshness`: max_age_seconds = 60
   - Validity: 1 hour.
4. Copy the issued delegation JSON.
5. Paste it into the agent UI's "Delegation" box → Accept.
6. In "Campaign goal" enter:
   - Recipients: 3 `@acme.com` addresses, one per line.
   - Body: anything.
7. Click "Run outreach". Watch the activity log show three approvals
   and three stub-sends.
8. Open `http://localhost:8090` to see the mailer's audit view.

Verification points:

- All three recipients should be marked `sent` in the mailer audit.
- If you change the body before re-running, the mailer should reject
  due to `body hash mismatch` (proves the content-hash binding works).
- If you add an `@evil.com` recipient, the gateway should reject with
  `constraint_violated: domain 'evil.com' not in allow list` (proves
  `DomainAllow` is enforced).
- If you submit a 6th send within 60 seconds, the gateway should
  reject with `max_rate exceeded` (proves `MaxRate` is enforced).

Then repeat the same flow for the deploy demo, using:
- Scope: `ops:deploy:staging`
- Constraints: `RequireAnchorFreshness` 30s
- Goal: environment=staging, service=api, commit_hash=`<40-hex>`

Verification: changing the commit hash between requests should still
pass (each is a fresh anchor); replaying the same receipt should fail
with HTTP 409 `already used`.

## Phase 5 — optional: rename gateway / sdk directories

Once everything works at the existing names, you can rename:

```sh
mv maat-gateway-slice8 maat-gateway
mv maat-sdk-slice9     maat-sdk
```

Then update `maat-demos/Cargo.toml` workspace.dependencies to point
to the new paths:

```toml
maat-sdk-core   = { path = "../maat-sdk/crates/maat-sdk-core" }
maat-agent      = { path = "../maat-sdk/crates/maat-agent" }
maat-resource   = { path = "../maat-sdk/crates/maat-resource" }
```

And the gateway's workspace `Cargo.toml` path to `maat` (it uses
`../maat` already so no change there).

## What this slice did not do

- Did not add SDK 0.2.0 with the `VerifyOutcome::AwaitingHumanConfirm`
  enum variant. No demo in this slice uses `RequireHumanConfirm`, so
  the existing SDK's 4xx-on-202 behavior doesn't bite. When the first
  human-confirm demo lands, the SDK update should go with it.
- Did not add integration tests that spin up a gateway in-process.
  Demo tests gate on `MAAT_TESTKIT_GATEWAY_URL` and run against an
  externally-running stack. This is the same pattern existing gateway
  integration tests use.
- Did not write any cross-demo shared code beyond the testkit. Each
  demo is independent so the production-isolation discipline holds.
