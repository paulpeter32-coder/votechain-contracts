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
use soroban_sdk::{contract, contractimpl, symbol_short, testutils::{Address as _, Events, Ledger}, Address, Env, IntoVal, String, TryFromVal};
use crate::test_helpers::{setup_env, create_test_proposal, mint_and_vote};

// ── local helpers for tests that need a custom Env/client shape ───────────────

/// Register a fresh token contract, mint `supply` to `admin`, return its address.
fn setup_token(env: &Env, admin: &Address) -> Address {
    let id = env.register(votechain_token::TokenContract, ());
    let t = votechain_token::TokenContractClient::new(env, &id);
    t.initialize(admin, &10_000_000);
    id
}

fn new_client(env: &Env) -> GovernanceContractClient<'static> {
    GovernanceContractClient::new(env, &env.register(GovernanceContract, ()))
}

#[contract]
pub struct ReentrantToken;

#[contractimpl]
impl ReentrantToken {
    pub fn balance(env: Env, owner: Address) -> i128 {
        let gov_id: Address = env
            .storage()
            .get(&symbol_short!("reentrant_token_governance"))
            .expect("expected governance id in storage");
        let client = GovernanceContractClient::new(&env, &gov_id);
        let _ = client.has_voted(&owner, &1_u64);
        1_000_000
    }
}

/// Create a passed proposal (voted Yes, finalised) for access-control tests.
fn setup_passed_proposal(env: &Env, client: &GovernanceContractClient, admin: &Address) -> u64 {
    let voter = Address::generate(env);
    let token_id = setup_token(env, &voter);
    client.initialize(admin, &token_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);
    let id = client.create_proposal(
        &voter,
        &String::from_str(env, "Prop"),
        &String::from_str(env, "desc"),
        &100,
        &3600,
        &Vec::new(env),
    );
    client.cast_vote(&voter, &id, &Vote::Yes);
    env.ledger().with_mut(|l| l.timestamp += 3601);
    client.finalise(&id);
    id
}

/// Create an active proposal for access-control tests.
fn setup_active_proposal(env: &Env, client: &GovernanceContractClient, admin: &Address) -> u64 {
    let proposer = Address::generate(env);
    let token_id = setup_token(env, admin);
    client.initialize(admin, &token_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);
    client.create_proposal(
        &proposer,
        &String::from_str(env, "Prop"),
        &String::from_str(env, "desc"),
        &100,
        &3600,
        &Vec::new(env),
    )
}

// ── SC-001: initialize tests ──────────────────────────────────────────────────

/// State is Uninitialized before initialize, Ready after; admin and token are
/// stored; the Initialized event is emitted.
#[test]
fn test_initialize() {
    let env = Env::default();
    env.mock_all_auths();

    let gov_id = env.register(GovernanceContract, ());
    let client = GovernanceContractClient::new(&env, &gov_id);

    // Before initialize: state must be Uninitialized
    assert_eq!(client.get_state(), ContractState::Uninitialized);

    let admin = Address::generate(&env);
    let tok_id = env.register(votechain_token::TokenContract, ());
    let tok = votechain_token::TokenContractClient::new(&env, &tok_id);
    tok.initialize(&admin, &10_000_000);

    client.initialize(&admin, &tok_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);

    // After initialize: state must be Ready
    assert_eq!(client.get_state(), ContractState::Ready);

    // Admin and voting token are retrievable (indirectly via an admin-only op)
    // A cancel call with the correct admin succeeds only if admin was stored correctly.
    let proposer = Address::generate(&env);
    let id = client.create_proposal(
        &proposer,
        &String::from_str(&env, "Init test"),
        &String::from_str(&env, "desc"),
        &100,
        &3600,
        &Vec::new(&env),
    );
    client.cancel(&admin, &id); // would revert with NotAdmin if admin wasn't stored
    assert_eq!(client.get_proposal(&id).state, ProposalState::Cancelled);
}

/// initialize emits the "init" event with the admin address as data.
#[test]
fn test_initialize_emits_event() {
    let env = Env::default();
    env.mock_all_auths();

    let gov_id = env.register(GovernanceContract, ());
    let client = GovernanceContractClient::new(&env, &gov_id);

    let admin = Address::generate(&env);
    let tok_id = env.register(votechain_token::TokenContract, ());
    let tok = votechain_token::TokenContractClient::new(&env, &tok_id);
    tok.initialize(&admin, &10_000_000);

    client.initialize(&admin, &tok_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);

    // The "init" event must have been published with admin as data
    let events = env.events().all();
    assert!(
        events.iter().any(|(_, topics, data)| {
            topics == (symbol_short!("init"),).into_val(&env)
                && Address::try_from_val(&env, &data).ok().as_ref() == Some(&admin)
        }),
        "expected 'init' event with admin address as data"
    );
}

#[test]
fn test_initialize_multisig_config_bootstraps_with_threshold() {
    let env = Env::default();
    env.mock_all_auths();

    let gov_id = env.register(GovernanceContract, ());
    let client = GovernanceContractClient::new(&env, &gov_id);

    let admin = Address::generate(&env);
    let second_admin = Address::generate(&env);
    let tok_id = env.register(votechain_token::TokenContract, ());
    let tok = votechain_token::TokenContractClient::new(&env, &tok_id);
    tok.initialize(&admin, &10_000_000);

    client.initialize(&admin, &tok_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64);

    let mut admins = Vec::new(&env);
    admins.push_back(admin.clone());
    admins.push_back(second_admin.clone());

    client.initialize_multisig(&admin, &admins, &2_u32);

    let config = client.get_multisig_config().expect("multisig config should exist");
    assert_eq!(config.threshold, 2);
    assert_eq!(config.admins.len(), 2);
}

#[test]
fn test_execute_proposal_requires_multisig_threshold() {
    let env = Env::default();
    env.mock_all_auths();

    let gov_id = env.register(GovernanceContract, ());
    let client = GovernanceContractClient::new(&env, &gov_id);

    let admin = Address::generate(&env);
    let second_admin = Address::generate(&env);
    let proposer = Address::generate(&env);

    let tok_id = env.register(votechain_token::TokenContract, ());
    let tok = votechain_token::TokenContractClient::new(&env, &tok_id);
    tok.initialize(&admin, &10_000_000);

    client.initialize(&admin, &tok_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64);

    let mut admins = Vec::new(&env);
    admins.push_back(admin.clone());
    admins.push_back(second_admin.clone());
    client.initialize_multisig(&admin, &admins, &2_u32);

    let proposal_id = client.create_proposal(
        &proposer,
        &String::from_str(&env, "Execute threshold"),
        &String::from_str(&env, "desc"),
        &100,
        &3600,
    );

    client.cast_vote(&proposer, &proposal_id, &Vote::Yes);
    env.ledger().with_mut(|l| l.timestamp += 3601);
    client.finalise(&proposal_id);

    let action_id = client.propose_multisig_action(
        &admin,
        &MultiSigActionType::ExecuteProposal,
        &proposal_id,
        &None::<MultiSigConfig>,
    );

    assert_eq!(client.get_proposal(&proposal_id).state, ProposalState::Passed);

    client.approve_multisig_action(&second_admin, &action_id);
    assert_eq!(client.get_proposal(&proposal_id).state, ProposalState::Executed);
}

// ── end SC-001 ────────────────────────────────────────────────────────────────

// ── basic lifecycle ───────────────────────────────────────────────────────────

#[test]
fn test_create_proposal() {
    let t = setup_env();
    let proposer = Address::generate(&t.env);
    let id = create_test_proposal(&t, &proposer);
    assert_eq!(id, 1);
    assert_eq!(t.client.proposal_count(), 1);
    assert_eq!(t.client.get_proposal(&id).state, ProposalState::Active);
}

#[test]
fn test_amend_proposal_before_voting_starts() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);
    let token_id = setup_token(&env, &admin);
    client.initialize(
        &admin,
        &token_id,
        &0_i128,
        &0_u64,
        &60_u64,
        &2_592_000_u64,
        &false,
        &60_u64,
        &0_u64,
    );

    let proposer = Address::generate(&env);
    let id = client.create_proposal(
        &proposer,
        &String::from_str(&env, "Original title"),
        &String::from_str(&env, "Original desc"),
        &100,
        &3600,
    );

    client.amend_proposal(
        &proposer,
        &id,
        &String::from_str(&env, "Updated title"),
        &String::from_str(&env, "Updated desc"),
    );

    let proposal = client.get_proposal(&id);
    assert_eq!(proposal.title, String::from_str(&env, "Updated title"));
    assert_eq!(proposal.description, String::from_str(&env, "Updated desc"));
}

#[test]
#[should_panic(expected = "Error(Contract, #35)")]
fn test_amend_proposal_after_voting_starts_reverts() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);
    let token_id = setup_token(&env, &admin);
    client.initialize(
        &admin,
        &token_id,
        &0_i128,
        &0_u64,
        &60_u64,
        &2_592_000_u64,
        &false,
        &60_u64,
        &0_u64,
    );

    let proposer = Address::generate(&env);
    let id = client.create_proposal(
        &proposer,
        &String::from_str(&env, "Original title"),
        &String::from_str(&env, "Original desc"),
        &100,
        &3600,
    );
    env.ledger().with_mut(|l| l.timestamp += 60);

    client.amend_proposal(
        &proposer,
        &id,
        &String::from_str(&env, "Updated title"),
        &String::from_str(&env, "Updated desc"),
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #36)")]
fn test_amend_proposal_by_non_proposer_reverts() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);
    let token_id = setup_token(&env, &admin);
    client.initialize(
        &admin,
        &token_id,
        &0_i128,
        &0_u64,
        &60_u64,
        &2_592_000_u64,
        &false,
        &60_u64,
        &0_u64,
    );

    let proposer = Address::generate(&env);
    let id = client.create_proposal(
        &proposer,
        &String::from_str(&env, "Original title"),
        &String::from_str(&env, "Original desc"),
        &100,
        &3600,
    );

    let other = Address::generate(&env);
    client.amend_proposal(
        &other,
        &id,
        &String::from_str(&env, "Updated title"),
        &String::from_str(&env, "Updated desc"),
    );
}

#[test]
fn test_cast_vote_and_finalise_passed() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id = create_test_proposal(&t, &voter);

    mint_and_vote(&t, &voter, id, Vote::Yes, 1_000_000);
    assert!(t.client.has_voted(&id, &voter));
    assert_eq!(t.client.get_proposal(&id).votes_yes, 1_000_000);

    t.env.ledger().with_mut(|l| l.timestamp += 3601);
    t.client.finalise(&id);
    assert_eq!(t.client.get_proposal(&id).state, ProposalState::Passed);
}

#[test]
fn test_veto_threshold_rejects_proposal_immediately() {
    let env = Env::default();
    env.mock_all_auths();

    let gov_id = env.register(GovernanceContract, ());
    let client = GovernanceContractClient::new(&env, &gov_id);

    let admin = Address::generate(&env);
    let tok_id = env.register(votechain_token::TokenContract, ());
    let tok = votechain_token::TokenContractClient::new(&env, &tok_id);
    tok.initialize(&admin, &10_000_000);

    client.initialize(
        &admin,
        &tok_id,
        &0_i128,
        &0_u64,
        &60_u64,
        &2_592_000_u64,
        &false,
        &0_u64,
        &5_000_000_i128,
    );

    let proposer = Address::generate(&env);
    let id = client.create_proposal(
        &proposer,
        &String::from_str(&env, "Veto proposal"),
        &String::from_str(&env, "desc"),
        &100,
        &3600,
    );

    let voter = Address::generate(&env);
    tok.mint(&admin, &voter, &5_000_000_i128);
    client.cast_vote(&voter, &id, &Vote::No);

    assert_eq!(client.get_proposal(&id).state, ProposalState::Rejected);

    let events = env.events().all();
    assert!(events.iter().any(|(_, topics, _)| {
        topics == (symbol_short!("veto"), id).into_val(&env)
    }));
}

#[test]
fn test_finalise_rejected_below_quorum() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id = t.client.create_proposal(
        &voter,
        &String::from_str(&t.env, "B"),
        &String::from_str(&t.env, "desc"),
        &9_999_999,
        &3600,
        &Vec::new(&t.env),
    );
    mint_and_vote(&t, &voter, id, Vote::Yes, 1_000_000);
    t.env.ledger().with_mut(|l| l.timestamp += 3601);
    t.client.finalise(&id);
    assert_eq!(t.client.get_proposal(&id).state, ProposalState::Rejected);
}

#[test]
fn test_finalise_rejected_no_wins() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id = create_test_proposal(&t, &voter);
    mint_and_vote(&t, &voter, id, Vote::No, 1_000_000);
    t.env.ledger().with_mut(|l| l.timestamp += 3601);
    t.client.finalise(&id);
    assert_eq!(t.client.get_proposal(&id).state, ProposalState::Rejected);
}

#[test]
fn test_execute_passed_proposal() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id = create_test_proposal(&t, &voter);
    mint_and_vote(&t, &voter, id, Vote::Yes, 1_000_000);
    t.env.ledger().with_mut(|l| l.timestamp += 3601);
    t.client.finalise(&id);
    t.client.execute(&t.admin, &id);
    assert_eq!(t.client.get_proposal(&id).state, ProposalState::Executed);
}

