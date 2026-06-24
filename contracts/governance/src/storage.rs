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

//! Storage accessors for the governance contract.
//!
//! # Namespacing strategy
//!
//! All storage entries are keyed by variants of [`DataKey`].  Soroban
//! serialises the enum variant discriminant into the XDR key before any
//! payload, so every variant occupies a completely isolated key space.
//! Adding a new data type requires only a new enum variant — there is no
//! risk of collision with existing keys.
//!
//! Storage tiers in use:
//! - **Instance** – singleton config values (`Admin`, `VotingToken`,
//!   `ProposalCount`, `MinProposalBalance`, `ProposalCooldown`, `MinDuration`, `MaxDuration`, `Version`).
//!   Shares the contract instance TTL; cheap to access.
//! - **Persistent** – proposal data and per-voter records (`Proposal`,
//!   `HasVoted`, `VoteRecord`, `VoterSnapshot`, `LastProposal`).
//!   Survives ledger expiry; must be bumped explicitly for long-lived entries.
//!
//! ## Key-space isolation between related variants
//!
//! Several variants share the same payload shape `(u64, Address)` but are
//! distinct enum variants (`HasVoted`, `VoteRecord`, `VoterSnapshot`).
//! Because the discriminant is part of the serialised key, these can never
//! collide even when called with identical arguments.

use soroban_sdk::{Env, Address, String};
use crate::types::{ContractError, ContractState, DataKey, Proposal, VoteRecord};
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
//   DataKey::Admin              – admin address (set at init, read on admin ops)
//   DataKey::VotingToken        – governance token address (read on every vote)
//   DataKey::ProposalCount      – monotonic proposal ID counter (SEC-007)
//   DataKey::MinProposalBalance – minimum token balance to create a proposal
//   DataKey::ProposalCooldown   – seconds between proposals per address
//   DataKey::ContractState      – lifecycle state (Uninitialized / Ready)
//   DataKey::RestrictAdminVote  – whether admin may vote on own proposals
//   DataKey::Paused             – contract pause flag
//   DataKey::Version            – semver tuple (major, minor, patch)
//   DataKey::TTLBumpLedgers     – configurable ledger count for TTL bumping
//
// PERSISTENT storage – per-key TTL, survives ledger expiry independently.
//                      Used for data that must outlive any single ledger and
//                      is keyed by a variable (proposal ID, voter address, etc.).
//                      TTL is bumped on write to prevent expiry of long-running proposals.
//
//   DataKey::Proposal(id)                  – full proposal struct
//   DataKey::HasVoted(proposal_id, voter)  – deduplication flag per voter
//   DataKey::VoteRecord(proposal_id, voter)– immutable vote audit record
//   DataKey::VoterSnapshot(proposal_id, voter) – balance snapshot at vote time
//   DataKey::LastProposal(proposer)        – timestamp of proposer's last proposal
//   DataKey::MultiSigAction(id)            – multi-sig action awaiting approval
//   DataKey::MultiSigApproval(action_id, approver) – approval flag per approver
//
// TEMPORARY storage  – not used in this contract. Allowances in the token
//                      contract use temporary storage; see token/src/storage.rs.
// =============================================================================

// =============================================================================
// TTL Bump Configuration & Helpers
// =============================================================================

/// Default TTL bump amount in ledgers (~60 days at ~10 sec per ledger).
/// Configurable via `set_ttl_bump_ledgers()`.
const DEFAULT_TTL_BUMP_LEDGERS: u32 = 518_400;

/// Retrieves the configured TTL bump amount in ledgers.
/// Used when extending TTL for persistent storage entries.
pub fn get_ttl_bump_ledgers(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::TTLBumpLedgers)
        .unwrap_or(DEFAULT_TTL_BUMP_LEDGERS)
}

/// Sets the TTL bump amount in ledgers for persistent storage entries.
pub fn set_ttl_bump_ledgers(env: &Env, ledgers: u32) {
    env.storage().instance().set(&DataKey::TTLBumpLedgers, &ledgers);
}

