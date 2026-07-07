//! Decision strategy.
//!
//! The autonomous loop calls `pick_next` to decide what to buy next.
//! The deterministic strategy sorts the catalog by ascending price
//! and picks items in order, stopping when the next item would
//! exceed the goal's budget. Smarter (LLM-driven) strategies are a
//! one-file change behind this trait.

use crate::catalog::Product;

#[derive(Debug, Clone)]
pub struct Goal {
    pub max_total_cents: i64,
    pub max_per_item_cents: i64,
    /// Optional: force a specific failure path for demo purposes.
    /// `Some("per_action_cap")` causes the agent to deliberately submit
    /// a value claim above the per-action cap; etc.
    pub mode: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ChosenItem {
    pub product: Product,
    /// The amount (cents) the agent claims it will spend on this item.
    /// Normally equal to product.price_cents, but in forced-failure
    /// modes the agent may claim more (to trip the per-action cap) or
    /// less (to trip the store's amount-match check).
    pub claim_amount_cents: i64,
}

pub trait Strategy: Send + Sync {
    fn pick_next(
        &self,
        catalog: &[Product],
        already_bought_total_cents: i64,
        already_bought_ids: &[uuid::Uuid],
        goal: &Goal,
    ) -> Option<ChosenItem>;
}

pub struct DeterministicStrategy;

impl Strategy for DeterministicStrategy {
    fn pick_next(
        &self,
        catalog: &[Product],
        already_bought_total_cents: i64,
        already_bought_ids: &[uuid::Uuid],
        goal: &Goal,
    ) -> Option<ChosenItem> {
        // Items not yet bought, sorted ascending by price.
        let mut items: Vec<&Product> = catalog
            .iter()
            .filter(|p| !already_bought_ids.contains(&p.id))
            .filter(|p| p.price_cents <= goal.max_per_item_cents)
            .collect();
        items.sort_by_key(|p| p.price_cents);

        for product in items {
            let projected = already_bought_total_cents + product.price_cents;
            if projected > goal.max_total_cents {
                continue;
            }
            // Forced-failure modes for demo legibility.
            //
            // `per_action_cap`: derive the claim from the goal's per-item
            //   budget (which equals the delegation's MaxValue in the
            //   documented demo flow), then go $1 over. Guarantees the
            //   gateway rejects on MaxValue regardless of which item the
            //   strategy picked.
            //
            // `amount_mismatch`: claim 1 cent less than the actual price.
            //   Gateway approves (claim is small), store rejects on the
            //   amount-match check. Different layer of the stack.
            let claim_amount_cents = match goal.mode.as_deref() {
                Some("per_action_cap") => goal.max_per_item_cents + 100,
                Some("amount_mismatch") => product.price_cents - 1,
                _ => product.price_cents,
            };
            return Some(ChosenItem {
                product: product.clone(),
                claim_amount_cents,
            });
        }
        None
    }
}
