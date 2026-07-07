//! Thin Stripe wrapper.
//!
//! `async-stripe` is the underlying SDK. This module narrows the
//! surface to exactly what the demo needs: create a customer with a
//! payment method (called once at setup), and create a payment
//! intent for a charge (called once per checkout).
//!
//! In test mode (`sk_test_*`) every operation is a real Stripe API
//! call but no real money moves; test card 4242... succeeds, test
//! card 4000000000000002 declines.

use stripe::{
    Client, CreatePaymentIntent, Currency, Customer, CustomerId, Expandable, PaymentIntent,
    PaymentIntentConfirmationMethod, PaymentIntentId,
};

#[derive(Clone)]
pub struct StripeClient {
    inner: Client,
}

#[derive(Debug, thiserror::Error)]
pub enum StripeError {
    #[error("stripe call failed: {0}")]
    Call(String),
    #[error("stripe declined the charge: {0}")]
    Declined(String),
    #[error("invalid customer id: {0}")]
    InvalidCustomerId(String),
    #[error("unsupported currency: {0}")]
    UnsupportedCurrency(String),
}

pub struct ChargeOutcome {
    pub payment_intent_id: String,
    pub status: String,
}

impl StripeClient {
    pub fn new(secret_key: &str) -> Self {
        StripeClient {
            inner: Client::new(secret_key.to_string()),
        }
    }

    /// Create + confirm a payment intent off-session against the
    /// customer's default payment method. Returns Ok with the intent's
    /// id and status on success; Err(Declined) on a Stripe-level
    /// decline; Err(Call) on transport / API errors.
    pub async fn charge_customer(
        &self,
        customer_id: &str,
        amount_cents: i64,
        currency_code: &str,
        metadata: &[(&str, String)],
    ) -> Result<ChargeOutcome, StripeError> {
        let cust = customer_id
            .parse::<CustomerId>()
            .map_err(|e| StripeError::InvalidCustomerId(e.to_string()))?;

        let currency = parse_currency(currency_code)?;

        // Stripe quirk: PaymentIntent::create does NOT automatically use
        // the customer's invoice_settings.default_payment_method. That
        // setting only applies to invoices/subscriptions. For a raw
        // off-session PaymentIntent we must look up the saved method
        // and pass it explicitly.
        let customer = Customer::retrieve(&self.inner, &cust, &[])
            .await
            .map_err(|e| StripeError::Call(format!("customer lookup failed: {}", e)))?;

        let pm_id = customer
            .invoice_settings
            .as_ref()
            .and_then(|s| s.default_payment_method.as_ref())
            .map(|pm| match pm {
                Expandable::Id(id) => id.clone(),
                Expandable::Object(obj) => obj.id.clone(),
            })
            .ok_or_else(|| {
                StripeError::Call(
                    "customer has no default payment method; setup script needs to attach one"
                        .into(),
                )
            })?;

        let mut create = CreatePaymentIntent::new(amount_cents, currency);
        create.customer = Some(cust);
        create.payment_method = Some(pm_id);
        create.confirm = Some(true);
        create.confirmation_method = Some(PaymentIntentConfirmationMethod::Automatic);
        create.off_session = Some(stripe::PaymentIntentOffSession::Exists(true));

        let mut meta = std::collections::HashMap::new();
        for (k, v) in metadata {
            meta.insert((*k).to_string(), v.clone());
        }
        if !meta.is_empty() {
            create.metadata = Some(meta);
        }

        let intent: PaymentIntent = PaymentIntent::create(&self.inner, create)
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("card_declined")
                    || msg.contains("declined")
                    || msg.contains("payment_intent_authentication_failure")
                {
                    StripeError::Declined(msg)
                } else {
                    StripeError::Call(msg)
                }
            })?;

        Ok(ChargeOutcome {
            payment_intent_id: intent.id.to_string(),
            status: format!("{:?}", intent.status).to_lowercase(),
        })
    }

    /// Lookup a payment intent. Used by tests and by the transactions
    /// view to confirm Stripe's record matches our own.
    #[allow(dead_code)]
    pub async fn lookup_intent(&self, id: &str) -> Result<PaymentIntent, StripeError> {
        let pi_id = id
            .parse::<PaymentIntentId>()
            .map_err(|e| StripeError::Call(format!("bad intent id: {}", e)))?;
        PaymentIntent::retrieve(&self.inner, &pi_id, &[])
            .await
            .map_err(|e| StripeError::Call(e.to_string()))
    }
}

fn parse_currency(code: &str) -> Result<Currency, StripeError> {
    // async-stripe's Currency enum is large; rely on its FromStr.
    code.to_lowercase()
        .parse::<Currency>()
        .map_err(|_| StripeError::UnsupportedCurrency(code.to_string()))
}