/// Bumps the TTL for a persistent storage entry.
/// 
/// Uses a conservative strategy:
/// - Threshold: half of the configured bump amount
/// - Bump amount: configured TTL bump amount
/// 
/// This ensures entries are extended before they expire.
fn get_ttl_bump_params(env: &Env) -> (u32, u32) {
    let bump_amount = get_ttl_bump_ledgers(env);
    let threshold = bump_amount / 2;
    (threshold, bump_amount)
}

/// Bumps the TTL for a proposal and all its related entries.
/// 
/// Extends TTL for the Proposal key.
/// Individual vote entries are bumped when votes are cast.
pub fn bump_ttl_proposal(env: &Env, proposal_id: u64) {
    let (threshold, bump_amount) = get_ttl_bump_params(env);
    env.storage()
        .persistent()
        .extend_ttl(&DataKey::Proposal(proposal_id), threshold, bump_amount);
}

/// Bumps the TTL for a voter's last proposal timestamp.
pub fn bump_ttl_last_proposal(env: &Env, proposer: &Address) {
    let (threshold, bump_amount) = get_ttl_bump_params(env);
    env.storage()
        .persistent()
        .extend_ttl(&DataKey::LastProposal(proposer.clone()), threshold, bump_amount);
}

/// Bumps the TTL for a voter's has_voted flag on a proposal.
pub fn bump_ttl_has_voted(env: &Env, proposal_id: u64, voter: &Address) {
    let (threshold, bump_amount) = get_ttl_bump_params(env);
    env.storage()
        .persistent()
        .extend_ttl(&DataKey::HasVoted(proposal_id, voter.clone()), threshold, bump_amount);
}

/// Bumps the TTL for a voter's vote record on a proposal.
pub fn bump_ttl_vote_record(env: &Env, proposal_id: u64, voter: &Address) {
    let (threshold, bump_amount) = get_ttl_bump_params(env);
    env.storage()
        .persistent()
        .extend_ttl(&DataKey::VoteRecord(proposal_id, voter.clone()), threshold, bump_amount);
}

/// Bumps the TTL for a voter's snapshot on a proposal.
pub fn bump_ttl_voter_snapshot(env: &Env, proposal_id: u64, voter: &Address) {
    let (threshold, bump_amount) = get_ttl_bump_params(env);
    env.storage()
        .persistent()
        .extend_ttl(&DataKey::VoterSnapshot(proposal_id, voter.clone()), threshold, bump_amount);
}

/// Bumps the TTL for a multi-sig action.
pub fn bump_ttl_multisig_action(env: &Env, action_id: u64) {
    let (threshold, bump_amount) = get_ttl_bump_params(env);
    env.storage()
        .persistent()
        .extend_ttl(&DataKey::MultiSigAction(action_id), threshold, bump_amount);
}

/// Bumps the TTL for a multi-sig approval.
pub fn bump_ttl_multisig_approval(env: &Env, action_id: u64, approver: &Address) {
    let (threshold, bump_amount) = get_ttl_bump_params(env);
    env.storage()
        .persistent()
        .extend_ttl(&DataKey::MultiSigApproval(action_id, approver.clone()), threshold, bump_amount);
}

/// Persists a proposal to contract storage, keyed by its ID.
/// TTL is automatically bumped to prevent expiry on long-running proposals.
pub fn save_proposal(env: &Env, p: &Proposal) {
    env.storage().persistent().set(&DataKey::Proposal(p.id), p);
    bump_proposal_ttl(env, p.id);
}

/// Loads a proposal from storage by ID.
///
/// # Errors
/// - [`ContractError::ProposalNotFound`] if no proposal exists for `id`.
pub fn load_proposal(env: &Env, id: u64) -> Result<Proposal, ContractError> {
    env.storage()
        .persistent()
        .get(&DataKey::Proposal(id))
        .ok_or(ContractError::ProposalNotFound)
}

