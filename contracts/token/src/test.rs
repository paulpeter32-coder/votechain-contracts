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

#![cfg(test)]
use super::*;
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events},
    Address, Env, IntoVal, TryFromVal,
};

fn setup() -> (Env, TokenContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register(TokenContract, ());
    (env.clone(), TokenContractClient::new(&env, &id))
}

#[test]
fn test_initialize() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    c.initialize(&admin, &1_000_000);
    assert_eq!(c.total_supply(), 1_000_000);
    assert_eq!(c.balance(&admin), 1_000_000);
}

#[test]
fn test_transfer() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    c.initialize(&admin, &1_000);
    c.transfer(&admin, &user, &400);
    assert_eq!(c.balance(&admin), 600);
    assert_eq!(c.balance(&user), 400);
}

#[test]
fn test_mint_burn() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    c.initialize(&admin, &1_000);
    c.mint(&admin, &user, &500);
    assert_eq!(c.total_supply(), 1_500);
    c.burn(&admin, &user, &200);
    assert_eq!(c.total_supply(), 1_300);
}

#[test]
#[should_panic]
fn test_overdraft() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    c.initialize(&admin, &100);
    c.transfer(&admin, &user, &999);
}

#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn test_mint_non_admin() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    c.initialize(&admin, &1_000);
    c.mint(&user, &user, &500);
}

#[test]
fn test_balance_of() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let user1 = Address::generate(&env);
    let user2 = Address::generate(&env);
    c.initialize(&admin, &1_000);
    assert_eq!(c.balance(&admin), 1_000);
    assert_eq!(c.balance(&user1), 0);
    c.transfer(&admin, &user1, &300);
    assert_eq!(c.balance(&admin), 700);
    assert_eq!(c.balance(&user1), 300);
    assert_eq!(c.balance(&user2), 0);
}

#[test]
fn test_balance_of_named_function() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    c.initialize(&admin, &1_000);
    assert_eq!(c.balance_of(&admin), 1_000);
    assert_eq!(c.balance_of(&user), 0);
    c.transfer(&admin, &user, &400);
    assert_eq!(c.balance_of(&admin), 600);
    assert_eq!(c.balance_of(&user), 400);
}

#[test]
fn test_transfer_atomicity() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    c.initialize(&admin, &1_000);
    let before_total = c.total_supply();
    c.transfer(&admin, &user, &400);
    assert_eq!(c.balance(&admin) + c.balance(&user), before_total);
    assert_eq!(c.total_supply(), before_total);
}

#[test]
fn test_mint_increases_supply() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    c.initialize(&admin, &1_000);
    assert_eq!(c.total_supply(), 1_000);
    c.mint(&admin, &user, &500);
    assert_eq!(c.balance(&user), 500);
    assert_eq!(c.total_supply(), 1_500);
}

#[test]
fn test_events_mint() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    c.initialize(&admin, &0);
    c.mint(&admin, &user, &300);
    let events = env.events().all();
    assert!(events.iter().any(|(_, topics, data)| {
        topics == (symbol_short!("mint"), user.clone()).into_val(&env)
            && i128::try_from_val(&env, &data).ok() == Some(300_i128)
    }));
}

#[test]
fn test_events_transfer() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    c.initialize(&admin, &1_000);
    c.transfer(&admin, &user, &200);
    let events = env.events().all();
    assert!(events.iter().any(|(_, topics, data)| {
        topics == (symbol_short!("transfer"), admin.clone(), user.clone()).into_val(&env)
            && i128::try_from_val(&env, &data).ok() == Some(200_i128)
    }));
}

// ── TEST-005: ContractError variant coverage ──────────────────────────────────

// AdminNotSet (1): call mint before initialise
#[test]
#[should_panic]
fn test_error_admin_not_set() {
    let (env, c) = setup();
    let user = Address::generate(&env);
    c.mint(&user, &user, &100); // no initialize → AdminNotSet
}

// InvalidAmount (3): transfer zero
#[test]
#[should_panic]
fn test_error_invalid_amount_transfer_zero() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    c.initialize(&admin, &1_000);
    c.transfer(&admin, &user, &0);
}

// InvalidAmount (3): mint zero
#[test]
#[should_panic]
fn test_error_invalid_amount_mint_zero() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    c.initialize(&admin, &1_000);
    c.mint(&admin, &user, &0);
}

// AllowanceExceeded (5)
#[test]
#[should_panic]
fn test_error_allowance_exceeded() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let spender = Address::generate(&env);
    let recipient = Address::generate(&env);
    c.initialize(&admin, &1_000);
    c.approve(&admin, &spender, &50);
    c.transfer_from(&spender, &admin, &recipient, &100); // exceeds allowance of 50
}