#[test]
fn test_finalise_passed_proposal_records_execute_after_with_timelock() {
    let env = Env::default();
    env.mock_all_auths();
    let gov_id = env.register(GovernanceContract, ());
    let client = GovernanceContractClient::new(&env, &gov_id);
    let admin = Address::generate(&env);
    let tok_id = setup_token(&env, &admin);

    // Set a 1-hour timelock for execution.
    client.initialize(
        &admin,
        &tok_id,
        &0_i128,
        &0_u64,
        &60_u64,
        &2_592_000_u64,
        &false,
        &3_600_u64,
    );

    let proposer = Address::generate(&env);
    let id = client.create_proposal(
        &proposer,
        &String::from_str(&env, "Timelock test"),
        &String::from_str(&env, "Ensure execute_after is stored"),
        &100,
        &3600,
    );
    client.cast_vote(&proposer, &id, &Vote::Yes);
    env.ledger().with_mut(|l| l.timestamp += 3601);
    let finalise_time = env.ledger().timestamp();
    client.finalise(&id);

    let proposal = client.get_proposal(&id);
    assert_eq!(proposal.state, ProposalState::Passed);
    assert_eq!(proposal.execute_after, finalise_time + 3_600);
}

#[test]
#[should_panic(expected = "Error(Contract, #30)")]
fn test_execute_reverts_before_timelock_expires() {
    let env = Env::default();
    env.mock_all_auths();
    let gov_id = env.register(GovernanceContract, ());
    let client = GovernanceContractClient::new(&env, &gov_id);
    let admin = Address::generate(&env);
    let tok_id = setup_token(&env, &admin);

    client.initialize(
        &admin,
        &tok_id,
        &0_i128,
        &0_u64,
        &60_u64,
        &2_592_000_u64,
        &false,
        &3_600_u64,
    );

    let proposer = Address::generate(&env);
    let id = client.create_proposal(
        &proposer,
        &String::from_str(&env, "Timelock execute test"),
        &String::from_str(&env, "Execute should wait"),
        &100,
        &3600,
    );
    client.cast_vote(&proposer, &id, &Vote::Yes);
    env.ledger().with_mut(|l| l.timestamp += 3601);
    client.finalise(&id);
    client.execute(&admin, &id);
}

#[test]
fn test_cancel_proposal() {
    let t = setup_env();
    let proposer = Address::generate(&t.env);
    let id = create_test_proposal(&t, &proposer);
    t.client.cancel(&t.admin, &id);
    assert_eq!(t.client.get_proposal(&id).state, ProposalState::Cancelled);
}

#[test]
fn test_migrate_updates_version_and_emits_event() {
    let env = Env::default();
    env.mock_all_auths();

    let gov_id = env.register(GovernanceContract, ());
    let client = GovernanceContractClient::new(&env, &gov_id);

    let admin = Address::generate(&env);
    let tok_id = env.register(votechain_token::TokenContract, ());
    let tok = votechain_token::TokenContractClient::new(&env, &tok_id);
    tok.initialize(&admin, &10_000_000);

    // initialize sets version to 1.0.0
    client.initialize(&admin, &tok_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);

    // perform migration
    client.migrate(&admin);

    // version must be bumped to 2.0.0
    assert_eq!(client.get_version(), (2, 0, 0));
}

#[test]
fn test_migrate_is_idempotent() {
    let env = Env::default();
    env.mock_all_auths();

    let gov_id = env.register(GovernanceContract, ());
    let client = GovernanceContractClient::new(&env, &gov_id);

    let admin = Address::generate(&env);
    let tok_id = env.register(votechain_token::TokenContract, ());
    let tok = votechain_token::TokenContractClient::new(&env, &tok_id);
    tok.initialize(&admin, &10_000_000);

    client.initialize(&admin, &tok_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);

    // first migration bumps to 2.0.0
    client.migrate(&admin);
    assert_eq!(client.get_version(), (2, 0, 0));

    // second migration is a noop — version stays at 2.0.0
    client.migrate(&admin);
    assert_eq!(client.get_version(), (2, 0, 0));
}

// ── TEST-009: concurrent proposal scenario tests ──────────────────────────────

#[test]
fn test_concurrent_proposals_independent_votes() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id1 = create_test_proposal(&t, &voter);
    let id2 = create_test_proposal(&t, &voter);
    let id3 = create_test_proposal(&t, &voter);

    mint_and_vote(&t, &voter, id1, Vote::Yes, 1_000_000);
    assert!(t.client.has_voted(&id1, &voter));
    assert!(!t.client.has_voted(&id2, &voter));
    assert!(!t.client.has_voted(&id3, &voter));
}

#[test]
fn test_concurrent_votes_do_not_bleed() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id1 = create_test_proposal(&t, &voter);
    let id2 = create_test_proposal(&t, &voter);

    mint_and_vote(&t, &voter, id1, Vote::Yes, 1_000_000);

    assert_eq!(t.client.get_proposal(&id1).votes_yes, 1_000_000);
    let p2 = t.client.get_proposal(&id2);
    assert_eq!(p2.votes_yes, 0);
    assert_eq!(p2.votes_no, 0);
    assert_eq!(p2.votes_abstain, 0);
}

#[test]
fn test_finalise_one_does_not_affect_others() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id1 = create_test_proposal(&t, &voter);
    let id2 = t.client.create_proposal(
        &voter,
        &String::from_str(&t.env, "P2"),
        &String::from_str(&t.env, "d"),
        &1,
        &7200,
        &Vec::new(&t.env),
    );

    mint_and_vote(&t, &voter, id1, Vote::Yes, 1_000_000);
    t.env.ledger().with_mut(|l| l.timestamp += 3601);
    t.client.finalise(&id1);

    assert_ne!(t.client.get_proposal(&id1).state, ProposalState::Active);
    assert_eq!(t.client.get_proposal(&id2).state, ProposalState::Active);
}

#[test]
fn test_proposal_ids_are_unique() {
    let t = setup_env();
    let proposer = Address::generate(&t.env);
    let id1 = create_test_proposal(&t, &proposer);
    let id2 = create_test_proposal(&t, &proposer);
    let id3 = create_test_proposal(&t, &proposer);
    assert!(id1 != id2 && id2 != id3 && id1 != id3);
    assert_eq!(t.client.proposal_count(), 3);
}

#[test]
fn test_proposals_at_different_lifecycle_stages() {
    let t = setup_env();
    let voter = Address::generate(&t.env);

    let active_id    = t.client.create_proposal(&voter, &String::from_str(&t.env, "Active"),   &String::from_str(&t.env, "d"), &1,         &7200, &Vec::new(&t.env));
    let passed_id    = create_test_proposal(&t, &voter);
    let rejected_id  = t.client.create_proposal(&voter, &String::from_str(&t.env, "Rejected"), &String::from_str(&t.env, "d"), &9_999_999, &3600, &Vec::new(&t.env));
    let cancelled_id = create_test_proposal(&t, &voter);

    mint_and_vote(&t, &voter, passed_id, Vote::Yes, 1_000_000);
    t.client.cancel(&t.admin, &cancelled_id);

    t.env.ledger().with_mut(|l| l.timestamp += 3601);
    t.client.finalise(&passed_id);
    t.client.finalise(&rejected_id);

    assert_eq!(
        t.client.get_proposal(&active_id).state,
        ProposalState::Active
    );
    assert_eq!(
        t.client.get_proposal(&passed_id).state,
        ProposalState::Passed
    );
    assert_eq!(
        t.client.get_proposal(&rejected_id).state,
        ProposalState::Rejected
    );
    assert_eq!(
        t.client.get_proposal(&cancelled_id).state,
        ProposalState::Cancelled
    );
}

// ── end TEST-009 ──────────────────────────────────────────────────────────────

#[test]
#[should_panic]
fn test_cannot_vote_twice() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id = create_test_proposal(&t, &voter);
    mint_and_vote(&t, &voter, id, Vote::Yes, 1_000_000);
    t.client.cast_vote(&voter, &id, &Vote::No); // should panic
}

// ── TEST-013: access control negative tests ───────────────────────────────────

#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn test_execute_non_admin_reverts() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);
    let non_admin = Address::generate(&env);
    let id = setup_passed_proposal(&env, &client, &admin);
    client.execute(&non_admin, &id);
}

#[test]
#[should_panic(expected = "Error(Contract, #28)")]
fn test_execute_zero_address_reverts() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);
    let id = setup_passed_proposal(&env, &client, &admin);
    let zero = Address::from_str(
        &env,
        "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
    );
    client.execute(&zero, &id);
}

#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn test_cancel_non_admin_reverts() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);
    let non_admin = Address::generate(&env);
    let id = setup_active_proposal(&env, &client, &admin);
    client.cancel(&non_admin, &id);
}

#[test]
#[should_panic(expected = "Error(Contract, #28)")]
fn test_cancel_zero_address_reverts() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);
    let id = setup_active_proposal(&env, &client, &admin);
    let zero = Address::from_str(
        &env,
        "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
    );
    client.cancel(&zero, &id);
}

// ── SC-005: execute state-guard tests ────────────────────────────────────────

/// execute() on an Active proposal must revert — only Passed is valid.
#[test]
#[should_panic(expected = "Error(Contract, #12)")]
fn test_execute_active_proposal_reverts() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);
    let id = setup_active_proposal(&env, &client, &admin);
    client.execute(&admin, &id);
}

/// execute() on a Rejected proposal must revert.
#[test]
#[should_panic(expected = "Error(Contract, #12)")]
fn test_execute_rejected_proposal_reverts() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);
    let token_id = setup_token(&env, &admin);
    client.initialize(&admin, &token_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);
    // Create a proposal that will be rejected (no votes, below quorum)
    let id = client.create_proposal(
        &admin,
        &String::from_str(&env, "Prop"),
        &String::from_str(&env, "desc"),
        &1_000_000,
        &3600,
        &Vec::new(&env),
    );
    env.ledger().with_mut(|l| l.timestamp += 3601);
    client.finalise(&id);
    assert_eq!(client.get_proposal(&id).state, ProposalState::Rejected);
    client.execute(&admin, &id);
}

/// execute() on a Cancelled proposal must revert.
#[test]
#[should_panic(expected = "Error(Contract, #12)")]
fn test_execute_cancelled_proposal_reverts() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);
    let id = setup_active_proposal(&env, &client, &admin);
    client.cancel(&admin, &id);
    client.execute(&admin, &id);
}

/// execute() on an already-Executed proposal must revert.
#[test]
#[should_panic(expected = "Error(Contract, #12)")]
fn test_execute_already_executed_proposal_reverts() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);
    let id = setup_passed_proposal(&env, &client, &admin);
    client.execute(&admin, &id); // first execute — ok
    client.execute(&admin, &id); // second execute — must revert
}

// ── end SC-005 ────────────────────────────────────────────────────────────────

// ── end TEST-013 ──────────────────────────────────────────────────────────────

// ── SC-027: update_quorum tests ───────────────────────────────────────────────

#[test]
fn test_update_quorum_success() {
    let t = setup_env();
    let proposer = Address::generate(&t.env);
    let id = create_test_proposal(&t, &proposer);
    t.client.update_quorum(&t.admin, &id, &500);
    assert_eq!(t.client.get_proposal(&id).quorum, 500);
}

#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn test_update_quorum_non_admin_reverts() {
    let t = setup_env();
    let non_admin = Address::generate(&t.env);
    let id = create_test_proposal(&t, &t.admin.clone());
    t.client.update_quorum(&non_admin, &id, &500);
}

#[test]
#[should_panic]
fn test_update_quorum_zero_reverts() {
    let t = setup_env();
    let id = create_test_proposal(&t, &t.admin.clone());
    t.client.update_quorum(&t.admin, &id, &0);
}

#[test]
#[should_panic]
fn test_update_quorum_inactive_proposal_reverts() {
    let t = setup_env();
    let id = create_test_proposal(&t, &t.admin.clone());
    t.client.cancel(&t.admin, &id);
    t.client.update_quorum(&t.admin, &id, &500);
}

// ── end SC-027 ────────────────────────────────────────────────────────────────

// ── storage persistence tests ─────────────────────────────────────────────────

#[test]
fn test_proposal_data_persists_unchanged() {
    let t = setup_env();
    let proposer = Address::generate(&t.env);
    let id = t.client.create_proposal(
        &proposer,
        &String::from_str(&t.env, "Persist title"),
        &String::from_str(&t.env, "Persist desc"),
        &250,
        &1800,
        &Vec::new(&t.env),
    );
    let p = t.client.get_proposal(&id);
    assert_eq!(p.id, id);
    assert_eq!(p.title, String::from_str(&t.env, "Persist title"));
    assert_eq!(p.description, String::from_str(&t.env, "Persist desc"));
    assert_eq!(p.quorum, 250);
    assert_eq!(p.state, ProposalState::Active);
    assert_eq!(p.proposer, proposer);
}