/// Increments the proposal counter and returns the new ID.
///
/// SEC-007: The counter is the sole source of proposal IDs; no caller-supplied
/// ID is accepted.  The read-increment-write executes atomically within a
/// single Soroban transaction, so concurrent creation attempts (different
/// transactions in the same ledger) each observe a unique counter value.
///
/// # Errors
/// - [`ContractError::ProposalCountOverflow`] if the counter would exceed `u64::MAX`.
pub fn next_id(env: &Env) -> Result<u64, ContractError> {
    let current: u64 = env
        .storage()
        .instance()
        .get(&DataKey::ProposalCount)
        .unwrap_or(0);
    let n = current
        .checked_add(1)
        .ok_or(ContractError::ProposalCountOverflow)?;
    env.storage().instance().set(&DataKey::ProposalCount, &n);
    Ok(n)
}

/// Stores the admin address in instance storage.
pub fn set_admin(env: &Env, admin: &Address) {
    env.storage().instance().set(&DataKey::Admin, admin);
}

/// Returns `true` if the contract has been initialised (admin key exists).
pub fn is_initialized(env: &Env) -> bool {
    env.storage().instance().has(&DataKey::Admin)
}

/// Stores the contract lifecycle state in instance storage.
pub fn set_contract_state(env: &Env, state: &ContractState) {
    env.storage().instance().set(&DataKey::ContractState, state);
}

/// Returns the contract lifecycle state.
///
/// Defaults to [`ContractState::Uninitialized`] if the key has never been written
/// (i.e. before the very first `initialize` call).
pub fn get_contract_state(env: &Env) -> ContractState {
    env.storage()
        .instance()
        .get(&DataKey::ContractState)
        .unwrap_or(ContractState::Uninitialized)
}

/// Returns the stored admin address.
///
/// # Errors
/// - [`ContractError::AdminNotSet`] if the contract has not been initialised.
pub fn get_admin(env: &Env) -> Result<Address, ContractError> {
    env.storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(ContractError::AdminNotSet)
}

/// Stores the governance token address in instance storage.
pub fn set_voting_token(env: &Env, token: &Address) {
    env.storage().instance().set(&DataKey::VotingToken, token);
}

/// Returns the stored governance token address.
///
/// # Errors
/// - [`ContractError::VotingTokenNotSet`] if the contract has not been initialised.
pub fn get_voting_token(env: &Env) -> Result<Address, ContractError> {
    env.storage()
        .instance()
        .get(&DataKey::VotingToken)
        .ok_or(ContractError::VotingTokenNotSet)
}

pub fn set_veto_threshold(env: &Env, v: i128) {
    env.storage().instance().set(&DataKey::VetoThreshold, &v);
}

pub fn get_veto_threshold(env: &Env) -> i128 {
    env.storage().instance().get(&DataKey::VetoThreshold).unwrap_or(0)
}

/// Records that `voter` has voted on `proposal_id`.
/// TTL is automatically bumped to prevent expiry on long-running proposals.
pub fn mark_voted(env: &Env, proposal_id: u64, voter: &Address) {
    env.storage().persistent().set(&DataKey::HasVoted(proposal_id, voter.clone()), &true);
    bump_has_voted_ttl(env, proposal_id, voter);
}

/// Returns `true` if `voter` has already voted on `proposal_id`.
pub fn has_voted(env: &Env, proposal_id: u64, voter: &Address) -> bool {
    env.storage()
        .persistent()
        .get(&DataKey::HasVoted(proposal_id, voter.clone()))
        .unwrap_or(false)
}

/// Stores the vote record for `voter` on `proposal_id`.
/// TTL is automatically bumped to prevent expiry on long-running proposals.
pub fn save_vote_record(env: &Env, proposal_id: u64, voter: &Address, record: &VoteRecord) {
    env.storage().persistent().set(&DataKey::VoteRecord(proposal_id, voter.clone()), record);
    bump_vote_record_ttl(env, proposal_id, voter);
}