// ── end TEST-005 ──────────────────────────────────────────────────────────────

#[test]
fn test_events_burn() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    c.initialize(&admin, &1_000);
    c.burn(&admin, &admin, &400);
    let events = env.events().all();
    assert!(events.iter().any(|(_, topics, data)| {
        topics == (symbol_short!("burn"), admin.clone()).into_val(&env)
            && i128::try_from_val(&env, &data).ok() == Some(400_i128)
    }));
}

#[test]
fn test_transfer_to_self_is_noop() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    c.initialize(&admin, &1_000);
    // Transfer to self should succeed without changing balances.
    c.transfer(&admin, &admin, &400);
    assert_eq!(c.balance(&admin), 1_000);
    assert_eq!(c.total_supply(), 1_000);
    // No transfer event should be emitted for a self-transfer.
    let events = env.events().all();
    assert!(!events.iter().any(|(_, topics, _)| {
        topics == (symbol_short!("transfer"), admin.clone(), admin.clone()).into_val(&env)
    }));
}

#[test]
#[should_panic]
fn test_transfer_zero_amount() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    c.initialize(&admin, &1_000);
    // Zero amount must be rejected.
    c.transfer(&admin, &user, &0);
}

#[test]
#[should_panic]
fn test_transfer_negative_amount() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    c.initialize(&admin, &1_000);
    // Negative amount must be rejected.
    c.transfer(&admin, &user, &-1);
}

#[test]
fn test_initialize_zero_address_reverts() {
    let (env, c) = setup();
    let zero = Address::from_str(
        &env,
        "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
    );
    c.initialize(&zero, &1_000);
}

#[test]
fn test_get_version() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    c.initialize(&admin, &1_000);
    assert_eq!(c.get_version(), (1, 0, 0));
}

#[test]
fn test_approve_sets_allowance_and_allows_transfer_from() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let spender = Address::generate(&env);
    let recipient = Address::generate(&env);

    c.initialize(&admin, &1_000);
    c.approve(&admin, &spender, &500);
    assert_eq!(allowance(&env, &admin, &spender), 500);

    c.transfer_from(&spender, &admin, &recipient, &200);
    assert_eq!(c.balance(&admin), 800);
    assert_eq!(c.balance(&recipient), 200);
    assert_eq!(allowance(&env, &admin, &spender), 300);
}

#[test]
#[should_panic]
fn test_transfer_from_insufficient_allowance() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let spender = Address::generate(&env);
    let recipient = Address::generate(&env);

    c.initialize(&admin, &1_000);
    c.approve(&admin, &spender, &100);
    c.transfer_from(&spender, &admin, &recipient, &200);
}

#[test]
#[should_panic]
fn test_transfer_from_insufficient_balance() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let spender = Address::generate(&env);
    let recipient = Address::generate(&env);

    c.initialize(&admin, &100);
    c.approve(&admin, &spender, &500);
    c.transfer_from(&spender, &admin, &recipient, &200);
}

#[test]
#[should_panic]
fn test_approve_zero_address_reverts() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let zero = Address::from_str(
        &env,
        "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
    );
    c.initialize(&admin, &1_000);
    c.approve(&admin, &zero, &100);
}

#[test]
fn test_transfer_event_emitted() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    c.initialize(&admin, &1_000);
    c.transfer(&admin, &user, &250);
    let events = env.events().all();
    assert!(events.iter().any(|(_, topics, data)| {
        topics == (symbol_short!("transfer"), admin.clone(), user.clone()).into_val(&env)
            && i128::try_from_val(&env, &data).ok() == Some(250_i128)
    }));
}

// ── SC-017: transfer_admin tests ──────────────────────────────────────────────

#[test]
fn test_transfer_admin_success() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    c.initialize(&admin, &1_000);
    c.transfer_admin(&admin, &new_admin);
    // new admin can now mint (proves privilege transfer)
    let user = Address::generate(&env);
    c.mint(&new_admin, &user, &500);
    assert_eq!(c.balance(&user), 500);
}

#[test]
fn test_transfer_admin_old_admin_loses_privileges() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    c.initialize(&admin, &1_000);
    c.transfer_admin(&admin, &new_admin);
    // new admin can now mint (proves privilege transfer)
    let user = Address::generate(&env);
    c.mint(&new_admin, &user, &500);
    assert_eq!(c.balance(&user), 500);
}

#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn test_transfer_admin_old_admin_cannot_mint() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    c.initialize(&admin, &1_000);
    c.transfer_admin(&admin, &new_admin);
    // old admin can no longer mint — must panic
    let user = Address::generate(&env);
    c.mint(&admin, &user, &100);
}