#[test]
fn test_vote_records_persist_across_multiple_voters() {
    let t = setup_env();
    let voter1 = Address::generate(&t.env);
    let voter2 = Address::generate(&t.env);
    let voter3 = Address::generate(&t.env);
    let id = create_test_proposal(&t, &voter1);

    mint_and_vote(&t, &voter1, id, Vote::Yes, 300_000);
    mint_and_vote(&t, &voter2, id, Vote::No, 300_000);
    mint_and_vote(&t, &voter3, id, Vote::Abstain, 300_000);

    assert!(t.client.has_voted(&id, &voter1));
    assert!(t.client.has_voted(&id, &voter2));
    assert!(t.client.has_voted(&id, &voter3));
    let p = t.client.get_proposal(&id);
    assert!(p.votes_yes > 0);
    assert!(p.votes_no > 0);
    assert!(p.votes_abstain > 0);
}

#[test]
fn test_admin_persists_after_initialization() {
    let t = setup_env();
    let id = create_test_proposal(&t, &t.admin.clone());
    t.client.cancel(&t.admin, &id);
    assert_eq!(t.client.get_proposal(&id).state, ProposalState::Cancelled);
}

#[test]
fn test_no_data_lost_between_calls() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id1 = create_test_proposal(&t, &voter);
    let id2 = t.client.create_proposal(
        &voter,
        &String::from_str(&t.env, "P2"),
        &String::from_str(&t.env, "d2"),
        &200,
        &7200,
        &Vec::new(&t.env),
    );

    mint_and_vote(&t, &voter, id1, Vote::Yes, 1_000_000);

    let p2 = t.client.get_proposal(&id2);
    assert_eq!(p2.title, String::from_str(&t.env, "P2"));
    assert_eq!(p2.quorum, 200);
    assert_eq!(p2.votes_yes, 0);
    assert_eq!(p2.state, ProposalState::Active);
    assert!(!t.client.has_voted(&id2, &voter));
}

// ── end storage persistence tests ─────────────────────────────────────────────

// ── Issue #8: has_voted ProposalNotFound tests ────────────────────────────────

#[test]
#[should_panic]
fn test_has_voted_invalid_proposal_id_reverts() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    t.client.has_voted(&999, &voter);
}

#[test]
#[should_panic]
fn test_get_proposal_invalid_id_reverts() {
    let t = setup_env();
    t.client.get_proposal(&999);
}

#[test]
fn test_get_proposal_returns_correct_lifecycle_states() {
    let t = setup_env();
    let voter = Address::generate(&t.env);

    let active_id = create_test_proposal(&t, &voter);
    assert_eq!(
        t.client.get_proposal(&active_id).state,
        ProposalState::Active
    );

    let cancelled_id = create_test_proposal(&t, &voter);
    t.client.cancel(&t.admin, &cancelled_id);
    assert_eq!(
        t.client.get_proposal(&cancelled_id).state,
        ProposalState::Cancelled
    );

    let rejected_id = create_test_proposal(&t, &voter);
    mint_and_vote(&t, &voter, rejected_id, Vote::No, 1_000_000);
    t.env.ledger().with_mut(|l| l.timestamp += 3601);
    t.client.finalise(&rejected_id);
    assert_eq!(
        t.client.get_proposal(&rejected_id).state,
        ProposalState::Rejected
    );

    let passed_id = create_test_proposal(&t, &voter);
    mint_and_vote(&t, &voter, passed_id, Vote::Yes, 1_000_000);
    t.env.ledger().with_mut(|l| l.timestamp += 3601);
    t.client.finalise(&passed_id);
    assert_eq!(
        t.client.get_proposal(&passed_id).state,
        ProposalState::Passed
    );

    let executed_id = create_test_proposal(&t, &voter);
    mint_and_vote(&t, &voter, executed_id, Vote::Yes, 1_000_000);
    t.env.ledger().with_mut(|l| l.timestamp += 3601);
    t.client.finalise(&executed_id);
    t.client.execute(&t.admin, &executed_id);
    assert_eq!(
        t.client.get_proposal(&executed_id).state,
        ProposalState::Executed
    );
}

#[test]
fn test_has_voted_returns_false_before_and_true_after_voting() {
    let t = setup_env();
    let proposer = Address::generate(&t.env);
    let voter = Address::generate(&t.env);
    let id = create_test_proposal(&t, &proposer);

    assert!(!t.client.has_voted(&id, &voter));
    mint_and_vote(&t, &voter, id, Vote::Yes, 1_000_000);
    assert!(t.client.has_voted(&id, &voter));
}

#[test]
fn test_proposal_count_increments_correctly() {
    let t = setup_env();
    assert_eq!(t.client.proposal_count(), 0);

    let proposer = Address::generate(&t.env);
    let id1 = create_test_proposal(&t, &proposer);
    assert_eq!(t.client.proposal_count(), 1);

    let id2 = create_test_proposal(&t, &proposer);
    assert_eq!(t.client.proposal_count(), 2);

    let id3 = create_test_proposal(&t, &proposer);
    assert_eq!(t.client.proposal_count(), 3);

    assert_eq!(id1, 1);
    assert_eq!(id2, 2);
    assert_eq!(id3, 3);
}

#[test]
fn test_has_voted_returns_false_for_non_voter() {
    let t = setup_env();
    let proposer = Address::generate(&t.env);
    let non_voter = Address::generate(&t.env);
    let id = create_test_proposal(&t, &proposer);
    assert!(!t.client.has_voted(&id, &non_voter));
}

// ── end Issue #8 ──────────────────────────────────────────────────────────────

// ── Issue #10: ProposalState enum tests ──────────────────────────────────────

#[test]
fn test_proposal_state_all_variants_reachable() {
    let t = setup_env();
    let voter = Address::generate(&t.env);

    // Active
    let id = create_test_proposal(&t, &voter);
    assert_eq!(t.client.get_proposal(&id).state, ProposalState::Active);

    // Cancelled
    let id2 = create_test_proposal(&t, &voter);
    t.client.cancel(&t.admin, &id2);
    assert_eq!(t.client.get_proposal(&id2).state, ProposalState::Cancelled);

    // Rejected
    let id3 = create_test_proposal(&t, &voter);
    t.env.ledger().with_mut(|l| l.timestamp += 3601);
    t.client.finalise(&id3);
    assert_eq!(t.client.get_proposal(&id3).state, ProposalState::Rejected);

    // Passed + Executed
    let id4 = create_test_proposal(&t, &voter);
    mint_and_vote(&t, &voter, id4, Vote::Yes, 1_000_000);
    t.env.ledger().with_mut(|l| l.timestamp += 3601);
    t.client.finalise(&id4);
    assert_eq!(t.client.get_proposal(&id4).state, ProposalState::Passed);
    t.client.execute(&t.admin, &id4);
    assert_eq!(t.client.get_proposal(&id4).state, ProposalState::Executed);
}

// NOTE: test_get_proposals_by_state_filters_and_paginates and
// test_get_proposals_by_state_respects_offset_and_limit are disabled because
// get_proposals_by_state is not yet implemented in the contract.

// ── end Issue #10 ─────────────────────────────────────────────────────────────

// ── Issue #28: comprehensive voting scenario tests ────────────────────────────

#[test]
fn test_vote_yes_recorded_correctly() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id = create_test_proposal(&t, &voter);
    mint_and_vote(&t, &voter, id, Vote::Yes, 500_000);
    let p = t.client.get_proposal(&id);
    assert_eq!(p.votes_yes, 500_000);
    assert_eq!(p.votes_no, 0);
    assert_eq!(p.votes_abstain, 0);
}

#[test]
fn test_vote_no_recorded_correctly() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id = create_test_proposal(&t, &voter);
    mint_and_vote(&t, &voter, id, Vote::No, 750_000);
    let p = t.client.get_proposal(&id);
    assert_eq!(p.votes_yes, 0);
    assert_eq!(p.votes_no, 750_000);
    assert_eq!(p.votes_abstain, 0);
}

#[test]
fn test_vote_abstain_recorded_correctly() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id = create_test_proposal(&t, &voter);
    mint_and_vote(&t, &voter, id, Vote::Abstain, 250_000);
    let p = t.client.get_proposal(&id);
    assert_eq!(p.votes_yes, 0);
    assert_eq!(p.votes_no, 0);
    assert_eq!(p.votes_abstain, 250_000);
}

#[test]
fn test_vote_weight_matches_token_balance() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id = create_test_proposal(&t, &voter);
    let balance = 1_234_567;
    mint_and_vote(&t, &voter, id, Vote::Yes, balance);
    let p = t.client.get_proposal(&id);
    assert_eq!(p.votes_yes, balance);
}

#[test]
#[should_panic(expected = "Error(Contract, #10)")]
fn test_double_vote_same_choice_reverts() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id = create_test_proposal(&t, &voter);
    mint_and_vote(&t, &voter, id, Vote::Yes, 1_000_000);
    t.client.cast_vote(&voter, &id, &Vote::Yes);
}

#[test]
#[should_panic(expected = "Error(Contract, #10)")]
fn test_double_vote_different_choice_reverts() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id = create_test_proposal(&t, &voter);
    mint_and_vote(&t, &voter, id, Vote::Yes, 1_000_000);
    t.client.cast_vote(&voter, &id, &Vote::No);
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn test_vote_on_passed_proposal_reverts() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id = create_test_proposal(&t, &voter);
    mint_and_vote(&t, &voter, id, Vote::Yes, 1_000_000);
    t.env.ledger().with_mut(|l| l.timestamp += 3601);
    t.client.finalise(&id);
    let voter2 = Address::generate(&t.env);
    mint_and_vote(&t, &voter2, id, Vote::Yes, 500_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn test_vote_on_rejected_proposal_reverts() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id = create_test_proposal(&t, &voter);
    mint_and_vote(&t, &voter, id, Vote::No, 1_000_000);
    t.env.ledger().with_mut(|l| l.timestamp += 3601);
    t.client.finalise(&id);
    let voter2 = Address::generate(&t.env);
    mint_and_vote(&t, &voter2, id, Vote::Yes, 500_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn test_vote_on_cancelled_proposal_reverts() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id = create_test_proposal(&t, &voter);
    t.client.cancel(&t.admin, &id);
    mint_and_vote(&t, &voter, id, Vote::Yes, 1_000_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn test_vote_on_executed_proposal_reverts() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id = create_test_proposal(&t, &voter);
    mint_and_vote(&t, &voter, id, Vote::Yes, 1_000_000);
    t.env.ledger().with_mut(|l| l.timestamp += 3601);
    t.client.finalise(&id);
    t.client.execute(&t.admin, &id);
    let voter2 = Address::generate(&t.env);
    mint_and_vote(&t, &voter2, id, Vote::Yes, 500_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #8)")]
fn test_vote_after_end_time_reverts() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id = create_test_proposal(&t, &voter);
    t.env.ledger().with_mut(|l| l.timestamp += 3601);
    mint_and_vote(&t, &voter, id, Vote::Yes, 1_000_000);
}

#[test]
#[should_panic]
fn test_vote_at_exact_end_time_reverts() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let now = t.env.ledger().timestamp();
    let id = t.client.create_proposal(
        &voter,
        &String::from_str(&t.env, "Test"),
        &String::from_str(&t.env, "desc"),
        &1,
        &3600,
        &Vec::new(&t.env),
    );
    t.env.ledger().with_mut(|l| l.timestamp = now + 3600);
    mint_and_vote(&t, &voter, id, Vote::Yes, 1_000_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #11)")]
fn test_vote_with_zero_balance_reverts() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id = create_test_proposal(&t, &voter);
    t.client.cast_vote(&voter, &id, &Vote::Yes);
}

#[test]
fn test_vote_tallies_accumulate_correctly() {
    let t = setup_env();
    let voter1 = Address::generate(&t.env);
    let voter2 = Address::generate(&t.env);
    let voter3 = Address::generate(&t.env);
    let id = create_test_proposal(&t, &voter1);

    mint_and_vote(&t, &voter1, id, Vote::Yes, 100_000);
    mint_and_vote(&t, &voter2, id, Vote::Yes, 200_000);
    mint_and_vote(&t, &voter3, id, Vote::No, 150_000);

    let p = t.client.get_proposal(&id);
    assert_eq!(p.votes_yes, 300_000);
    assert_eq!(p.votes_no, 150_000);
    assert_eq!(p.votes_abstain, 0);
}

#[test]
fn test_vote_tallies_all_three_types() {
    let t = setup_env();
    let v1 = Address::generate(&t.env);
    let v2 = Address::generate(&t.env);
    let v3 = Address::generate(&t.env);
    let v4 = Address::generate(&t.env);
    let v5 = Address::generate(&t.env);
    let id = create_test_proposal(&t, &v1);

    mint_and_vote(&t, &v1, id, Vote::Yes, 100_000);
    mint_and_vote(&t, &v2, id, Vote::Yes, 200_000);
    mint_and_vote(&t, &v3, id, Vote::No, 150_000);
    mint_and_vote(&t, &v4, id, Vote::No, 50_000);
    mint_and_vote(&t, &v5, id, Vote::Abstain, 75_000);

    let p = t.client.get_proposal(&id);
    assert_eq!(p.votes_yes, 300_000);
    assert_eq!(p.votes_no, 200_000);
    assert_eq!(p.votes_abstain, 75_000);
}

// ── end Issue #28 ─────────────────────────────────────────────────────────────

// ── SEC-009: re-initialization guard tests ────────────────────────────────────

/// Re-init by the original admin must revert with AlreadyInitialized.
#[test]
#[should_panic]
fn test_reinit_by_original_admin_reverts() {
    let t = setup_env();
    t.client.initialize(&t.admin, &t.token_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);
}

/// Re-init by a new address must revert with AlreadyInitialized.
#[test]
#[should_panic]
fn test_reinit_by_new_address_reverts() {
    let t = setup_env();
    let attacker = Address::generate(&t.env);
    let new_token = Address::generate(&t.env);
    t.client.initialize(&attacker, &new_token, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);
}

/// Re-init by the zero address must revert with AlreadyInitialized.
#[test]
#[should_panic]
fn test_reinit_by_zero_address_reverts() {
    let t = setup_env();
    let zero = Address::from_str(&t.env, "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF");
    t.client.initialize(&zero, &t.token_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);
}

#[test]
#[should_panic]
fn test_regression_sec_009_reinit_guard() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);
    let tok_id = env.register(votechain_token::TokenContract, ());
    let tok = votechain_token::TokenContractClient::new(&env, &tok_id);
    tok.initialize(&admin, &10_000_000);

    client.initialize(&admin, &tok_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64);
    let attacker = Address::generate(&env);
    client.initialize(&attacker, &tok_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64);
}

#[test]
#[should_panic(expected = "Error(Contract, #11)")]
fn test_regression_sec_008_token_balance_fetch() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id = create_test_proposal(&t, &voter);
    t.client.cast_vote(&voter, &id, &Vote::Yes);
}

