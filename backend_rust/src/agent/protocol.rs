//! The proposal an agent sends, as a type.
//!
//! The shape is the contract, so it maps to serde with no interpretation: every struct
//! carries `deny_unknown_fields`, because a misspelled key on an import payload is not a
//! value to drop quietly — it is a row the importer thought it sent. `Target` is internally
//! tagged on `kind` so there is one discriminator field and no ambiguity for the client:
//! `existing` carries a portfolio id and nothing else, `new` carries a name and a currency
//! and no id.
//!
//! Money and dates are `String` here, not `Decimal` and not `DateTime`. That is deliberate
//! and it is the opposite of what the `/api` DTOs do: an agent's payload is untrusted input
//! whose *format* failures must come back as named error codes with a path
//! (`non_canonical_decimal` at `assets[0].transactions[2].price`), which a serde
//! deserializer cannot express. So the DTOs accept any string and [`crate::agent::staging::validate`]
//! checks them, in one place, with the path attached. The `/api` path keeps its strict
//! codec because there a 422 with serde's wording is the right answer.

use serde::{Deserialize, Serialize};

use crate::models::TxType;

/// Where the rows go. Exactly one portfolio per batch — the payload cannot express a batch
/// that spans two, which is why the 25-portfolio display cap in `backup::detail` has
/// nothing to do with this type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum Target {
    New {
        name: String,
        base_currency: String,
    },
    Existing {
        portfolio_id: i64,
    },
}

/// One transaction row as the agent wrote it.
///
/// `type` is the closed [`TxType`] from Workstream 0, so an unknown type is refused here
/// rather than stored and then silently ignored by the engines. `date` is a `YYYY-MM-DD`
/// string — a broker statement is a calendar date, not an instant, and the `/api` route's
/// insistence on RFC 3339 (`tests/date_boundary_test.rs`) is a property of that route, not
/// of the truth.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProposalTx {
    pub r#type: TxType,
    pub date: String,
    pub quantity: String,
    pub price: String,
    pub fee: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProposalAsset {
    pub symbol: String,
    #[serde(default)]
    pub isin: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    pub asset_type: String,
    /// Optional on purpose, and its absence is a decision the preview has to surface: the
    /// currency defaults to the portfolio's base currency, and the browser importer's habit
    /// of hardcoding USD is wrong for a EUR statement.
    #[serde(default)]
    pub currency: Option<String>,
    /// At least one row is required — an asset with nothing to import is a mistake worth
    /// naming, and `validate` says which one.
    pub transactions: Vec<ProposalTx>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    pub target: Target,
    pub assets: Vec<ProposalAsset>,
}