#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn test_transfer_admin_non_admin_reverts() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let non_admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    c.initialize(&admin, &1_000);
    c.transfer_admin(&non_admin, &new_admin);
}

#[test]
#[should_panic]
fn test_transfer_admin_zero_address_reverts() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    c.initialize(&admin, &1_000);
    let zero = Address::from_str(
        &env,
        "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
    );
    c.transfer_admin(&admin, &zero);
}

#[test]
fn test_transfer_admin_emits_event() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    c.initialize(&admin, &1_000);
    c.transfer_admin(&admin, &new_admin);
    let events = env.events().all();
    assert!(events.iter().any(|(_, topics, data)| {
        topics == (symbol_short!("admxfer"),).into_val(&env)
            && <(Address, Address)>::try_from_val(&env, &data)
                .ok()
                .as_ref()
                == Some(&(admin.clone(), new_admin.clone()))
    }));
}

// ── end SC-017 ────────────────────────────────────────────────────────────────

// ── SC-015: freeze/unfreeze tests ─────────────────────────────────────────────

#[test]
fn test_freeze_prevents_transfer_from_frozen_sender() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let recipient = Address::generate(&env);
    c.initialize(&admin, &1_000);
    c.transfer(&admin, &user, &500);
    c.freeze(&admin, &user);
    assert!(c.is_frozen(&user));
    let result = c.try_transfer(&user, &recipient, &100);
    assert!(result.is_err());
}

#[test]
fn test_freeze_prevents_transfer_to_frozen_recipient() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let recipient = Address::generate(&env);
    c.initialize(&admin, &1_000);
    c.freeze(&admin, &recipient);
    let result = c.try_transfer(&admin, &recipient, &100);
    assert!(result.is_err());
}

#[test]
fn test_unfreeze_restores_transfer() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let recipient = Address::generate(&env);
    c.initialize(&admin, &1_000);
    c.transfer(&admin, &user, &500);
    c.freeze(&admin, &user);
    c.unfreeze(&admin, &user);
    assert!(!c.is_frozen(&user));
    c.transfer(&user, &recipient, &100);
    assert_eq!(c.balance(&recipient), 100);
}

#[test]
fn test_freeze_prevents_mint_to_frozen_address() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    c.initialize(&admin, &0);
    c.freeze(&admin, &user);
    let result = c.try_mint(&admin, &user, &500);
    assert!(result.is_err());
}

#[test]
fn test_freeze_prevents_burn_from_frozen_address() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    c.initialize(&admin, &1_000);
    c.mint(&admin, &user, &500);
    c.freeze(&admin, &user);
    let result = c.try_burn(&admin, &user, &100);
    assert!(result.is_err());
}

#[test]
fn test_freeze_prevents_transfer_from_for_frozen_owner() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let spender = Address::generate(&env);
    let recipient = Address::generate(&env);
    c.initialize(&admin, &1_000);
    c.approve(&admin, &spender, &500);
    c.freeze(&admin, &admin);
    let result = c.try_transfer_from(&spender, &admin, &recipient, &100);
    assert!(result.is_err());
}

#[test]
fn test_freeze_does_not_prevent_balance_read() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    c.initialize(&admin, &1_000);
    c.mint(&admin, &user, &400);
    c.freeze(&admin, &user);
    // Balance must still be readable (for voting)
    assert_eq!(c.balance(&user), 400);
}

#[test]
fn test_freeze_emits_event() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    c.initialize(&admin, &0);
    c.freeze(&admin, &user);
    let events = env.events().all();
    assert!(events.iter().any(|(_, topics, _)| {
        topics == (symbol_short!("freeze"), user.clone()).into_val(&env)
    }));
}

#[test]
fn test_unfreeze_emits_event() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    c.initialize(&admin, &0);
    c.freeze(&admin, &user);
    c.unfreeze(&admin, &user);
    let events = env.events().all();
    assert!(events.iter().any(|(_, topics, _)| {
        topics == (symbol_short!("unfreeze"), user.clone()).into_val(&env)
    }));
}

#[test]
#[should_panic]
fn test_freeze_non_admin_reverts() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let non_admin = Address::generate(&env);
    let target = Address::generate(&env);
    c.initialize(&admin, &0);
    c.freeze(&non_admin, &target);
}

#[test]
#[should_panic]
fn test_unfreeze_non_admin_reverts() {
    let (env, c) = setup();
    let admin = Address::generate(&env);
    let non_admin = Address::generate(&env);
    let target = Address::generate(&env);
    c.initialize(&admin, &0);
    c.freeze(&admin, &target);
    c.unfreeze(&non_admin, &target);
}

// ── end SC-015 ────────────────────────────────────────────────────────────────