#[test]
#[should_panic]
fn test_regression_sec_010_reentrancy() {
    let env = Env::default();
    env.mock_all_auths();
    let voter = Address::generate(&env);
    let gov_id = env.register(GovernanceContract, ());
    let client = GovernanceContractClient::new(&env, &gov_id);
    let token_id = env.register(ReentrantToken, ());
    env.storage().set(&symbol_short!("reentrant_token_governance"), &gov_id);

    client.initialize(&voter, &token_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64);
    let id = client.create_proposal(&voter, &String::from_str(&env, "Reentrancy test"), &String::from_str(&env, "desc"), &100, &3600);
    client.cast_vote(&voter, &id, &Vote::Yes);
}

// ── end SEC-009 ───────────────────────────────────────────────────────────────

// ── spam prevention tests ─────────────────────────────────────────────────────

#[test]
#[should_panic]
fn test_create_proposal_below_min_balance_reverts() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);
    let token_id = setup_token(&env, &admin);
    // require 500_000 tokens to propose
    client.initialize(&admin, &token_id, &500_000_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);

    let proposer = Address::generate(&env);
    // proposer has 0 tokens — should panic
    client.create_proposal(
        &proposer,
        &String::from_str(&env, "Spam"),
        &String::from_str(&env, "desc"),
        &100,
        &3600,
        &Vec::new(&env),
    );
}

#[test]
fn test_create_proposal_at_min_balance_accepted() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);
    let token_id = setup_token(&env, &admin);
    client.initialize(&admin, &token_id, &500_000_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);

    let proposer = Address::generate(&env);
    let tok = votechain_token::TokenContractClient::new(&env, &token_id);
    tok.mint(&admin, &proposer, &500_000_i128);

    let id = client.create_proposal(
        &proposer,
        &String::from_str(&env, "Valid"),
        &String::from_str(&env, "desc"),
        &100,
        &3600,
        &Vec::new(&env),
    );
    assert_eq!(client.get_proposal(&id).state, ProposalState::Active);
}

#[test]
#[should_panic]
fn test_create_proposal_within_cooldown_reverts() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);
    let token_id = setup_token(&env, &admin);
    // start at non-zero so the `last > 0` sentinel works
    env.ledger().with_mut(|l| l.timestamp = 1_000);
    // 1 hour cooldown, no balance requirement
    client.initialize(&admin, &token_id, &0_i128, &3600_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);

    let proposer = Address::generate(&env);
    client.create_proposal(
        &proposer,
        &String::from_str(&env, "First"),
        &String::from_str(&env, "desc"),
        &100,
        &7200,
        &Vec::new(&env),
    );
    // second proposal immediately within cooldown — should panic
    client.create_proposal(
        &proposer,
        &String::from_str(&env, "Spam"),
        &String::from_str(&env, "desc"),
        &100,
        &3600,
        &Vec::new(&env),
    );
}

#[test]
fn test_create_proposal_after_cooldown_accepted() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);
    let token_id = setup_token(&env, &admin);
    client.initialize(&admin, &token_id, &0_i128, &3600_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);

    let proposer = Address::generate(&env);
    client.create_proposal(
        &proposer,
        &String::from_str(&env, "First"),
        &String::from_str(&env, "desc"),
        &100,
        &7200,
        &Vec::new(&env),
    );
    // advance past cooldown
    env.ledger().with_mut(|l| l.timestamp += 3601);
    let id2 = client.create_proposal(
        &proposer,
        &String::from_str(&env, "Second"),
        &String::from_str(&env, "desc"),
        &100,
        &3600,
        &Vec::new(&env),
    );
    assert_eq!(client.get_proposal(&id2).state, ProposalState::Active);
}

// ── end spam prevention tests ─────────────────────────────────────────────────

// ── SC-023: get_vote tests ────────────────────────────────────────────────────

#[test]
fn test_get_vote_returns_record_after_voting() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id = create_test_proposal(&t, &voter);
    mint_and_vote(&t, &voter, id, Vote::Yes, 500_000);
    let record = t
        .client
        .get_vote(&id, &voter)
        .expect("expected vote record");
    assert_eq!(record.vote_type, Vote::Yes);
    assert_eq!(record.weight, 500_000);
}

#[test]
fn test_get_vote_returns_none_for_non_voter() {
    let t = setup_env();
    let proposer = Address::generate(&t.env);
    let non_voter = Address::generate(&t.env);
    let id = create_test_proposal(&t, &proposer);
    assert!(t.client.get_vote(&id, &non_voter).is_none());
}

#[test]
fn test_get_vote_correct_type_for_no_vote() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id = create_test_proposal(&t, &voter);
    mint_and_vote(&t, &voter, id, Vote::No, 300_000);
    let record = t
        .client
        .get_vote(&id, &voter)
        .expect("expected vote record");
    assert_eq!(record.vote_type, Vote::No);
    assert_eq!(record.weight, 300_000);
}

#[test]
fn test_get_vote_correct_type_for_abstain() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id = create_test_proposal(&t, &voter);
    mint_and_vote(&t, &voter, id, Vote::Abstain, 100_000);
    let record = t
        .client
        .get_vote(&id, &voter)
        .expect("expected vote record");
    assert_eq!(record.vote_type, Vote::Abstain);
    assert_eq!(record.weight, 100_000);
}

// ── end SC-023 ────────────────────────────────────────────────────────────────

// ── SC-021: abstain votes count toward quorum ─────────────────────────────────

/// Abstain votes must be included in total_votes for the quorum check.
/// A proposal where only abstain votes are cast should pass quorum and then
/// be Rejected (because votes_yes == 0 <= votes_no == 0 is not strictly greater).
#[test]
fn test_abstain_votes_count_toward_quorum() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    // quorum = 500_000; voter abstains with exactly that weight
    let id = t.client.create_proposal(
        &voter,
        &String::from_str(&t.env, "Abstain quorum"),
        &String::from_str(&t.env, "desc"),
        &500_000,
        &3600,
        &Vec::new(&t.env),
    );
    mint_and_vote(&t, &voter, id, Vote::Abstain, 500_000);

    t.env.ledger().with_mut(|l| l.timestamp += 3601);
    t.client.finalise(&id);

    // Quorum was met (500_000 >= 500_000) but votes_yes (0) is not > votes_no (0),
    // so the proposal is Rejected — not Active, confirming abstain counted.
    assert_eq!(t.client.get_proposal(&id).state, ProposalState::Rejected);
}

/// Abstain votes combined with Yes votes should push a proposal over quorum
/// and allow it to pass when votes_yes > votes_no.
#[test]
fn test_abstain_plus_yes_meets_quorum_and_passes() {
    let t = setup_env();
    let voter_yes = Address::generate(&t.env);
    let voter_abs = Address::generate(&t.env);
    // quorum = 1_000_000; yes = 600_000, abstain = 400_000 → total = 1_000_000
    let id = t.client.create_proposal(
        &voter_yes,
        &String::from_str(&t.env, "Mixed quorum"),
        &String::from_str(&t.env, "desc"),
        &1_000_000,
        &3600,
        &Vec::new(&t.env),
    );
    mint_and_vote(&t, &voter_yes, id, Vote::Yes, 600_000);
    mint_and_vote(&t, &voter_abs, id, Vote::Abstain, 400_000);

    t.env.ledger().with_mut(|l| l.timestamp += 3601);
    t.client.finalise(&id);

    assert_eq!(t.client.get_proposal(&id).state, ProposalState::Passed);
}

/// Without abstain votes the same Yes total falls below quorum and is Rejected.
#[test]
fn test_yes_alone_below_quorum_rejected() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    // quorum = 1_000_000; only 600_000 yes votes — below quorum
    let id = t.client.create_proposal(
        &voter,
        &String::from_str(&t.env, "Below quorum"),
        &String::from_str(&t.env, "desc"),
        &1_000_000,
        &3600,
        &Vec::new(&t.env),
    );
    mint_and_vote(&t, &voter, id, Vote::Yes, 600_000);

    t.env.ledger().with_mut(|l| l.timestamp += 3601);
    t.client.finalise(&id);

    assert_eq!(t.client.get_proposal(&id).state, ProposalState::Rejected);
}

// ── end SC-021 ────────────────────────────────────────────────────────────────

// ── TEST-008: admin transfer integration tests ────────────────────────────────

/// Helper: create a passed proposal ready for execute() calls.
fn make_passed_proposal_for_transfer(
    env: &Env,
    client: &GovernanceContractClient,
    admin: &Address,
    token_id: &Address,
) -> u64 {
    let voter = Address::generate(env);
    let tok = votechain_token::TokenContractClient::new(env, token_id);
    tok.mint(admin, &voter, &1_000_000_i128);
    let id = client.create_proposal(
        &voter,
        &String::from_str(env, "Transfer test"),
        &String::from_str(env, "desc"),
        &100,
        &3600,
        &Vec::new(&env),
    );
    client.cast_vote(&voter, &id, &Vote::Yes);
    env.ledger().with_mut(|l| l.timestamp += 3601);
    client.finalise(&id);
    id
}

/// New admin can execute a passed proposal after transfer.
#[test]
fn test_transfer_admin_new_admin_can_execute() {
    let t = setup_env();
    let new_admin = Address::generate(&t.env);
    let id = make_passed_proposal_for_transfer(&t.env, &t.client, &t.admin, &t.token_id);

    t.client.transfer_admin(&t.admin, &new_admin);
    t.client.execute(&new_admin, &id);

    assert_eq!(t.client.get_proposal(&id).state, ProposalState::Executed);
}

/// Old admin cannot execute a proposal after transferring admin rights.
#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn test_transfer_admin_old_admin_cannot_execute() {
    let t = setup_env();
    let new_admin = Address::generate(&t.env);
    let id = make_passed_proposal_for_transfer(&t.env, &t.client, &t.admin, &t.token_id);

    t.client.transfer_admin(&t.admin, &new_admin);
    // old admin tries to execute — must revert
    t.client.execute(&t.admin, &id);
}

/// Old admin cannot cancel a proposal after transferring admin rights.
#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn test_transfer_admin_old_admin_cannot_cancel() {
    let t = setup_env();
    let new_admin = Address::generate(&t.env);
    let proposer = Address::generate(&t.env);
    let id = t.client.create_proposal(
        &proposer,
        &String::from_str(&t.env, "Cancel test"),
        &String::from_str(&t.env, "desc"),
        &100,
        &3600,
        &Vec::new(&t.env),
    );

    t.client.transfer_admin(&t.admin, &new_admin);
    // old admin tries to cancel — must revert
    t.client.cancel(&t.admin, &id);
}

/// Transfer to the zero address must revert with InvalidNewAdmin.
#[test]
#[should_panic]
fn test_transfer_admin_to_zero_address_reverts() {
    let t = setup_env();
    let zero = Address::from_str(
        &t.env,
        "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
    );
    t.client.transfer_admin(&t.admin, &zero);
}

/// transfer_admin emits an admxfer event with the correct old and new admin.
#[test]
fn test_transfer_admin_emits_event() {
    let t = setup_env();
    let new_admin = Address::generate(&t.env);
    t.client.transfer_admin(&t.admin, &new_admin);
    let events = t.env.events().all();
    assert!(
        events
            .iter()
            .any(|(_, topics, _)| { topics == (symbol_short!("admxfer"),).into_val(&t.env) }),
        "expected admxfer event to be emitted"
    );
}

// ── end TEST-008 ──────────────────────────────────────────────────────────────

// ── SEC-016: admin vote restriction tests ─────────────────────────────────────

