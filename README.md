# maat-demos

Demonstration suite for the Maat protocol. Three demos exercising
different parts of the protocol's grammar:

| Demo | Constraint focus | Action descriptor | Anchor state binding |
|---|---|---|---|
| **commerce** | `MaxValue` (currency, amount, decimals) | `ValueClaim` | none |
| **email-outreach** | `MaxRate` + `DomainAllow` | `RecipientClaim` (recipients + body hash) | content-hash of body |
| **deploy** | `RequireAnchorFreshness` + hierarchical scope | `TargetClaim` (env, service, commit) | content-hash of commit + env |

Each demo has an **agent** crate (autonomous actor) and a **resource**
crate (the executing system that honors receipts). The two communicate
over HTTP using `X-Maat-Receipt` as the wire envelope. Neither has
a path-dep on the other — the resource validates receipts independently
through the SDK's `ReceiptValidator`.

## Architecture

```
maatFolder/
├── maat/                  # protocol library
├── maat-gateway-slice8/   # gateway product (dashboard, KMS, verify, ledger)
├── maat-sdk-slice9/       # integration layer (agent + resource crates)
└── maat-demos/            ← THIS
    ├── testkit/           # shared test helpers
    ├── commerce/
    │   ├── store/         # the merchant
    │   └── agent/         # the autonomous shopper
    ├── email-outreach/
    │   ├── mailer/        # the resource
    │   └── agent/         # the campaign runner
    └── deploy/
        ├── runner/        # the resource
        └── agent/         # the deploy orchestrator
```

All path deps point at sibling top-level dirs (`../maat`,
`../maat-sdk-slice9/crates/...`). No demo
depends on any other demo or on the gateway. Demos talk to a running
gateway over HTTP — exactly as a third-party integrator would.

## Running a demo end-to-end

1. Start the gateway + KMS + dashboard (see `maat-gateway-slice8/` for those).
2. Apply each demo's migration to its Postgres database.
3. Set env vars and run the binaries.
4. Open the agent's UI, paste a delegation issued via the dashboard,
   submit a goal, watch it work.