/// Returns the vote record for `voter` on `proposal_id`, or `None` if not voted.
pub fn get_vote_record(env: &Env, proposal_id: u64, voter: &Address) -> Option<VoteRecord> {
    env.storage()
        .persistent()
        .get(&DataKey::VoteRecord(proposal_id, voter.clone()))
}

pub fn set_min_proposal_balance(env: &Env, v: i128) {
    env.storage()
        .instance()
        .set(&DataKey::MinProposalBalance, &v);
}

pub fn get_min_proposal_balance(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::MinProposalBalance)
        .unwrap_or(0)
}

pub fn set_proposal_cooldown(env: &Env, v: u64) {
    env.storage().instance().set(&DataKey::ProposalCooldown, &v);
}

pub fn get_proposal_cooldown(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::ProposalCooldown)
        .unwrap_or(0)
}

pub fn set_last_proposal(env: &Env, proposer: &Address, ts: u64) {
    env.storage().persistent().set(&DataKey::LastProposal(proposer.clone()), &ts);
    bump_last_proposal_ttl(env, proposer);
}

pub fn get_last_proposal(env: &Env, proposer: &Address) -> u64 {
    env.storage()
        .persistent()
        .get(&DataKey::LastProposal(proposer.clone()))
        .unwrap_or(0)
}

/// Records the voter's token balance snapshot for a given proposal.
/// Called once per voter per proposal at the time of casting their vote.
/// TTL is automatically bumped to prevent expiry on long-running proposals.
pub fn save_voter_snapshot(env: &Env, proposal_id: u64, voter: &Address, weight: i128) {
    env.storage()
        .persistent()
        .set(&DataKey::VoterSnapshot(proposal_id, voter.clone()), &weight);
    bump_voter_snapshot_ttl(env, proposal_id, voter);
}

/// Returns the stored vote-weight snapshot for a voter on a proposal.
/// Returns `None` if no snapshot has been recorded yet.
pub fn get_voter_snapshot(env: &Env, proposal_id: u64, voter: &Address) -> Option<i128> {
    env.storage()
        .persistent()
        .get(&DataKey::VoterSnapshot(proposal_id, voter.clone()))
}
/// Stores the contract version as a `(major, minor, patch)` tuple.
pub fn set_version(env: &Env, version: (u32, u32, u32)) {
    env.storage().instance().set(&DataKey::Version, &version);
}

/// Returns the stored contract version as a `(major, minor, patch)` tuple.
pub fn get_version(env: &Env) -> (u32, u32, u32) {
    env.storage()
        .instance()
        .get(&DataKey::Version)
        .unwrap_or((0, 0, 0))
}

/// Stores whether admin is restricted from voting on their own proposals.
pub fn set_restrict_admin_vote(env: &Env, v: bool) {
    env.storage()
        .instance()
        .set(&DataKey::RestrictAdminVote, &v);
}

/// Returns whether admin vote restriction is enabled. Defaults to `false`.
pub fn get_restrict_admin_vote(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::RestrictAdminVote)
        .unwrap_or(false)
}

pub fn set_amend_window(env: &Env, v: u64) {
    env.storage().instance().set(&DataKey::AmendWindow, &v);
}

pub fn get_amend_window(env: &Env) -> u64 {
    env.storage().instance().get(&DataKey::AmendWindow).unwrap_or(0)
}

/// Stores the mandatory delay (seconds) a passed proposal must wait before it can be executed.
pub fn set_timelock_duration(env: &Env, v: u64) {
    env.storage().instance().set(&DataKey::TimelockDuration, &v);
}

/// Returns the configured timelock duration in seconds. Defaults to 0 (no delay).
pub fn get_timelock_duration(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::TimelockDuration)
        .unwrap_or(0)
}

/// Sets the contract paused state.
pub fn set_paused(env: &Env, paused: bool) {
    env.storage().instance().set(&DataKey::Paused, &paused);
}