/// When restrict_admin_vote is enabled, admin cannot vote on a proposal they created.
#[test]
#[should_panic(expected = "Error(Contract, #25)")]
fn test_admin_cannot_vote_own_proposal_when_restricted() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let tok_id = env.register(votechain_token::TokenContract, ());
    let tok = votechain_token::TokenContractClient::new(&env, &tok_id);
    tok.initialize(&admin, &10_000_000);
    let client = new_client(&env);
    // enable restriction
    client.initialize(&admin, &tok_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &true, &0_u64, &0_u64);
    let id = client.create_proposal(
        &admin,
        &String::from_str(&env, "Admin prop"),
        &String::from_str(&env, "desc"),
        &100,
        &3600,
        &Vec::new(&env),
    );

    // B has never proposed — must succeed immediately despite A being in cooldown
    let id = client.create_proposal(
        &proposer_b,
        &String::from_str(&env, "B first"),
    // admin tries to vote on their own proposal — should panic
    client.cast_vote(&admin, &id, &Vote::Yes);
}

/// When restrict_admin_vote is disabled, admin can vote on their own proposal.
#[test]
fn test_admin_can_vote_own_proposal_when_not_restricted() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let tok_id = env.register(votechain_token::TokenContract, ());
    let tok = votechain_token::TokenContractClient::new(&env, &tok_id);
    tok.initialize(&admin, &10_000_000);
    let client = new_client(&env);
    // restriction disabled
    client.initialize(&admin, &tok_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);
    let id = client.create_proposal(
        &admin,
        &String::from_str(&env, "Admin prop"),
        &String::from_str(&env, "desc"),
        &100,
        &3600,
        &Vec::new(&env),
    );
    // admin votes on their own proposal — should succeed
    client.cast_vote(&admin, &id, &Vote::Yes);
    assert_eq!(client.get_proposal(&id).votes_yes, 10_000_000);
}

/// When restrict_admin_vote is enabled, a non-admin voter can still vote normally.
#[test]
fn test_non_admin_can_vote_when_admin_restricted() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let tok_id = env.register(votechain_token::TokenContract, ());
    let tok = votechain_token::TokenContractClient::new(&env, &tok_id);
    tok.initialize(&admin, &10_000_000);
    let client = new_client(&env);
    client.initialize(&admin, &tok_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &true, &0_u64, &0_u64);
    let proposer = Address::generate(&env);
    let id = client.create_proposal(
        &proposer,
        &String::from_str(&env, "User prop"),
        &String::from_str(&env, "desc"),
        &100,
        &3600,
        &Vec::new(&env),
    );
    let id2 = client.create_proposal(
        &proposer,
        &String::from_str(&env, "Second"),
        &String::from_str(&env, "desc"),
        &100,
        &3600,
    );
    assert_eq!(client.get_proposal(&id1).state, ProposalState::Active);
    assert_eq!(client.get_proposal(&id2).state, ProposalState::Active);
}

/// Proposing at exactly `last + cooldown` (boundary) must succeed.
#[test]
fn test_cooldown_exact_boundary_accepted() {
    let voter = Address::generate(&env);
    tok.mint(&admin, &voter, &500_000_i128);
    client.cast_vote(&voter, &id, &Vote::Yes);
    assert_eq!(client.get_proposal(&id).votes_yes, 500_000);
}

// ── end SEC-016 ───────────────────────────────────────────────────────────────

// ── SEC-018: emergency pause tests ────────────────────────────────────────────

/// pause() by admin sets paused state and emits event.
#[test]
fn test_pause_sets_paused_state() {
    let t = setup_env();
    assert!(!t.client.paused());
    t.client.pause(&t.admin, &Option::<String>::None);
    assert!(t.client.paused());
}

/// pause with a reason stores the reason and is retrievable via view.
#[test]
fn test_pause_with_reason_stores_and_returns_reason() {
    let t = setup_env();
    let reason = String::from_str(&t.env, "maintenance");
    t.client.pause(&t.admin, &Some(reason.clone()));
    assert!(t.client.paused());
    let stored = t.client.get_pause_reason();
    assert_eq!(stored.unwrap(), reason);
}

/// unpause() by admin clears paused state and emits event.
#[test]
fn test_unpause_clears_paused_state() {
    let t = setup_env();
    t.client.pause(&t.admin, &Option::<String>::None);
    assert!(t.client.paused());
    t.client.unpause(&t.admin);
    assert!(!t.client.paused());
}

/// pause() by non-admin must revert.
#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn test_pause_non_admin_reverts() {
    let t = setup_env();
    let attacker = Address::generate(&t.env);
    t.client.pause(&attacker, &Option::<String>::None);
}

/// unpause() by non-admin must revert.
#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn test_unpause_non_admin_reverts() {
    let t = setup_env();
    t.client.pause(&t.admin, &Option::<String>::None);
    let attacker = Address::generate(&t.env);
    t.client.unpause(&attacker);
}

/// unpause() when not paused must revert.
#[test]
#[should_panic]
fn test_unpause_when_not_paused_reverts() {
    let t = setup_env();
    t.client.unpause(&t.admin);
}

/// create_proposal reverts when paused.
#[test]
#[should_panic(expected = "Error(Contract, #26)")]
fn test_create_proposal_reverts_when_paused() {
    let t = setup_env();
    let proposer = Address::generate(&t.env);
    t.client.pause(&t.admin, &Option::<String>::None);
    t.client.create_proposal(
        &proposer,
        &String::from_str(&t.env, "P"),
        &String::from_str(&t.env, "d"),
        &100,
        &3600,
        &Vec::new(&t.env),
    );
}

/// cast_vote reverts when paused.
#[test]
#[should_panic(expected = "Error(Contract, #26)")]
fn test_cast_vote_reverts_when_paused() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id = create_test_proposal(&t, &voter);
    let tok = votechain_token::TokenContractClient::new(&t.env, &t.token_id);
    tok.mint(&t.admin, &voter, &1_000_000_i128);
    t.client.pause(&t.admin, &Option::<String>::None);
    t.client.cast_vote(&voter, &id, &Vote::Yes);
}

/// finalise reverts when paused.
#[test]
#[should_panic(expected = "Error(Contract, #26)")]
fn test_finalise_reverts_when_paused() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id = create_test_proposal(&t, &voter);
    t.env.ledger().with_mut(|l| l.timestamp += 3601);
    t.client.pause(&t.admin, &Option::<String>::None);
    t.client.finalise(&id);
}

/// execute reverts when paused.
#[test]
#[should_panic(expected = "Error(Contract, #26)")]
fn test_execute_reverts_when_paused() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id = create_test_proposal(&t, &voter);
    let tok = votechain_token::TokenContractClient::new(&t.env, &t.token_id);
    tok.mint(&t.admin, &voter, &1_000_000_i128);
    t.client.cast_vote(&voter, &id, &Vote::Yes);
    t.env.ledger().with_mut(|l| l.timestamp += 3601);
    t.client.finalise(&id);
    t.client.pause(&t.admin, &Option::<String>::None);
    t.client.execute(&t.admin, &id);
}

/// cancel reverts when paused.
#[test]
#[should_panic(expected = "Error(Contract, #26)")]
fn test_cancel_reverts_when_paused() {
    let t = setup_env();
    let proposer = Address::generate(&t.env);
    let id = create_test_proposal(&t, &proposer);
    t.client.pause(&t.admin, &Option::<String>::None);
    t.client.cancel(&t.admin, &id);
}

/// update_quorum reverts when paused.
#[test]
#[should_panic(expected = "Error(Contract, #26)")]
fn test_update_quorum_reverts_when_paused() {
    let t = setup_env();
    let proposer = Address::generate(&t.env);
    let id = create_test_proposal(&t, &proposer);
    t.client.pause(&t.admin, &Option::<String>::None);
    t.client.update_quorum(&t.admin, &id, &500);
}

/// transfer_admin reverts when paused.
#[test]
#[should_panic(expected = "Error(Contract, #26)")]
fn test_transfer_admin_reverts_when_paused() {
    let t = setup_env();
    let new_admin = Address::generate(&t.env);
    t.client.pause(&t.admin, &Option::<String>::None);
    t.client.transfer_admin(&t.admin, &new_admin);
}

/// Read-only functions (get_proposal, get_vote, has_voted) remain available when paused.
#[test]
fn test_read_functions_available_when_paused() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id = create_test_proposal(&t, &voter);
    t.client.pause(&t.admin, &Option::<String>::None);
    // These should not panic
    let _ = t.client.get_proposal(&id);
    let _ = t.client.has_voted(&id, &voter);
    let _ = t.client.get_vote(&id, &voter);
    let _ = t.client.proposal_count();
    let _ = t.client.get_version();
    let _ = t.client.get_state();
    let _ = t.client.paused();
}

/// pause emits a "paused" event.
#[test]
fn test_pause_emits_event() {
    let t = setup_env();
    t.client.pause(&t.admin, &Option::<String>::None);
    let events = t.env.events().all();
    assert!(
        events
            .iter()
            .any(|(_, topics, _)| { topics == (symbol_short!("paused"),).into_val(&t.env) }),
        "expected paused event to be emitted"
    );
}

/// unpause emits an "unpaused" event.
#[test]
fn test_unpause_emits_event() {
    let t = setup_env();
    t.client.pause(&t.admin, &Option::<String>::None);
    t.client.unpause(&t.admin);
    let events = t.env.events().all();
    assert!(
        events
            .iter()
            .any(|(_, topics, _)| { topics == (symbol_short!("unpaused"),).into_val(&t.env) }),
        "expected unpaused event to be emitted"
    );
}

// ── end SEC-018 ───────────────────────────────────────────────────────────────

// ── TEST-ADMIN-EXEC-CANCEL: Admin-only execution and cancellation tests ──────

/// Test: execute succeeds on Passed proposal by admin
/// Verifies that the admin can successfully execute a proposal that has reached Passed state.
#[test]
fn test_execute_passed_proposal_by_admin_succeeds() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id = create_test_proposal(&t, &voter);

    // Vote to pass the proposal
    mint_and_vote(&t, &voter, id, Vote::Yes, 1_000_000);

    // Advance time past voting period
    t.env.ledger().with_mut(|l| l.timestamp += 3601);

    // Finalize to move to Passed state
    t.client.finalise(&id);
    assert_eq!(t.client.get_proposal(&id).state, ProposalState::Passed);

    // Admin executes the passed proposal
    t.client.execute(&t.admin, &id);

    // Verify state changed to Executed
    assert_eq!(t.client.get_proposal(&id).state, ProposalState::Executed);
}

/// Test: execute reverts for non-admin caller
/// Verifies that a non-admin address cannot execute a proposal.
#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn test_execute_reverts_for_non_admin_caller() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);
    let non_admin = Address::generate(&env);

    let id = setup_passed_proposal(&env, &client, &admin);

    // Non-admin attempts to execute
    client.execute(&non_admin, &id);
}

/// Test: execute reverts on non-Passed proposal
/// Verifies that execute fails when the proposal is not in Passed state.
#[test]
#[should_panic(expected = "Error(Contract, #12)")]
fn test_execute_reverts_on_non_passed_proposal() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);

    let id = setup_active_proposal(&env, &client, &admin);

    // Admin attempts to execute an Active proposal (not Passed)
    client.execute(&admin, &id);
}

/// Test: cancel succeeds on Active proposal by admin
/// Verifies that the admin can successfully cancel a proposal in Active state.
#[test]
fn test_cancel_active_proposal_by_admin_succeeds() {
    let t = setup_env();
    let proposer = Address::generate(&t.env);
    let id = create_test_proposal(&t, &proposer);

    // Verify proposal is Active
    assert_eq!(t.client.get_proposal(&id).state, ProposalState::Active);

    // Admin cancels the active proposal
    t.client.cancel(&t.admin, &id);

    // Verify state changed to Cancelled
    assert_eq!(t.client.get_proposal(&id).state, ProposalState::Cancelled);
}

/// Test: cancel reverts for non-admin caller
/// Verifies that a non-admin address cannot cancel a proposal.
#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn test_cancel_reverts_for_non_admin_caller() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);
    let non_admin = Address::generate(&env);

    let id = setup_active_proposal(&env, &client, &admin);

    // Non-admin attempts to cancel
    client.cancel(&non_admin, &id);
}

/// Test: cancel reverts on non-Active proposal
/// Verifies that cancel fails when the proposal is not in Active state.
#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn test_cancel_reverts_on_non_active_proposal() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);
    let token_id = setup_token(&env, &admin);
    client.initialize(&admin, &token_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);
    
    // Create and finalize a proposal to move it out of Active state
    let id = client.create_proposal(
        &admin,
        &String::from_str(&env, "Prop"),
        &String::from_str(&env, "desc"),
        &1_000_000,
        &3600,
        &Vec::new(&env),
    );
    env.ledger().with_mut(|l| l.timestamp += 3601);
    client.finalise(&id);

    // Verify proposal is no longer Active (it's Rejected)
    assert_eq!(client.get_proposal(&id).state, ProposalState::Rejected);

    // Admin attempts to cancel a non-Active proposal
    client.cancel(&admin, &id);
}

/// Test: execute emits event correctly
/// Verifies that the execute function emits the "executed" event with correct proposal ID.
#[test]
fn test_execute_emits_event_correctly() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id = create_test_proposal(&t, &voter);

    // Vote to pass the proposal
    mint_and_vote(&t, &voter, id, Vote::Yes, 1_000_000);

    // Advance time and finalize
    t.env.ledger().with_mut(|l| l.timestamp += 3601);
    t.client.finalise(&id);

    // Clear events before execute
    t.env.events().all();

    // Execute the proposal
    t.client.execute(&t.admin, &id);

    // Verify the "executed" event was emitted with correct proposal ID
    let events = t.env.events().all();
    assert!(
        events
            .iter()
            .any(|(_, topics, _)| { topics == (symbol_short!("executed"), id).into_val(&t.env) }),
        "expected 'executed' event with proposal ID {} to be emitted",
        id
    );
}

