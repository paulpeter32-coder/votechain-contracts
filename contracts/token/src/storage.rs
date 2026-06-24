// Copyright 2024 VoteChain Contributors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Storage accessors for the token contract.
//!
//! # Namespacing strategy
//!
//! All storage entries are keyed by variants of [`TokenDataKey`].  Soroban
//! serialises the enum variant discriminant into the XDR key before any
//! payload, so every variant occupies a completely isolated key space.
//! Adding a new data type requires only a new enum variant — there is no
//! risk of collision with existing keys.
//!
//! Storage tiers in use:
//! - **Instance** – singleton config values (`Admin`, `TotalSupply`, `Version`).
//!   Shares the contract instance TTL; cheap to access.
//! - **Persistent** – per-address data (`Balance`).
//!   Survives ledger expiry; must be bumped explicitly for long-lived entries.
//! - **Temporary** – short-lived allowances (`Allowance`).
//!   Automatically expires; no manual TTL management required.

use crate::types::{ContractError, TokenDataKey};
use soroban_sdk::{Address, Env};

// =============================================================================
// Storage Strategy
// =============================================================================
//
// Soroban provides three storage tiers. Each key in this contract is assigned
// to the tier that best matches its access pattern and lifetime:
//
// INSTANCE storage  – contract-wide singleton values that share the contract's
//                     TTL. Reads are cheap because the entire instance bucket is
//                     loaded in one host-function call. Used for configuration
//                     that is set once and read on almost every invocation.
//
//   TokenDataKey::Admin       – admin address (set at init, read on mint/burn)
//   TokenDataKey::TotalSupply – total tokens in circulation (read on every mint/burn)
//   TokenDataKey::Version     – semver tuple (major, minor, patch)
//
// PERSISTENT storage – per-key TTL, survives ledger expiry independently.
//                      Used for data keyed by a variable (owner address, etc.)
//                      that must survive indefinitely.
//
//   TokenDataKey::Balance(owner) – token balance per address
//
// TEMPORARY storage  – expires at the end of the ledger entry's TTL without
//                      renewal. Suitable for short-lived approvals that do not
//                      need to persist across many ledgers.
//
//   TokenDataKey::Allowance(owner, spender) – ERC-20-style spending allowance
// =============================================================================

/// Returns the token balance of `owner`. Defaults to `0` if never set.
pub fn balance_of(env: &Env, owner: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&TokenDataKey::Balance(owner.clone()))
        .unwrap_or(0)
}

/// Sets the token balance of `owner` to `amount`.
pub fn set_balance(env: &Env, owner: &Address, amount: i128) {
    env.storage()
        .persistent()
        .set(&TokenDataKey::Balance(owner.clone()), &amount);
}

/// Returns the spending allowance granted by `owner` to `spender`. Defaults to `0`.
pub fn allowance(env: &Env, owner: &Address, spender: &Address) -> i128 {
    env.storage()
        .temporary()
        .get(&TokenDataKey::Allowance(owner.clone(), spender.clone()))
        .unwrap_or(0)
}

/// Sets the spending allowance granted by `owner` to `spender`.
pub fn set_allowance(env: &Env, owner: &Address, spender: &Address, amount: i128) {
    env.storage().temporary().set(
        &TokenDataKey::Allowance(owner.clone(), spender.clone()),
        &amount,
    );
}

/// Returns the total token supply. Defaults to `0` before initialisation.
pub fn total_supply(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&TokenDataKey::TotalSupply)
        .unwrap_or(0)
}

/// Stores the total token supply.
pub fn set_total_supply(env: &Env, s: i128) {
    env.storage().instance().set(&TokenDataKey::TotalSupply, &s);
}

/// Returns the stored admin address.
///
/// # Errors
/// - [`ContractError::AdminNotSet`] if the contract has not been initialised.
pub fn get_admin(env: &Env) -> Result<Address, ContractError> {
    env.storage()
        .instance()
        .get(&TokenDataKey::Admin)
        .ok_or(ContractError::AdminNotSet)
}

/// Stores the admin address in instance storage.
pub fn set_admin(env: &Env, admin: &Address) {
    env.storage().instance().set(&TokenDataKey::Admin, admin);
}

/// Stores the contract version as a `(major, minor, patch)` tuple.
pub fn set_version(env: &Env, version: (u32, u32, u32)) {
    env.storage()
        .instance()
        .set(&TokenDataKey::Version, &version);
}

/// Returns the stored contract version as a `(major, minor, patch)` tuple.
pub fn get_version(env: &Env) -> (u32, u32, u32) {
    env.storage().instance().get(&TokenDataKey::Version).unwrap_or((0, 0, 0))
}

/// Returns `true` if `addr` is frozen.
pub fn is_frozen(env: &Env, addr: &Address) -> bool {
    env.storage().persistent().get(&TokenDataKey::Frozen(addr.clone())).unwrap_or(false)
}

/// Sets the frozen flag for `addr`.
pub fn set_frozen(env: &Env, addr: &Address, frozen: bool) {
    env.storage().persistent().set(&TokenDataKey::Frozen(addr.clone()), &frozen);
}