/// Returns `true` if the contract is currently paused.
pub fn is_paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::Paused)
        .unwrap_or(false)
}

/// Stores an optional pause reason string in instance storage.
pub fn set_pause_reason(env: &Env, reason: Option<String>) {
    match reason {
        Some(r) => env.storage().instance().set(&DataKey::PauseReason, &r),
        None => env.storage().instance().remove(&DataKey::PauseReason),
    }
}

/// Returns the stored pause reason, or `None` if not set.
pub fn get_pause_reason(env: &Env) -> Option<String> {
    env.storage().instance().get(&DataKey::PauseReason)
}

pub fn set_min_duration(env: &Env, v: u64) {
    env.storage().instance().set(&DataKey::MinDuration, &v);
}

pub fn get_min_duration(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::MinDuration)
        .unwrap_or(60)
}

pub fn set_max_duration(env: &Env, v: u64) {
    env.storage().instance().set(&DataKey::MaxDuration, &v);
}

pub fn get_max_duration(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::MaxDuration)
        .unwrap_or(2_592_000)
}

/// Stores the pending admin address nominated for rotation.
pub fn set_pending_admin(env: &Env, addr: &Address) {
    env.storage().instance().set(&DataKey::PendingAdmin, addr);
}

/// Returns the pending admin address, if any.
pub fn get_pending_admin(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::PendingAdmin)
}

/// Clears the pending admin nomination.
pub fn clear_pending_admin(env: &Env) {
    env.storage().instance().remove(&DataKey::PendingAdmin);
    env.storage().instance().remove(&DataKey::AdminTransferExpiry);
}

/// Stores the expiry timestamp for the pending admin nomination.
pub fn set_admin_transfer_expiry(env: &Env, ts: u64) {
    env.storage().instance().set(&DataKey::AdminTransferExpiry, &ts);
}

/// Returns the expiry timestamp for the pending admin nomination.
pub fn get_admin_transfer_expiry(env: &Env) -> u64 {
    env.storage().instance().get(&DataKey::AdminTransferExpiry).unwrap_or(0)
}

// =============================================================================
// Multi-sig storage accessors
// =============================================================================

/// Stores the multi-sig admin configuration.
pub fn set_multisig_config(env: &Env, config: &MultiSigConfig) {
    env.storage().instance().set(&DataKey::MultiSigConfig, config);
}

/// Returns the multi-sig config, or `None` if not configured.
pub fn get_multisig_config(env: &Env) -> Option<MultiSigConfig> {
    env.storage().instance().get(&DataKey::MultiSigConfig)
}

/// Increments the multi-sig action counter and returns the new ID.
///
/// # Errors
/// - [`ContractError::ProposalCountOverflow`] if the counter would exceed `u64::MAX`.
pub fn next_multisig_action_id(env: &Env) -> Result<u64, ContractError> {
    let current: u64 = env
        .storage()
        .instance()
        .get(&DataKey::MultiSigActionCount)
        .unwrap_or(0);
    let n = current
        .checked_add(1)
        .ok_or(ContractError::ProposalCountOverflow)?;
    env.storage().instance().set(&DataKey::MultiSigActionCount, &n);
    Ok(n)
}

/// Persists a multi-sig action.
/// TTL is automatically bumped to prevent expiry on long-running actions.
pub fn save_multisig_action(env: &Env, action: &MultiSigAction) {
    env.storage()
        .persistent()
        .set(&DataKey::MultiSigAction(action.id), action);
    bump_multisig_action_ttl(env, action.id);
}

/// Loads a multi-sig action by ID.
///
/// # Errors
/// - [`ContractError::ActionNotFound`] if no action exists for `id`.
pub fn load_multisig_action(env: &Env, id: u64) -> Result<MultiSigAction, ContractError> {
    env.storage()
        .persistent()
        .get(&DataKey::MultiSigAction(id))
        .ok_or(ContractError::ActionNotFound)
}