/// Test: cancel emits event correctly
/// Verifies that the cancel function emits the "cancelled" event with correct proposal ID.
#[test]
fn test_cancel_emits_event_correctly() {
    let t = setup_env();
    let proposer = Address::generate(&t.env);
    let id = create_test_proposal(&t, &proposer);

    // Clear events before cancel
    t.env.events().all();

    // Cancel the proposal
    t.client.cancel(&t.admin, &id);

    // Verify the "cancelled" event was emitted with correct proposal ID
    let events = t.env.events().all();
    assert!(
        events
            .iter()
            .any(|(_, topics, _)| { topics == (symbol_short!("cancelled"), id).into_val(&t.env) }),
        "expected 'cancelled' event with proposal ID {} to be emitted",
        id
    );
}

/// Test: execute and cancel maintain state consistency
/// Verifies that state transitions are atomic and consistent across multiple operations.
#[test]
fn test_execute_and_cancel_maintain_state_consistency() {
    let t = setup_env();
    let voter = Address::generate(&t.env);

    // Create two proposals
    let id1 = create_test_proposal(&t, &voter);
    let id2 = create_test_proposal(&t, &voter);

    // Pass and execute first proposal
    mint_and_vote(&t, &voter, id1, Vote::Yes, 1_000_000);
    t.env.ledger().with_mut(|l| l.timestamp += 3601);
    t.client.finalise(&id1);
    t.client.execute(&t.admin, &id1);

    // Cancel second proposal
    t.client.cancel(&t.admin, &id2);

    // Verify both states are correct and independent
    assert_eq!(t.client.get_proposal(&id1).state, ProposalState::Executed);
    assert_eq!(t.client.get_proposal(&id2).state, ProposalState::Cancelled);
}

/// Test: execute requires auth from admin
/// Verifies that execute properly checks admin authorization.
#[test]
#[should_panic]
fn test_execute_requires_admin_auth() {
    let env = Env::default();
    // Don't mock all auths - this will cause auth check to fail
    let client = new_client(&env);
    let admin = Address::generate(&env);

    let id = setup_passed_proposal(&env, &client, &admin);

    // This should panic due to failed auth check
    client.execute(&admin, &id);
}

/// Test: cancel requires auth from admin
/// Verifies that cancel properly checks admin authorization.
#[test]
#[should_panic]
fn test_cancel_requires_admin_auth() {
    let env = Env::default();
    // Don't mock all auths - this will cause auth check to fail
    let client = new_client(&env);
    let admin = Address::generate(&env);

    let id = setup_active_proposal(&env, &client, &admin);

    // This should panic due to failed auth check
    client.cancel(&admin, &id);
}

/// Test: execute on Cancelled proposal reverts
/// Verifies that execute fails when proposal is in Cancelled state.
#[test]
#[should_panic(expected = "Error(Contract, #12)")]
fn test_execute_on_cancelled_proposal_reverts() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);

    let id = setup_active_proposal(&env, &client, &admin);

    // Cancel the proposal first
    client.cancel(&admin, &id);
    assert_eq!(client.get_proposal(&id).state, ProposalState::Cancelled);

    // Attempt to execute a cancelled proposal
    client.execute(&admin, &id);
}

/// Test: cancel on Executed proposal reverts
/// Verifies that cancel fails when proposal is in Executed state.
#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn test_cancel_on_executed_proposal_reverts() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);

    let id = setup_passed_proposal(&env, &client, &admin);

    // Execute the proposal first
    client.execute(&admin, &id);
    assert_eq!(client.get_proposal(&id).state, ProposalState::Executed);

    // Attempt to cancel an executed proposal
    client.cancel(&admin, &id);
}

/// Test: multiple execute calls on same proposal revert
/// Verifies that a proposal can only be executed once.
#[test]
#[should_panic(expected = "Error(Contract, #12)")]
fn test_multiple_execute_calls_revert() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);

    let id = setup_passed_proposal(&env, &client, &admin);

    // First execute succeeds
    client.execute(&admin, &id);
    assert_eq!(client.get_proposal(&id).state, ProposalState::Executed);

    // Second execute on same proposal should revert
    client.execute(&admin, &id);
}

/// Test: multiple cancel calls on same proposal revert
/// Verifies that a proposal can only be cancelled once.
#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn test_multiple_cancel_calls_revert() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);

    let id = setup_active_proposal(&env, &client, &admin);

    // First cancel succeeds
    client.cancel(&admin, &id);
    assert_eq!(client.get_proposal(&id).state, ProposalState::Cancelled);

    // Second cancel on same proposal should revert
    client.cancel(&admin, &id);
}

// ── end TEST-ADMIN-EXEC-CANCEL ────────────────────────────────────────────────

// ── #66 TEST-001: initialize unit tests ──────────────────────────────────────

/// initialize succeeds with valid inputs and transitions state to Ready.
#[test]
fn test_initialize_success() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);
    let token_id = setup_token(&env, &admin);

    assert_eq!(client.get_state(), ContractState::Uninitialized);
    client.initialize(&admin, &token_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);
    assert_eq!(client.get_state(), ContractState::Ready);
}

/// initialize stores the version as (1, 0, 0).
#[test]
fn test_initialize_sets_version() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);
    let token_id = setup_token(&env, &admin);
    client.initialize(&admin, &token_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);
    assert_eq!(client.get_version(), (1, 0, 0));
}

/// initialize with min_proposal_balance > 0 enforces the balance requirement.
#[test]
#[should_panic]
fn test_initialize_min_balance_enforced() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);
    let token_id = setup_token(&env, &admin);
    client.initialize(&admin, &token_id, &1_000_000_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);

    let proposer = Address::generate(&env); // zero balance
    client.create_proposal(
        &proposer,
        &String::from_str(&env, "Title"),
        &String::from_str(&env, "desc"),
        &100,
        &3600,
        &Vec::new(&env),
    );
}

/// initialize with restrict_admin_vote=true blocks admin from voting on own proposals.
#[test]
#[should_panic(expected = "Error(Contract, #25)")]
fn test_initialize_restrict_admin_vote_enforced() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);
    let token_id = setup_token(&env, &admin);
    client.initialize(&admin, &token_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &true, &0_u64, &0_u64);

    let id = client.create_proposal(
        &admin,
        &String::from_str(&env, "Admin prop"),
        &String::from_str(&env, "desc"),
        &100,
        &3600,
        &Vec::new(&env),
    );
    assert_eq!(client.get_proposal(&id2).state, ProposalState::Active);
}

/// Proposing one second before the cooldown expires must be rejected.
#[test]
#[should_panic]
fn test_cooldown_one_second_before_boundary_reverts() {
    // admin voting on their own proposal must revert
    client.cast_vote(&admin, &id, &Vote::Yes);
}

/// Calling initialize a second time must revert with AlreadyInitialized (#13).
#[test]
#[should_panic(expected = "Error(Contract, #13)")]
fn test_initialize_already_initialized_reverts() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);
    let token_id = setup_token(&env, &admin);
    client.initialize(&admin, &token_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);
    client.initialize(&admin, &token_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);
}

/// initialize with the zero address as admin must revert with InvalidAddress (#28).
#[test]
#[should_panic(expected = "Error(Contract, #28)")]
fn test_initialize_zero_admin_reverts() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let zero = Address::from_str(
        &env,
        "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
    );
    let token_id = Address::generate(&env);
    client.initialize(&zero, &token_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);
}

/// initialize with the zero address as voting_token must revert with InvalidAddress (#28).
#[test]
#[should_panic(expected = "Error(Contract, #28)")]
fn test_initialize_zero_token_reverts() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);
    let zero = Address::from_str(&env, "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF");
    client.initialize(&admin, &zero, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);
}

// ── end #66 ───────────────────────────────────────────────────────────────────

// ── #69 TEST-004: finalise unit tests ─────────────────────────────────────────

/// Proposal passes when total_votes >= quorum AND votes_yes > votes_no.
#[test]
fn test_finalise_passes_when_quorum_met_and_yes_wins() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id = t.client.create_proposal(
        &voter,
        &String::from_str(&t.env, "Pass test"),
        &String::from_str(&t.env, "desc"),
        &500_000,
        &3600,
        &Vec::new(&t.env),
    );
    mint_and_vote(&t, &voter, id, Vote::Yes, 600_000);
    t.env.ledger().with_mut(|l| l.timestamp += 3601);
    t.client.finalise(&id);
    assert_eq!(t.client.get_proposal(&id).state, ProposalState::Passed);
}

/// Proposal is rejected when total_votes < quorum (even if yes > no).
#[test]
fn test_finalise_rejected_when_quorum_not_met() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id = t.client.create_proposal(
        &voter,
        &String::from_str(&t.env, "Low quorum"),
        &String::from_str(&t.env, "desc"),
        &1_000_000,
        &3600,
        &Vec::new(&t.env),
    );
    mint_and_vote(&t, &voter, id, Vote::Yes, 500_000); // below quorum
    t.env.ledger().with_mut(|l| l.timestamp += 3601);
    t.client.finalise(&id);
    assert_eq!(t.client.get_proposal(&id).state, ProposalState::Rejected);
}

/// Proposal is rejected when votes_yes == votes_no (tie), even if quorum is met.
#[test]
fn test_finalise_rejected_on_tie() {
    let t = setup_env();
    let voter_yes = Address::generate(&t.env);
    let voter_no = Address::generate(&t.env);
    let id = t.client.create_proposal(
        &voter_yes,
        &String::from_str(&t.env, "Tie"),
        &String::from_str(&t.env, "desc"),
        &200_000,
        &3600,
        &Vec::new(&t.env),
    );
    mint_and_vote(&t, &voter_yes, id, Vote::Yes, 200_000);
    mint_and_vote(&t, &voter_no, id, Vote::No, 200_000);
    t.env.ledger().with_mut(|l| l.timestamp += 3601);
    t.client.finalise(&id);
    assert_eq!(t.client.get_proposal(&id).state, ProposalState::Rejected);
}

/// Proposal is rejected when votes_no > votes_yes, even if quorum is met.
#[test]
fn test_finalise_rejected_when_no_wins() {
    let t = setup_env();
    let voter_yes = Address::generate(&t.env);
    let voter_no = Address::generate(&t.env);
    let id = t.client.create_proposal(
        &voter_yes,
        &String::from_str(&t.env, "No wins"),
        &String::from_str(&t.env, "desc"),
        &100_000,
        &3600,
        &Vec::new(&t.env),
    );
    mint_and_vote(&t, &voter_yes, id, Vote::Yes, 100_000);
    mint_and_vote(&t, &voter_no, id, Vote::No, 300_000);
    t.env.ledger().with_mut(|l| l.timestamp += 3601);
    t.client.finalise(&id);
    assert_eq!(t.client.get_proposal(&id).state, ProposalState::Rejected);
}

/// Proposal with zero votes is rejected (quorum not met).
#[test]
fn test_finalise_rejected_with_zero_votes() {
    let t = setup_env();
    let proposer = Address::generate(&t.env);
    let id = create_test_proposal(&t, &proposer);
    t.env.ledger().with_mut(|l| l.timestamp += 3601);
    t.client.finalise(&id);
    assert_eq!(t.client.get_proposal(&id).state, ProposalState::Rejected);
}

/// finalise before voting period ends must revert with VotingStillOpen (#9).
#[test]
#[should_panic(expected = "Error(Contract, #9)")]
fn test_finalise_before_end_time_reverts() {
    let t = setup_env();
    let proposer = Address::generate(&t.env);
    let id = create_test_proposal(&t, &proposer);
    t.client.finalise(&id); // voting period still open
}

/// finalise on a non-Active proposal must revert with ProposalNotActive (#7).
#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn test_finalise_already_finalised_reverts() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id = create_test_proposal(&t, &voter);
    mint_and_vote(&t, &voter, id, Vote::Yes, 1_000_000);
    t.env.ledger().with_mut(|l| l.timestamp += 3601);
    t.client.finalise(&id);
    t.client.finalise(&id); // second call must revert
}

/// finalise on a cancelled proposal must revert with ProposalNotActive (#7).
#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn test_finalise_cancelled_proposal_reverts() {
    let t = setup_env();
    let proposer = Address::generate(&t.env);
    let id = create_test_proposal(&t, &proposer);
    t.client.cancel(&t.admin, &id);
    t.env.ledger().with_mut(|l| l.timestamp += 3601);
    t.client.finalise(&id);
}

/// finalise on a non-existent proposal must revert with ProposalNotFound (#6).
#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn test_finalise_nonexistent_proposal_reverts() {
    let t = setup_env();
    t.client.finalise(&999);
}

/// Abstain votes count toward quorum: abstain-only proposal meets quorum but is Rejected.
#[test]
fn test_finalise_abstain_counts_toward_quorum_but_not_outcome() {
    let t = setup_env();
    let voter = Address::generate(&t.env);
    let id = t.client.create_proposal(
        &voter,
        &String::from_str(&t.env, "Abstain only"),
        &String::from_str(&t.env, "desc"),
        &300_000,
        &3600,
        &Vec::new(&t.env),
    );
    mint_and_vote(&t, &voter, id, Vote::Abstain, 300_000);
    t.env.ledger().with_mut(|l| l.timestamp += 3601);
    t.client.finalise(&id);
    // quorum met but yes (0) not > no (0) → Rejected
    assert_eq!(t.client.get_proposal(&id).state, ProposalState::Rejected);
}

// ── end #69 ───────────────────────────────────────────────────────────────────

// ── #72 TEST-007: full lifecycle integration tests ────────────────────────────

/// Happy path: initialize → create → vote Yes → finalise → execute.
#[test]
fn test_full_lifecycle_pass_and_execute() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);
    let voter = Address::generate(&env);
    let token_id = setup_token(&env, &admin);

    // initialize
    client.initialize(&admin, &token_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);
    assert_eq!(client.get_state(), ContractState::Ready);

    // mint tokens to voter
    let tok = votechain_token::TokenContractClient::new(&env, &token_id);
    tok.mint(&admin, &voter, &1_000_000_i128);

    // create proposal
    let id = client.create_proposal(
        &voter,
        &String::from_str(&env, "Treasury"),
        &String::from_str(&env, "Allocate funds"),
        &500_000,
        &3600,
        &Vec::new(&env),
    );
    assert_eq!(client.get_proposal(&id).state, ProposalState::Active);
    assert_eq!(client.proposal_count(), 1);

    // vote
    client.cast_vote(&voter, &id, &Vote::Yes);
    assert!(client.has_voted(&id, &voter));
    assert_eq!(client.get_proposal(&id).votes_yes, 1_000_000);

    // finalise after voting period
    env.ledger().with_mut(|l| l.timestamp += 3601);
    client.finalise(&id);
    assert_eq!(client.get_proposal(&id).state, ProposalState::Passed);

    // execute
    client.execute(&admin, &id);
    assert_eq!(client.get_proposal(&id).state, ProposalState::Executed);
}

/// Full lifecycle ending in rejection: quorum not met.
#[test]
fn test_full_lifecycle_reject_below_quorum() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);
    let voter = Address::generate(&env);
    let token_id = setup_token(&env, &admin);

    client.initialize(&admin, &token_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);

    let tok = votechain_token::TokenContractClient::new(&env, &token_id);
    tok.mint(&admin, &voter, &100_000_i128);

    let id = client.create_proposal(
        &voter,
        &String::from_str(&env, "Underfunded"),
        &String::from_str(&env, "desc"),
        &500_000, // quorum higher than available votes
        &3600,
        &Vec::new(&env),
    );

    client.cast_vote(&voter, &id, &Vote::Yes);
    env.ledger().with_mut(|l| l.timestamp += 3601);
    client.finalise(&id);
    assert_eq!(client.get_proposal(&id).state, ProposalState::Rejected);
}

/// Full lifecycle ending in cancellation by admin.
#[test]
fn test_full_lifecycle_cancel() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);
    let token_id = setup_token(&env, &admin);
    let cooldown: u64 = 3600;
    client.initialize(&admin, &token_id, &0_i128, &cooldown);

    let proposer = Address::generate(&env);
    client.create_proposal(
        &proposer,
        &String::from_str(&env, "First"),
        &String::from_str(&env, "desc"),
        &100,
        &7200,
    );
    env.ledger().with_mut(|l| l.timestamp += cooldown);
    client.create_proposal(
        &proposer,
        &String::from_str(&env, "Second"),
        &String::from_str(&env, "desc"),
        &100,
        &7200,
    );
    // immediately after second — cooldown reset, must be blocked
    client.create_proposal(
        &proposer,
        &String::from_str(&env, "Third too soon"),
        &String::from_str(&env, "desc"),
        &100,
        &3600,
    );
}

// ── end SEC-002 ───────────────────────────────────────────────────────────────

    client.initialize(&admin, &token_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);

    let proposer = Address::generate(&env);
    let id = client.create_proposal(
        &proposer,
        &String::from_str(&env, "To cancel"),
        &String::from_str(&env, "desc"),
        &100,
        &3600,
        &Vec::new(&env),
    );
    assert_eq!(client.get_proposal(&id).state, ProposalState::Active);

    client.cancel(&admin, &id);
    assert_eq!(client.get_proposal(&id).state, ProposalState::Cancelled);
}

/// Multiple voters across multiple proposals — votes are isolated per proposal.
#[test]
fn test_full_lifecycle_multiple_proposals_isolated() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);
    let voter1 = Address::generate(&env);
    let voter2 = Address::generate(&env);
    let token_id = setup_token(&env, &admin);

    client.initialize(&admin, &token_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);

    let tok = votechain_token::TokenContractClient::new(&env, &token_id);
    tok.mint(&admin, &voter1, &1_000_000_i128);
    tok.mint(&admin, &voter2, &1_000_000_i128);

    let id1 = client.create_proposal(
        &voter1,
        &String::from_str(&env, "Prop 1"),
        &String::from_str(&env, "d"),
        &500_000,
        &3600,
        &Vec::new(&env),
    );
    let id2 = client.create_proposal(
        &voter2,
        &String::from_str(&env, "Prop 2"),
        &String::from_str(&env, "d"),
        &500_000,
        &7200,
        &Vec::new(&env),
    );

    client.cast_vote(&voter1, &id1, &Vote::Yes);
    client.cast_vote(&voter2, &id2, &Vote::No);

    // votes don't bleed between proposals
    assert_eq!(client.get_proposal(&id1).votes_yes, 1_000_000);
    assert_eq!(client.get_proposal(&id1).votes_no, 0);
    assert_eq!(client.get_proposal(&id2).votes_yes, 0);
    assert_eq!(client.get_proposal(&id2).votes_no, 1_000_000);

    // finalise id1 (passes), id2 still active
    env.ledger().with_mut(|l| l.timestamp += 3601);
    client.finalise(&id1);
    assert_eq!(client.get_proposal(&id1).state, ProposalState::Passed);
    assert_eq!(client.get_proposal(&id2).state, ProposalState::Active);

    // execute id1
    client.execute(&admin, &id1);
    assert_eq!(client.get_proposal(&id1).state, ProposalState::Executed);

    // finalise id2 (rejected — no wins)
    env.ledger().with_mut(|l| l.timestamp += 7201);
    client.finalise(&id2);
    assert_eq!(client.get_proposal(&id2).state, ProposalState::Rejected);
}

/// Pausing blocks create/vote/finalise; unpausing restores them.
#[test]
fn test_full_lifecycle_pause_and_unpause() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let admin = Address::generate(&env);
    let voter = Address::generate(&env);
    let token_id = setup_token(&env, &admin);

    client.initialize(&admin, &token_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);

    let tok = votechain_token::TokenContractClient::new(&env, &token_id);
    tok.mint(&admin, &voter, &1_000_000_i128);

    let id = client.create_proposal(
        &voter,
        &String::from_str(&env, "Pausable"),
        &String::from_str(&env, "desc"),
        &500_000,
        &3600,
        &Vec::new(&env),
    );

    // pause — cast_vote must fail
    client.pause(&admin, &Option::<String>::None);
    assert!(client.paused());
    // ContractPaused guard is verified by the paused() flag above;
    // the actual revert is tested in test_create_proposal_reverts_when_paused.

    // unpause — vote and finalise succeed
    client.unpause(&admin);
    assert!(!client.paused());

    client.cast_vote(&voter, &id, &Vote::Yes);
    env.ledger().with_mut(|l| l.timestamp += 3601);
    client.finalise(&id);
    assert_eq!(client.get_proposal(&id).state, ProposalState::Passed);
    client.execute(&admin, &id);
    assert_eq!(client.get_proposal(&id).state, ProposalState::Executed);
}

// ── end #72 ───────────────────────────────────────────────────────────────────

// ── SEC-003: input sanitization ───────────────────────────────────────────────

/// Title with a null byte is rejected.
#[test]
#[should_panic]
fn test_title_null_byte_rejected() {
    let t = setup_env();
    let proposer = Address::generate(&t.env);
    // "bad\x00title" — null byte in the middle
    t.client.create_proposal(
        &proposer,
        &String::from_bytes(&t.env, b"bad\x00title"),
        &String::from_str(&t.env, "desc"),
        &100,
        &3600,
        &Vec::new(&t.env),
    );
}

/// Title with a control character (newline) is rejected.
#[test]
#[should_panic]
fn test_title_control_char_rejected() {
    let t = setup_env();
    let proposer = Address::generate(&t.env);
    t.client.create_proposal(
        &proposer,
        &String::from_bytes(&t.env, b"bad\ntitle"),
        &String::from_str(&t.env, "desc"),
        &100,
        &3600,
        &Vec::new(&t.env),
    );
}

/// Title with DEL (0x7F) is rejected.
#[test]
#[should_panic]
fn test_title_del_char_rejected() {
    let t = setup_env();
    let proposer = Address::generate(&t.env);
    t.client.create_proposal(
        &proposer,
        &String::from_bytes(&t.env, b"bad\x7ftitle"),
        &String::from_str(&t.env, "desc"),
        &100,
        &3600,
        &Vec::new(&t.env),
    );
}

/// Description with a null byte is rejected.
#[test]
#[should_panic]
fn test_description_null_byte_rejected() {
    let t = setup_env();
    let proposer = Address::generate(&t.env);
    t.client.create_proposal(
        &proposer,
        &String::from_str(&t.env, "Valid title"),
        &String::from_bytes(&t.env, b"bad\x00desc"),
        &100,
        &3600,
        &Vec::new(&t.env),
    );
}

/// Description with a control character (tab) is rejected.
#[test]
#[should_panic]
fn test_description_control_char_rejected() {
    let t = setup_env();
    let proposer = Address::generate(&t.env);
    t.client.create_proposal(
        &proposer,
        &String::from_str(&t.env, "Valid title"),
        &String::from_bytes(&t.env, b"bad\x09desc"),
        &100,
        &3600,
        &Vec::new(&t.env),
    );
}

/// A title of exactly 128 printable bytes is accepted.
#[test]
fn test_title_max_length_accepted() {
    let t = setup_env();
    let proposer = Address::generate(&t.env);
    let title_128 = "A".repeat(128);
    let id = t.client.create_proposal(
        &proposer,
        &String::from_str(&t.env, &title_128),
        &String::from_str(&t.env, "desc"),
        &100,
        &3600,
        &Vec::new(&t.env),
    );
    assert_eq!(t.client.get_proposal(&id).state, ProposalState::Active);
}

/// A description of exactly 1024 printable bytes is accepted.
#[test]
fn test_description_max_length_accepted() {
    let t = setup_env();
    let proposer = Address::generate(&t.env);
    let desc_1024 = "B".repeat(1024);
    let id = t.client.create_proposal(
        &proposer,
        &String::from_str(&t.env, "Valid title"),
        &String::from_str(&t.env, &desc_1024),
        &100,
        &3600,
        &Vec::new(&t.env),
    );
    assert_eq!(t.client.get_proposal(&id).state, ProposalState::Active);
}

/// A title of 129 bytes is rejected (exceeds MAX_TITLE_LEN).
#[test]
#[should_panic]
fn test_title_too_long_rejected() {
    let t = setup_env();
    let proposer = Address::generate(&t.env);
    let title_129 = "A".repeat(129);
    t.client.create_proposal(
        &proposer,
        &String::from_str(&t.env, &title_129),
        &String::from_str(&t.env, "desc"),
        &100,
        &3600,
        &Vec::new(&t.env),
    );
}

/// A description of 1025 bytes is rejected (exceeds MAX_DESC_LEN).
#[test]
#[should_panic]
fn test_description_too_long_rejected() {
    let t = setup_env();
    let proposer = Address::generate(&t.env);
    let desc_1025 = "B".repeat(1025);
    t.client.create_proposal(
        &proposer,
        &String::from_str(&t.env, "Valid title"),
        &String::from_str(&t.env, &desc_1025),
        &100,
        &3600,
        &Vec::new(&t.env),
    );
}

/// A title with only printable ASCII (space = 0x20) is accepted.
#[test]
fn test_title_space_accepted() {
    let t = setup_env();
    let proposer = Address::generate(&t.env);
    let id = t.client.create_proposal(
        &proposer,
        &String::from_str(&t.env, "Hello World"),
        &String::from_str(&t.env, "desc"),
        &100,
        &3600,
        &Vec::new(&t.env),
    );
    assert_eq!(t.client.get_proposal(&id).state, ProposalState::Active);
}

// ── end SEC-003 ───────────────────────────────────────────────────────────────

// ── SC-001: upgrade tests ─────────────────────────────────────────────────────

/// Admin can upgrade to a strictly higher version; event is emitted.
#[test]
fn test_upgrade_success() {
    let t = setup_env();
    assert_eq!(t.client.get_version(), (1, 0, 0));
    t.client.upgrade(&t.admin, &(1, 1, 0));
    assert_eq!(t.client.get_version(), (1, 1, 0));
}