/// Records that `approver` has approved multi-sig action `action_id`.
/// TTL is automatically bumped to prevent expiry on long-running actions.
pub fn set_multisig_approval(env: &Env, action_id: u64, approver: &Address) {
    env.storage()
        .persistent()
        .set(&DataKey::MultiSigApproval(action_id, approver.clone()), &true);
    bump_multisig_approval_ttl(env, action_id, approver);
}

/// Returns `true` if `approver` has already approved `action_id`.
pub fn has_multisig_approval(env: &Env, action_id: u64, approver: &Address) -> bool {
    env.storage()
        .persistent()
        .get(&DataKey::MultiSigApproval(action_id, approver.clone()))
        .unwrap_or(false)
}

// =============================================================================
// Storage TTL management for long-lived proposals
// =============================================================================

/// Default TTL bump amount for persistent storage entries (in ledgers).
/// ~60 days at 10-second ledger close time (535,680 ledgers).
const DEFAULT_PERSISTENT_STORAGE_TTL: u32 = 535_680;

/// Sets the TTL bump amount for persistent storage entries.
/// This controls how many ledgers into the future persistent entries are bumped.
pub fn set_persistent_storage_ttl(env: &Env, ttl: u32) {
    env.storage().instance().set(&DataKey::PersistentStorageTTL, &ttl);
}

/// Returns the configured TTL bump amount for persistent storage entries.
/// Defaults to [`DEFAULT_PERSISTENT_STORAGE_TTL`] if not configured.
pub fn get_persistent_storage_ttl(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::PersistentStorageTTL)
        .unwrap_or(DEFAULT_PERSISTENT_STORAGE_TTL)
}

/// Bumps the TTL of a proposal entry to prevent expiry on long-running proposals.
pub fn bump_proposal_ttl(env: &Env, proposal_id: u64) {
    let ttl = get_persistent_storage_ttl(env);
    env.storage()
        .persistent()
        .bump(&DataKey::Proposal(proposal_id), ttl);
}

/// Bumps the TTL of a vote record entry.
pub fn bump_vote_record_ttl(env: &Env, proposal_id: u64, voter: &Address) {
    let ttl = get_persistent_storage_ttl(env);
    env.storage()
        .persistent()
        .bump(&DataKey::VoteRecord(proposal_id, voter.clone()), ttl);
}

/// Bumps the TTL of a HasVoted flag entry.
pub fn bump_has_voted_ttl(env: &Env, proposal_id: u64, voter: &Address) {
    let ttl = get_persistent_storage_ttl(env);
    env.storage()
        .persistent()
        .bump(&DataKey::HasVoted(proposal_id, voter.clone()), ttl);
}

/// Bumps the TTL of a voter snapshot entry.
pub fn bump_voter_snapshot_ttl(env: &Env, proposal_id: u64, voter: &Address) {
    let ttl = get_persistent_storage_ttl(env);
    env.storage()
        .persistent()
        .bump(&DataKey::VoterSnapshot(proposal_id, voter.clone()), ttl);
}

/// Bumps the TTL of a LastProposal entry.
pub fn bump_last_proposal_ttl(env: &Env, proposer: &Address) {
    let ttl = get_persistent_storage_ttl(env);
    env.storage()
        .persistent()
        .bump(&DataKey::LastProposal(proposer.clone()), ttl);
}

/// Bumps the TTL of a multi-sig action entry.
pub fn bump_multisig_action_ttl(env: &Env, action_id: u64) {
    let ttl = get_persistent_storage_ttl(env);
    env.storage()
        .persistent()
        .bump(&DataKey::MultiSigAction(action_id), ttl);
}

/// Bumps the TTL of a multi-sig approval entry.
pub fn bump_multisig_approval_ttl(env: &Env, action_id: u64, approver: &Address) {
    let ttl = get_persistent_storage_ttl(env);
    env.storage()
        .persistent()
        .bump(&DataKey::MultiSigApproval(action_id, approver.clone()), ttl);
}