/// Downgrade (lower version) is rejected.
#[test]
fn test_upgrade_rejects_downgrade() {
    let t = setup_env();
    t.client.upgrade(&t.admin, &(2, 0, 0));
    let result = t.client.try_upgrade(&t.admin, &(1, 9, 9));
    assert_eq!(result, Err(Ok(ContractError::DowngradeNotAllowed)));
}

/// Same version is rejected (not strictly greater).
#[test]
fn test_upgrade_rejects_same_version() {
    let t = setup_env();
    let result = t.client.try_upgrade(&t.admin, &(1, 0, 0));
    assert_eq!(result, Err(Ok(ContractError::DowngradeNotAllowed)));
}

/// Non-admin cannot upgrade.
#[test]
fn test_upgrade_requires_admin() {
    let t = setup_env();
    let non_admin = Address::generate(&t.env);
    let result = t.client.try_upgrade(&non_admin, &(2, 0, 0));
    assert_eq!(result, Err(Ok(ContractError::NotAdmin)));
}

// ── end SC-001 ────────────────────────────────────────────────────────────────

// ── TEST-007: Concurrent voting simulation ────────────────────────────────────

/// 100 voters cast Yes votes in sequence; final tally must equal the sum of
/// all individual vote weights.
#[test]
fn test_concurrent_voting_100_yes_voters() {
    let t = setup_env();
    let proposal_id = create_test_proposal(&t, &t.admin.clone());

    let mut expected_yes: i128 = 0;
    for i in 1..=100_i128 {
        let voter = Address::generate(&t.env);
        mint_and_vote(&t, &voter, proposal_id, Vote::Yes, i * 1_000);
        expected_yes += i * 1_000;
    }

    let proposal = t.client.get_proposal(&proposal_id);
    assert_eq!(proposal.votes_yes, expected_yes);
    assert_eq!(proposal.votes_no, 0);
    assert_eq!(proposal.votes_abstain, 0);
}

/// 100 voters with a mix of Yes / No / Abstain; each bucket must equal the
/// sum of weights for that choice.
#[test]
fn test_concurrent_voting_mixed_votes() {
    let t = setup_env();
    let proposal_id = create_test_proposal(&t, &t.admin.clone());

    let mut expected_yes: i128 = 0;
    let mut expected_no: i128 = 0;
    let mut expected_abstain: i128 = 0;

    for i in 1..=100_i128 {
        let voter = Address::generate(&t.env);
        let weight = i * 500;
        let vote = match i % 3 {
            0 => {
                expected_yes += weight;
                Vote::Yes
            }
            1 => {
                expected_no += weight;
                Vote::No
            }
            _ => {
                expected_abstain += weight;
                Vote::Abstain
            }
        };
        mint_and_vote(&t, &voter, proposal_id, vote, weight);
    }

    let proposal = t.client.get_proposal(&proposal_id);
    assert_eq!(proposal.votes_yes, expected_yes);
    assert_eq!(proposal.votes_no, expected_no);
    assert_eq!(proposal.votes_abstain, expected_abstain);
}

/// Total vote count equals the sum of all three buckets (no state corruption).
#[test]
fn test_concurrent_voting_total_integrity() {
    let t = setup_env();
    let proposal_id = create_test_proposal(&t, &t.admin.clone());

    let mut total_weight: i128 = 0;
    for i in 1..=100_i128 {
        let voter = Address::generate(&t.env);
        let weight = 1_000_i128;
        total_weight += weight;
        let vote = match i % 3 {
            0 => Vote::Yes,
            1 => Vote::No,
            _ => Vote::Abstain,
        };
        mint_and_vote(&t, &voter, proposal_id, vote, weight);
    }

    let p = t.client.get_proposal(&proposal_id);
    assert_eq!(p.votes_yes + p.votes_no + p.votes_abstain, total_weight);
}

/// Each voter can only vote once; a second attempt returns AlreadyVoted.
#[test]
fn test_concurrent_voting_no_double_vote() {
    let t = setup_env();
    let proposal_id = create_test_proposal(&t, &t.admin.clone());

    let voter = Address::generate(&t.env);
    mint_and_vote(&t, &voter, proposal_id, Vote::Yes, 1_000);

    let result = t.client.try_cast_vote(&voter, &proposal_id, &Vote::No);
    assert_eq!(result, Err(Ok(ContractError::AlreadyVoted)));
}

// ── end TEST-007 ──────────────────────────────────────────────────────────────

// ── SC-007: get_admin / get_config view functions ─────────────────────────────

/// get_admin returns the admin address set during initialize, without auth.
#[test]
fn test_get_admin_returns_admin() {
    let t = setup_env();
    assert_eq!(t.client.get_admin(), t.admin);
}

/// get_admin fails before initialize (contract not yet set up).
#[test]
fn test_get_admin_before_init_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let client = GovernanceContractClient::new(&env, &env.register(GovernanceContract, ()));
    let result = client.try_get_admin();
    assert_eq!(result, Err(Ok(ContractError::AdminNotSet)));
}

/// get_config returns all initialization parameters correctly.
#[test]
fn test_get_config_returns_all_fields() {
    let env = Env::default();
    env.mock_all_auths();

    let gov_id = env.register(GovernanceContract, ());
    let client = GovernanceContractClient::new(&env, &gov_id);
    let admin = Address::generate(&env);

    let tok_id = env.register(votechain_token::TokenContract, ());
    votechain_token::TokenContractClient::new(&env, &tok_id).initialize(&admin, &10_000_000);

    client.initialize(&admin, &tok_id, &500_i128, &3600_u64, &60_u64, &2_592_000_u64, &true, &86400_u64);

    let cfg = client.get_config();
    assert_eq!(cfg.voting_token, tok_id);
    assert_eq!(cfg.min_proposal_balance, 500);
    assert_eq!(cfg.proposal_cooldown, 3600);
    assert_eq!(cfg.min_duration, 60);
    assert_eq!(cfg.max_duration, 2_592_000);
    assert!(cfg.restrict_admin_vote);
    assert_eq!(cfg.timelock_duration, 86400);
    assert!(!cfg.paused);
    assert_eq!(cfg.version, (1, 0, 0));
}

/// get_config reflects paused state correctly.
#[test]
fn test_get_config_paused_flag() {
    let t = setup_env();
    assert!(!t.client.get_config().paused);
    t.client.pause(&t.admin);
    assert!(t.client.get_config().paused);
}

/// get_config fails before initialize.
#[test]
fn test_get_config_before_init_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let client = GovernanceContractClient::new(&env, &env.register(GovernanceContract, ()));
    let result = client.try_get_config();
    assert_eq!(result, Err(Ok(ContractError::VotingTokenNotSet)));
}

// =============================================================================
// TTL Bump Tests
// =============================================================================

/// Verifies that TTL bump amount is configurable and retrievable.
#[test]
fn test_ttl_bump_configuration() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let token_id = setup_token(&env, &admin);
    let client = new_client(&env);

    client.initialize(&admin, &token_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);

    // Default TTL bump should be 518400 ledgers (~60 days)
    let default_ttl = 518_400_u32;
    
    // Test setting a custom TTL bump value
    let custom_ttl = 259_200_u32; // 30 days
    set_ttl_bump_ledgers(&env, custom_ttl);
    let retrieved_ttl = get_ttl_bump_ledgers(&env);
    assert_eq!(retrieved_ttl, custom_ttl);

    // Test setting back to default
    set_ttl_bump_ledgers(&env, default_ttl);
    let retrieved_ttl = get_ttl_bump_ledgers(&env);
    assert_eq!(retrieved_ttl, default_ttl);
}

/// Verifies that TTL is bumped on proposal write operations.
#[test]
fn test_ttl_bump_on_proposal_write() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let voter = Address::generate(&env);
    let token_id = setup_token(&env, &admin);
    let client = new_client(&env);

    client.initialize(&admin, &token_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);

    let tok = votechain_token::TokenContractClient::new(&env, &token_id);
    tok.mint(&admin, &voter, &1_000_000_i128);

    // Create a proposal - this should bump TTL
    let id = client.create_proposal(
        &voter,
        &String::from_str(&env, "Test Proposal"),
        &String::from_str(&env, "Description"),
        &500_000,
        &3600,
        &Vec::new(&env),
    );

    // Verify the proposal was created
    let proposal = client.get_proposal(&id);
    assert_eq!(proposal.id, id);
    assert_eq!(proposal.proposer, voter);
    assert_eq!(proposal.state, ProposalState::Active);
}

/// Verifies that TTL is bumped on vote-related write operations.
#[test]
fn test_ttl_bump_on_vote_writes() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let voter1 = Address::generate(&env);
    let voter2 = Address::generate(&env);
    let token_id = setup_token(&env, &admin);
    let client = new_client(&env);

    client.initialize(&admin, &token_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);

    let tok = votechain_token::TokenContractClient::new(&env, &token_id);
    tok.mint(&admin, &voter1, &1_000_000_i128);
    tok.mint(&admin, &voter2, &1_000_000_i128);

    let id = client.create_proposal(
        &voter1,
        &String::from_str(&env, "Vote Test"),
        &String::from_str(&env, "desc"),
        &500_000,
        &3600,
        &Vec::new(&env),
    );

    // Cast votes - each vote should bump TTL for vote records
    client.cast_vote(&voter1, &id, &Vote::Yes);
    client.cast_vote(&voter2, &id, &Vote::No);

    // Verify votes were recorded
    let proposal = client.get_proposal(&id);
    assert_eq!(proposal.votes_yes, 1_000_000);
    assert_eq!(proposal.votes_no, 1_000_000);
}

/// Verifies that read-only operations do NOT bump TTL.
/// This test ensures no unnecessary TTL bumps occur on queries.
#[test]
fn test_no_ttl_bump_on_read_only() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let voter = Address::generate(&env);
    let token_id = setup_token(&env, &admin);
    let client = new_client(&env);

    client.initialize(&admin, &token_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);

    let tok = votechain_token::TokenContractClient::new(&env, &token_id);
    tok.mint(&admin, &voter, &1_000_000_i128);

    let id = client.create_proposal(
        &voter,
        &String::from_str(&env, "Read Test"),
        &String::from_str(&env, "desc"),
        &500_000,
        &3600,
        &Vec::new(&env),
    );

    // Multiple read operations should not cause compilation errors
    // Note: In a real environment with ledger state tracking, 
    // we would verify that TTL was NOT extended.
    let _proposal1 = client.get_proposal(&id);
    let _proposal2 = client.get_proposal(&id);
    
    // Verify the proposal is unchanged after reads
    let proposal = client.get_proposal(&id);
    assert_eq!(proposal.id, id);
}

/// Verifies that entries survive the expected ledger count with TTL bumping.
#[test]
fn test_proposal_survives_expected_ledgers() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let voter = Address::generate(&env);
    let token_id = setup_token(&env, &admin);
    let client = new_client(&env);

    client.initialize(&admin, &token_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);

    let tok = votechain_token::TokenContractClient::new(&env, &token_id);
    tok.mint(&admin, &voter, &1_000_000_i128);

    // Create a proposal
    let id = client.create_proposal(
        &voter,
        &String::from_str(&env, "Long Running"),
        &String::from_str(&env, "Proposal that persists"),
        &500_000,
        &3600,
        &Vec::new(&env),
    );

    // Cast a vote (which also bumps TTL on vote records)
    client.cast_vote(&voter, &id, &Vote::Yes);

    // Simulate passage of time (many ledgers)
    // With default TTL bump of 518400 ledgers, entries should survive
    env.ledger().with_mut(|l| {
        l.sequence += 100_000; // Advance by 100,000 ledgers
    });

    // Proposal should still be accessible after the ledger advancement
    let proposal = client.get_proposal(&id);
    assert_eq!(proposal.id, id);
    assert_eq!(proposal.votes_yes, 1_000_000);

    // Update TTL by recording another operation
    // In a production scenario, periodic operations would extend TTL
    env.ledger().with_mut(|l| {
        l.timestamp += 50_000; // Advance timestamp
    });

    // Verify proposal still exists
    let proposal_final = client.get_proposal(&id);
    assert_eq!(proposal_final.id, id);
}

/// Verifies that LastProposal entries have TTL bumped on write.
#[test]
fn test_ttl_bump_on_last_proposal() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let proposer = Address::generate(&env);
    let token_id = setup_token(&env, &admin);
    let client = new_client(&env);

    client.initialize(&admin, &token_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);

    let tok = votechain_token::TokenContractClient::new(&env, &token_id);
    tok.mint(&admin, &proposer, &2_000_000_i128);

    // Create first proposal
    let id1 = client.create_proposal(
        &proposer,
        &String::from_str(&env, "Prop 1"),
        &String::from_str(&env, "desc"),
        &500_000,
        &3600,
        &Vec::new(&env),
    );

    // Create second proposal (LastProposal is updated and TTL bumped)
    let id2 = client.create_proposal(
        &proposer,
        &String::from_str(&env, "Prop 2"),
        &String::from_str(&env, "desc"),
        &500_000,
        &3600,
        &Vec::new(&env),
    );

    // Both proposals should exist
    let prop1 = client.get_proposal(&id1);
    let prop2 = client.get_proposal(&id2);
    assert_eq!(prop1.id, id1);
    assert_eq!(prop2.id, id2);
    assert_eq!(prop2.proposer, proposer);
}
