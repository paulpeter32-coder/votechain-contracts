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

#![no_std]

mod events;
mod storage;
mod types;

#[cfg(test)]
mod prop_tests;
#[cfg(test)]
mod test;
#[cfg(test)]
mod test_ttl;
#[cfg(test)]
pub mod test_helpers;
#[cfg(test)]
mod prop_tests;
#[cfg(test)]
mod integration_tests;

use soroban_sdk::{contract, contractclient, contractimpl, token, Address, Env, String, Vec};
use storage::{
    get_admin as storage_get_admin, get_contract_state, get_last_proposal, get_min_duration, get_min_proposal_balance,
    get_proposal_cooldown, get_restrict_admin_vote, get_timelock_duration, get_version,
    get_amend_window, get_voter_snapshot, get_voting_token, has_voted, is_initialized, is_paused, load_proposal,
    mark_voted, next_id, save_proposal, save_vote_record, save_voter_snapshot, set_admin,
    set_contract_state, set_last_proposal, set_min_duration, set_max_duration,
    set_min_proposal_balance, set_paused, set_proposal_cooldown, set_restrict_admin_vote,
    set_timelock_duration, set_version, set_veto_threshold, set_voting_token, get_vote_record, get_max_duration,
    set_pending_admin, get_pending_admin, clear_pending_admin,
    set_admin_transfer_expiry, get_admin_transfer_expiry,
    set_pause_reason, set_persistent_storage_ttl, get_persistent_storage_ttl,
};
use types::{ContractError, ContractState, DataKey, GovernanceConfig, Proposal, ProposalState, Vote, VoteRecord, MultiSigConfig, MultiSigAction, MultiSigActionType};

const MAX_TITLE_LEN: u32 = 128;
const MAX_DESC_LEN: u32 = 1024;
const MAX_TAGS: u32 = 5;
const MAX_TAG_LEN: u32 = 32;

// SEC-004: Stellar null/zero address used as the sentinel for invalid inputs.
const ZERO_ADDRESS: &str = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF";

// SEC-003: Maximum buffer size for byte-level string validation (matches MAX_DESC_LEN).
const MAX_VALIDATE_BUF: usize = 1024;

/// SEC-003: Validates that a Soroban `String` contains only printable UTF-8 bytes.
///
/// Rejects any byte that is a C0 control character (< 0x20), a null byte (0x00),
/// or the DEL character (0x7F). This prevents injection of control sequences that
/// could corrupt off-chain indexers or log parsers.
///
/// # Errors
/// Returns `err` if any byte in `s` fails the printable-ASCII check.
fn validate_string(s: &String, err: ContractError) -> Result<(), ContractError> {
    let len = s.len() as usize;
    // Stack buffer — len is already bounded by the caller's length check.
    let mut buf = [0u8; MAX_VALIDATE_BUF];
    s.copy_into_slice(&mut buf[..len]);
    for &b in &buf[..len] {
        if b < 0x20 || b == 0x7F {
            return Err(err);
        }
    }
    Ok(())
}

// SEC-004: Rejects the Stellar zero/default address on any address parameter.
fn require_non_zero_address(env: &Env, addr: &Address) -> Result<(), ContractError> {
    if *addr == Address::from_str(env, ZERO_ADDRESS) {
        return Err(ContractError::InvalidAddress);
    }
    Ok(())
}

/// Minimal client for querying the governance token's total supply.
#[contractclient(name = "TokenSupplyClient")]
pub trait TokenSupplyInterface {
    fn total_supply(env: Env) -> i128;
}

/// Returns `Ok(())` if `addr` is in the multi-sig admin list, else `NotMultiSigAdmin`.
fn require_multisig_admin(config: &MultiSigConfig, addr: &Address) -> Result<(), ContractError> {
    for i in 0..config.admins.len() {
        if config.admins.get(i).unwrap() == *addr {
            return Ok(());
        }
    }
    Err(ContractError::NotMultiSigAdmin)
}

/// Executes a multi-sig action after threshold is reached.
/// Marks the action as executed and performs the underlying operation.
fn execute_multisig_action(
    env: &Env,
    action_id: u64,
    action_type: &MultiSigActionType,
    proposal_id: u64,
    new_config: &Option<MultiSigConfig>,
) -> Result<(), ContractError> {
    // Mark executed first to prevent re-entrancy.
    let mut action = load_multisig_action(env, action_id)?;
    action.executed = true;
    save_multisig_action(env, &action);

    match action_type {
        MultiSigActionType::ExecuteProposal => {
            let mut proposal = load_proposal(env, proposal_id)?;
            if proposal.state != ProposalState::Passed {
                return Err(ContractError::ProposalNotPassed);
            }
            if env.ledger().timestamp() < proposal.execute_after {
                return Err(ContractError::TimelockNotExpired);
            }
            proposal.state = ProposalState::Executed;
            save_proposal(env, &proposal);
            events::proposal_executed(env, proposal_id);
        }
        MultiSigActionType::CancelProposal => {
            let mut proposal = load_proposal(env, proposal_id)?;
            if proposal.state != ProposalState::Active {
                return Err(ContractError::ProposalNotActive);
            }
            proposal.state = ProposalState::Cancelled;
            save_proposal(env, &proposal);
            events::proposal_cancelled(env, proposal_id);
        }
        MultiSigActionType::UpdateMultiSig => {
            let config = new_config.clone().ok_or(ContractError::MultiSigNotConfigured)?;
            if config.admins.is_empty() {
                return Err(ContractError::EmptyAdminList);
            }
            if config.threshold == 0 || config.threshold > config.admins.len() {
                return Err(ContractError::InvalidThreshold);
            }
            let threshold = config.threshold;
            set_multisig_config(env, &config);
            events::multisig_config_updated(env, threshold);
        }
        MultiSigActionType::Pause => {
            set_paused(env, true);
            // Emit using the stored admin address as the actor.
            if let Ok(admin) = get_admin(env) {
                events::contract_paused(env, &admin);
            }
        }
        MultiSigActionType::Unpause => {
            if !is_paused(env) {
                return Err(ContractError::NotPaused);
            }
            set_paused(env, false);
            if let Ok(admin) = get_admin(env) {
                events::contract_unpaused(env, &admin);
            }
        }
    }

    events::multisig_action_executed(env, action_id, action_type);
    Ok(())
}

#[contract]
pub struct GovernanceContract;

#[contractimpl]
impl GovernanceContract {
    /// Initialises the governance contract with an admin and a voting token.
    ///
    /// Must be called exactly once before any other function.
    ///
    /// # Parameters
    /// - `admin` – Address with privileged operations (execute, cancel, pause).
    /// - `voting_token` – Address of the SEP-41-compatible governance token contract.
    /// - `min_proposal_balance` – Minimum token balance required to create proposals (`0` = no minimum).
    /// - `proposal_cooldown` – Seconds between proposals per address (`0` = no cooldown).
    /// - `min_duration`: minimum allowed voting duration in seconds (e.g., 3600 for 1 hour)
    /// - `max_duration`: maximum allowed voting duration in seconds (e.g., 2592000 for 30 days)
    /// - `restrict_admin_vote`: when `true`, the admin address cannot cast votes on proposals
    ///   they created, preventing a conflict of interest.
    /// - `amend_window`: number of seconds after creation during which the proposer may
    ///   change the title and description before voting begins.
    /// - `timelock_duration`: mandatory delay in seconds between a proposal passing and it
    ///   becoming executable. Use `0` to disable the timelock.
    /// - `veto_threshold`: vote weight threshold that rejects a proposal immediately when
    ///   `votes_no >= veto_threshold`. Use `0` to disable the veto mechanism.
    /// - `persistent_storage_ttl`: TTL bump amount in ledgers for persistent storage entries;
    ///   controls how long proposals and votes survive (default ~60 days). Use `0` to use default.
    ///
    /// # Errors
    /// - [`ContractError::AlreadyInitialized`] if the contract has already been initialised.
    /// - [`ContractError::InvalidAddress`] if `admin` or `voting_token` is the zero address.
    ///
    /// # Example
    /// ```text
    /// GovernanceContract::initialize(
    ///     env, admin, token,
    ///     1_000_000,  // min 1M tokens to propose
    ///     86_400,     // 1-day cooldown
    ///     3_600,      // min 1-hour voting window
    ///     2_592_000,  // max 30-day voting window
    ///     true,       // restrict admin voting
    ///     0,          // no timelock
    ///     0,          // veto threshold disabled
    ///     535_680,    // TTL: ~60 days
    /// )?;
    /// ```
    pub fn initialize(
        env: Env,
        admin: Address,
        voting_token: Address,
        min_proposal_balance: i128,
        proposal_cooldown: u64,
        min_duration: u64,
        max_duration: u64,
        restrict_admin_vote: bool,
        amend_window: u64,
        timelock_duration: u64,
        veto_threshold: i128,
        persistent_storage_ttl: u32,
    ) -> Result<(), ContractError> {
        // SEC-005: auth is the first operation in every privileged function.
        admin.require_auth();
        // SEC-004: reject zero addresses before any state change.
        require_non_zero_address(&env, &admin)?;
        require_non_zero_address(&env, &voting_token)?;
        if is_initialized(&env) {
            return Err(ContractError::AlreadyInitialized);
        }
        // SEP-41 compliance check: verify the token contract exposes balance() and total_supply().
        let token_client = token::Client::new(&env, &voting_token);
        if token_client.try_balance(&admin).is_err() {
            return Err(ContractError::InvalidTokenContract);
        }
        if TokenSupplyClient::new(&env, &voting_token).try_total_supply().is_err() {
            return Err(ContractError::InvalidTokenContract);
        }
        set_admin(&env, &admin);
        set_voting_token(&env, &voting_token);
        let supply = TokenSupplyClient::new(&env, &voting_token).total_supply();
        if veto_threshold < 0 || veto_threshold > supply {
            return Err(ContractError::InvalidVetoThreshold);
        }
        if min_proposal_balance > 0 {
            set_min_proposal_balance(&env, min_proposal_balance);
        }
        if proposal_cooldown > 0 {
            set_proposal_cooldown(&env, proposal_cooldown);
        }
        set_min_duration(&env, min_duration);
        set_max_duration(&env, max_duration);
        set_restrict_admin_vote(&env, restrict_admin_vote);
        if amend_window > 0 {
            set_amend_window(&env, amend_window);
        }
        if timelock_duration > 0 {
            set_timelock_duration(&env, timelock_duration);
        }
        set_veto_threshold(&env, veto_threshold);
        if persistent_storage_ttl > 0 {
            set_persistent_storage_ttl(&env, persistent_storage_ttl);
        }
        set_version(&env, (1, 0, 0));
        set_contract_state(&env, &ContractState::Ready);
        events::contract_initialized(&env, &admin);
        Ok(())
    }

    /// Creates a new governance proposal.
    ///
    /// # Returns
    /// The numeric ID assigned to the new proposal.
    ///
    /// # Parameters
    /// - `proposer` – Address creating the proposal (must have sufficient balance).
    /// - `title` – Proposal title (1–128 characters, printable UTF-8 only).
    /// - `description` – Proposal description (1–1024 characters, printable UTF-8 only).
    /// - `quorum` – Minimum total votes required for the proposal to pass (must be > 0 and ≤ total supply).
    /// - `duration` – Voting period in seconds; must be within the [min_duration, max_duration] range set at init.
    ///
    /// # Errors
    /// - [`ContractError::InvalidAddress`] if `proposer` is the zero address.
    /// - [`ContractError::InvalidTitle`] if `title` is empty or exceeds 256 characters.
    /// - [`ContractError::InvalidDescription`] if `description` is empty or exceeds 4096 characters.
    /// - [`ContractError::InvalidQuorum`] if `quorum` is zero or negative.
    /// - [`ContractError::QuorumExceedsSupply`] if `quorum` exceeds the total token supply.
    /// - [`ContractError::InvalidDurationRange`] if `duration` is outside the configured [min_duration, max_duration] range.
    /// - [`ContractError::InsufficientBalance`] if proposer balance is below minimum.
    /// - [`ContractError::ProposalCooldown`] if proposer is within cooldown period.
    /// - [`ContractError::ProposalCountOverflow`] if the proposal ID counter would overflow.
    ///
    /// # Example
    /// ```text
    /// let id = GovernanceContract::create_proposal(
    ///     env, proposer,
    ///     String::from_slice(&env, "Increase Treasury"),
    ///     String::from_slice(&env, "Allocate 10M tokens to treasury"),
    ///     5_000_000,  // 5M token quorum
    ///     604_800,    // 7 days
    /// )?;
    /// ```
    pub fn create_proposal(
        env: Env,
        proposer: Address,
        title: String,
        description: String,
        quorum: i128,
        duration: u64,
        tags: Vec<String>,
    ) -> Result<u64, ContractError> {
        // SEC-005: auth first.
        proposer.require_auth();
        // SEC-004: reject zero address.
        require_non_zero_address(&env, &proposer)?;
        if is_paused(&env) {
            return Err(ContractError::ContractPaused);
        }

        // Title: non-empty, max 128 chars, printable bytes only (SEC-003)
        let title_len = title.len();
        if title_len == 0 || title_len > MAX_TITLE_LEN {
            return Err(ContractError::InvalidTitle);
        }
        validate_string(&title, ContractError::InvalidTitle)?;
        // Description: non-empty, max 1024 chars, printable bytes only (SEC-003)
        let desc_len = description.len();
        if desc_len == 0 || desc_len > MAX_DESC_LEN {
            return Err(ContractError::InvalidDescription);
        }
        validate_string(&description, ContractError::InvalidDescription)?;

        // Tags: max 5, each max 32 chars
        if tags.len() > MAX_TAGS {
            return Err(ContractError::TooManyTags);
        }
        for tag in tags.iter() {
            if tag.len() > MAX_TAG_LEN {
                return Err(ContractError::TagTooLong);
            }
        }

        // Quorum: > 0
        if quorum <= 0 {
            return Err(ContractError::InvalidQuorum);
        }
        // Duration: zero is explicitly rejected before the range check so callers
        // receive InvalidDuration (not InvalidDurationRange) for the zero case.
        if duration == 0 {
            return Err(ContractError::InvalidDuration);
        }
        // Duration: within [min_duration, max_duration] as configured at init.
        let min_dur = get_min_duration(&env);
        let max_dur = get_max_duration(&env);
        if duration < min_dur || duration > max_dur {
            return Err(ContractError::InvalidDurationRange);
        }

        let token_client = token::Client::new(&env, &get_voting_token(&env)?);

        // Quorum must not exceed total token supply
        let supply = TokenSupplyClient::new(&env, &get_voting_token(&env)?).total_supply();
        if quorum > supply {
            return Err(ContractError::QuorumExceedsSupply);
        }

        let min_balance = get_min_proposal_balance(&env);
        if min_balance > 0 {
            let balance = token_client.balance(&proposer);
            if balance < min_balance {
                return Err(ContractError::InsufficientBalance);
            }
        }

        let cooldown = get_proposal_cooldown(&env);
        if cooldown > 0 {
            let now = env.ledger().timestamp();
            let last = get_last_proposal(&env, &proposer);
            if last > 0 && now < last + cooldown {
                return Err(ContractError::ProposalCooldown);
            }
        }

        let now = env.ledger().timestamp();
        let start_time = now + get_amend_window(&env);
        // SEC-007: ID is generated contract-side only; checked_add prevents overflow.
        let id = next_id(&env)?;

        // SEC-014: Verify that the monotonic ID does not collide with an existing proposal.
        // This is a defense-in-depth measure; next_id() is the sole source of IDs.
        if env.storage().persistent().has(&DataKey::Proposal(id)) {
            return Err(ContractError::ProposalAlreadyExists);
        }

        let proposal = Proposal {
            id,
            proposer: proposer.clone(),
            title,
            description,
            votes_yes: 0,
            votes_no: 0,
            votes_abstain: 0,
            quorum,
            start_time,
            end_time: start_time + duration,
            state: ProposalState::Active,
            execute_after: 0,
            tags,
        };
        save_proposal(&env, &proposal);
        set_last_proposal(&env, &proposer, now);
        events::proposal_created(&env, id, &proposer);
        Ok(id)
    }

    /// Amends the title and description of an active proposal before voting starts.
    ///
    /// Only the original proposer may call this, and only within the configured
    /// amendment window before the proposal's voting window begins.
    ///
    /// # Errors
    /// - [`ContractError::InvalidAddress`] if `proposer` is the zero address.
    /// - [`ContractError::ProposalNotFound`] if `proposal_id` does not exist.
    /// - [`ContractError::ProposalNotActive`] if the proposal is not in `Active` status.
    /// - [`ContractError::NotProposalOwner`] if the caller is not the original proposer.
    /// - [`ContractError::ProposalAmendmentNotAllowed`] if the amendment window has closed.
    /// - [`ContractError::InvalidTitle`] if `title` is empty or exceeds the maximum length.
    /// - [`ContractError::InvalidDescription`] if `description` is empty or exceeds the maximum length.
    /// - [`ContractError::ContractPaused`] if the contract is paused.
    pub fn amend_proposal(
        env: Env,
        proposer: Address,
        proposal_id: u64,
        title: String,
        description: String,
    ) -> Result<(), ContractError> {
        proposer.require_auth();
        require_non_zero_address(&env, &proposer)?;
        if is_paused(&env) {
            return Err(ContractError::ContractPaused);
        }

        let mut proposal = load_proposal(&env, proposal_id)?;
        if proposal.state != ProposalState::Active {
            return Err(ContractError::ProposalNotActive);
        }
        if proposal.proposer != proposer {
            return Err(ContractError::NotProposalOwner);
        }

        let now = env.ledger().timestamp();
        if now >= proposal.start_time {
            return Err(ContractError::ProposalAmendmentNotAllowed);
        }

        let title_len = title.len();
        if title_len == 0 || title_len > MAX_TITLE_LEN {
            return Err(ContractError::InvalidTitle);
        }
        validate_string(&title, ContractError::InvalidTitle)?;

        let desc_len = description.len();
        if desc_len == 0 || desc_len > MAX_DESC_LEN {
            return Err(ContractError::InvalidDescription);
        }
        validate_string(&description, ContractError::InvalidDescription)?;

        proposal.title = title.clone();
        proposal.description = description.clone();
        save_proposal(&env, &proposal);
        events::proposal_amended(&env, proposal_id, &proposer, &title, &description);
        Ok(())
    }

    /// Casts a vote on an active proposal.
    ///
    /// The voter's token balance at call time is captured as the vote weight and stored
    /// immutably, preventing balance manipulation after the vote is recorded.
    ///
    /// # Parameters
    /// - `voter` – Address casting the vote; must authorise the call and hold tokens.
    /// - `proposal_id` – ID of the proposal to vote on.
    /// - `vote` – Vote direction: `Vote::Yes`, `Vote::No`, or `Vote::Abstain`.
    ///
    /// # Errors
    /// - [`ContractError::InvalidAddress`] if `voter` is the zero address.
    /// - [`ContractError::ProposalNotFound`] if `proposal_id` does not exist.
    /// - [`ContractError::ProposalNotActive`] if the proposal is not in `Active` status.
    /// - [`ContractError::VotingNotStarted`] if the current ledger timestamp is before `start_time`.
    /// - [`ContractError::VotingPeriodEnded`] if the current ledger timestamp is after `end_time`.
    /// - [`ContractError::AlreadyVoted`] if the voter has already voted on this proposal.
    /// - [`ContractError::NoVotingPower`] if the voter's token balance is zero.
    /// - [`ContractError::VoteTallyOverflow`] if adding the vote weight would overflow `i128`.
    /// - [`ContractError::AdminVoteRestricted`] if `restrict_admin_vote` is enabled and the admin
    ///   attempts to vote on a proposal they created.
    /// - [`ContractError::ContractPaused`] if the contract is paused.
    ///
    /// # Example
    /// ```text
    /// GovernanceContract::cast_vote(env, voter, proposal_id, Vote::Yes)?;
    /// ```
    pub fn cast_vote(
        env: Env,
        voter: Address,
        proposal_id: u64,
        vote: Vote,
    ) -> Result<(), ContractError> {
        // SEC-005: auth first.
        voter.require_auth();
        // SEC-004: reject zero address.
        require_non_zero_address(&env, &voter)?;
        if is_paused(&env) {
            return Err(ContractError::ContractPaused);
        }

        let proposal = load_proposal(&env, proposal_id)?;
        if proposal.state != ProposalState::Active {
            return Err(ContractError::ProposalNotActive);
        }

        let now = env.ledger().timestamp();
        if now < proposal.start_time {
            return Err(ContractError::VotingNotStarted);
        }
        if now >= proposal.end_time {
            return Err(ContractError::VotingPeriodEnded);
        }
        if has_voted(&env, proposal_id, &voter) {
            return Err(ContractError::AlreadyVoted);
        }

        if get_restrict_admin_vote(&env) {
            let admin = storage_get_admin(&env)?;
            if voter == admin && proposal.proposer == admin {
                return Err(ContractError::AdminVoteRestricted);
            }
        }

        // SEC-010 (checks-effects-interactions): write the dedup flag BEFORE the
        // cross-contract call so that even if the execution model ever allowed
        // re-entry, a second cast_vote for the same voter would be rejected.
        mark_voted(&env, proposal_id, &voter);

        let token_client = token::Client::new(&env, &get_voting_token(&env)?);
        // Snapshot: capture the voter's balance at vote time and persist it.
        // Using the stored snapshot (rather than re-querying) prevents any
        // balance manipulation after the vote is recorded.
        let weight = match get_voter_snapshot(&env, proposal_id, &voter) {
            Some(w) => w,
            None => {
                let live = token_client.balance(&voter);
                save_voter_snapshot(&env, proposal_id, &voter, live);
                live
            }
        };
        if weight <= 0 {
            return Err(ContractError::NoVotingPower);
        }

        let mut proposal = proposal;
        match vote {
            Vote::Yes => {
                proposal.votes_yes = proposal
                    .votes_yes
                    .checked_add(weight)
                    .ok_or(ContractError::VoteTallyOverflow)?
            }
            Vote::No => {
                proposal.votes_no = proposal
                    .votes_no
                    .checked_add(weight)
                    .ok_or(ContractError::VoteTallyOverflow)?
            }
            Vote::Abstain => {
                proposal.votes_abstain = proposal
                    .votes_abstain
                    .checked_add(weight)
                    .ok_or(ContractError::VoteTallyOverflow)?
            }
        }

        save_vote_record(
            &env,
            proposal_id,
            &voter,
            &VoteRecord {
                vote_type: vote.clone(),
                weight,
            },
        );
        save_proposal(&env, &proposal);
        events::vote_cast(&env, proposal_id, &voter, &vote, weight);
        if proposal.state == ProposalState::Rejected && veto_threshold > 0 && vote == Vote::No {
            events::proposal_vetoed(&env, proposal_id, proposal.votes_no, veto_threshold);
        }
        Ok(())
    }

    /// Returns the vote record (type and weight) for a specific voter on a proposal.
    ///
    /// Returns `None` for non-voters without reverting. Read-only.
    pub fn get_vote(env: Env, proposal_id: u64, voter: Address) -> Option<VoteRecord> {
        get_vote_record(&env, proposal_id, &voter)
    }

    /// Finalises a proposal after its voting period has ended.
    ///
    /// Computes the outcome using the following rules:
    ///
    /// ```text
    /// total_votes = votes_yes + votes_no + votes_abstain
    ///
    /// Passed   if total_votes >= quorum AND votes_yes > votes_no
    /// Rejected otherwise (quorum not met, or votes_yes <= votes_no)
    /// ```
    ///
    /// Abstain votes count toward the quorum threshold but do not influence
    /// the yes/no majority comparison. A tie (`votes_yes == votes_no`) resolves
    /// as Rejected even when quorum is met.
    ///
    /// # Errors
    /// - [`ContractError::ProposalNotFound`] if `proposal_id` does not exist.
    /// - [`ContractError::ProposalNotActive`] if the proposal is not in `Active` status.
    /// - [`ContractError::VotingStillOpen`] if the voting window has not yet closed.
    pub fn finalise(env: Env, proposal_id: u64) -> Result<(), ContractError> {
        if is_paused(&env) {
            return Err(ContractError::ContractPaused);
        }
        let mut proposal = load_proposal(&env, proposal_id)?;
        if proposal.state != ProposalState::Active {
            return Err(ContractError::ProposalNotActive);
        }
        let now = env.ledger().timestamp();
        if now <= proposal.end_time {
            return Err(ContractError::VotingStillOpen);
        }

        let total = proposal.votes_yes + proposal.votes_no + proposal.votes_abstain;
        if total >= proposal.quorum && proposal.votes_yes > proposal.votes_no {
            let timelock = get_timelock_duration(&env);
            proposal.execute_after = now + timelock;
            proposal.state = ProposalState::Passed;
        } else {
            proposal.state = ProposalState::Rejected;
        }

        save_proposal(&env, &proposal);
        events::proposal_finalised(&env, proposal_id, &proposal.state, proposal.execute_after);
        Ok(())
    }

    /// Marks a passed proposal as executed. Only the admin may call this.
    ///
    /// # Errors
    /// - [`ContractError::InvalidAddress`] if `admin` is the zero address.
    /// - [`ContractError::NotAdmin`] if `admin` does not match the stored admin.
    /// - [`ContractError::ProposalNotFound`] if `proposal_id` does not exist.
    /// - [`ContractError::ProposalNotPassed`] if the proposal has not passed.
    pub fn execute(env: Env, admin: Address, proposal_id: u64) -> Result<(), ContractError> {
        // SEC-005: auth first.
        admin.require_auth();
        // SEC-004: reject zero address.
        require_non_zero_address(&env, &admin)?;
        if is_paused(&env) {
            return Err(ContractError::ContractPaused);
        }
        if storage_get_admin(&env)? != admin {
            return Err(ContractError::NotAdmin);
        }
        let mut proposal = load_proposal(&env, proposal_id)?;
        if proposal.state != ProposalState::Passed {
            return Err(ContractError::ProposalNotPassed);
        }
        if env.ledger().timestamp() < proposal.execute_after {
            return Err(ContractError::TimelockNotExpired);
        }
        proposal.state = ProposalState::Executed;
        save_proposal(&env, &proposal);
        events::proposal_executed(&env, proposal_id);
        Ok(())
    }

    /// Cancels an active proposal. Only the admin may cancel.
    ///
    /// # Errors
    /// - [`ContractError::InvalidAddress`] if `admin` is the zero address.
    /// - [`ContractError::NotAdmin`] if `admin` does not match the stored admin.
    /// - [`ContractError::ProposalNotFound`] if `proposal_id` does not exist.
    /// - [`ContractError::ProposalNotActive`] if the proposal is not in `Active` status.
    pub fn cancel(env: Env, admin: Address, proposal_id: u64) -> Result<(), ContractError> {
        // SEC-005: auth first.
        admin.require_auth();
        // SEC-004: reject zero address.
        require_non_zero_address(&env, &admin)?;
        if is_paused(&env) {
            return Err(ContractError::ContractPaused);
        }
        if storage_get_admin(&env)? != admin {
            return Err(ContractError::NotAdmin);
        }
        let mut proposal = load_proposal(&env, proposal_id)?;
        if proposal.state != ProposalState::Active {
            return Err(ContractError::ProposalNotActive);
        }
        proposal.state = ProposalState::Cancelled;
        save_proposal(&env, &proposal);
        events::proposal_cancelled(&env, proposal_id);
        Ok(())
    }

    /// Updates the quorum threshold of an active proposal. Only the admin may call this.
    ///
    /// # Errors
    /// - [`ContractError::InvalidAddress`] if `admin` is the zero address.
    /// - [`ContractError::NotAdmin`] if `admin` does not match the stored admin.
    /// - [`ContractError::InvalidQuorum`] if `new_quorum` is zero or negative.
    /// - [`ContractError::ProposalNotFound`] if `proposal_id` does not exist.
    /// - [`ContractError::ProposalNotActive`] if the proposal is not in `Active` status.
    pub fn update_quorum(
        env: Env,
        admin: Address,
        proposal_id: u64,
        new_quorum: i128,
    ) -> Result<(), ContractError> {
        // SEC-005: auth first.
        admin.require_auth();
        // SEC-004: reject zero address.
        require_non_zero_address(&env, &admin)?;
        if is_paused(&env) {
            return Err(ContractError::ContractPaused);
        }
        if storage_get_admin(&env)? != admin {
            return Err(ContractError::NotAdmin);
        }
        if new_quorum <= 0 {
            return Err(ContractError::InvalidQuorum);
        }
        let mut proposal = load_proposal(&env, proposal_id)?;
        if proposal.state != ProposalState::Active {
            return Err(ContractError::ProposalNotActive);
        }
        proposal.quorum = new_quorum;
        save_proposal(&env, &proposal);
        events::quorum_updated(&env, proposal_id, new_quorum);
        Ok(())
    }

    /// Transfers admin rights to a new address. Only the current admin may call this.
    ///
    /// The old admin loses all privileges immediately upon successful transfer.
    ///
    /// # Errors
    /// - [`ContractError::InvalidAddress`] if `admin` or `new_admin` is the zero address.
    /// - [`ContractError::NotAdmin`] if `admin` does not match the stored admin.
    pub fn transfer_admin(
        env: Env,
        admin: Address,
        new_admin: Address,
    ) -> Result<(), ContractError> {
        // SEC-005: auth first.
        admin.require_auth();
        // SEC-004: reject zero addresses for both parameters.
        require_non_zero_address(&env, &admin)?;
        require_non_zero_address(&env, &new_admin)?;
        if is_paused(&env) {
            return Err(ContractError::ContractPaused);
        }
        if storage_get_admin(&env)? != admin {
            return Err(ContractError::NotAdmin);
        }
        set_admin(&env, &new_admin);
        events::admin_transferred(&env, &admin, &new_admin);
        Ok(())
    }

    /// SEC-006: Proposes a two-step admin key rotation.
    ///
    /// Nominates `new_admin` with an acceptance window of `window_secs` seconds
    /// (default 48 h when 0).  The admin key is NOT transferred until the nominee
    /// calls [`accept_admin_transfer`] within the window.
    ///
    /// # Errors
    /// - [`ContractError::InvalidAddress`] if either address is the zero address.
    /// - [`ContractError::NotAdmin`] if `admin` does not match the stored admin.
    /// - [`ContractError::ContractPaused`] if the contract is paused.
    pub fn propose_admin_transfer(
        env: Env,
        admin: Address,
        new_admin: Address,
        window_secs: u64,
    ) -> Result<(), ContractError> {
        admin.require_auth();
        require_non_zero_address(&env, &admin)?;
        require_non_zero_address(&env, &new_admin)?;
        if is_paused(&env) {
            return Err(ContractError::ContractPaused);
        }
        if storage_get_admin(&env)? != admin {
            return Err(ContractError::NotAdmin);
        }
        let window = if window_secs == 0 {
            172_800
        } else {
            window_secs
        }; // default 48 h
        let expiry = env.ledger().timestamp() + window;
        set_pending_admin(&env, &new_admin);
        set_admin_transfer_expiry(&env, expiry);
        events::admin_transfer_proposed(&env, &admin, &new_admin, expiry);
        Ok(())
    }

    /// SEC-006: Accepts a pending admin key rotation.
    ///
    /// Must be called by the nominated address before the acceptance window expires.
    /// On success the caller becomes the new admin and the nomination is cleared.
    ///
    /// # Errors
    /// - [`ContractError::InvalidAddress`] if `new_admin` is the zero address.
    /// - [`ContractError::PendingAdminNotSet`] if no nomination is outstanding.
    /// - [`ContractError::NotPendingAdmin`] if `new_admin` is not the nominated address.
    /// - [`ContractError::AdminTransferExpired`] if the acceptance window has passed.
    /// - [`ContractError::ContractPaused`] if the contract is paused.
    pub fn accept_admin_transfer(env: Env, new_admin: Address) -> Result<(), ContractError> {
        new_admin.require_auth();
        require_non_zero_address(&env, &new_admin)?;
        if is_paused(&env) {
            return Err(ContractError::ContractPaused);
        }
        let pending = get_pending_admin(&env).ok_or(ContractError::PendingAdminNotSet)?;
        if pending != new_admin {
            return Err(ContractError::NotPendingAdmin);
        }
        if env.ledger().timestamp() > get_admin_transfer_expiry(&env) {
            clear_pending_admin(&env);
            return Err(ContractError::AdminTransferExpired);
        }
        let old_admin = storage_get_admin(&env)?;
        set_admin(&env, &new_admin);
        clear_pending_admin(&env);
        events::admin_transferred(&env, &old_admin, &new_admin);
        Ok(())
    }

    /// Pauses the contract, blocking all state-changing operations.
    ///
    /// Read-only functions (`get_proposal`, `get_vote`, `has_voted`, etc.) remain
    /// available while paused. Only the admin may call this.
    ///
    /// # Errors
    /// - [`ContractError::InvalidAddress`] if `admin` is the zero address.
    /// - [`ContractError::NotAdmin`] if `admin` does not match the stored admin.
    pub fn pause(env: Env, admin: Address, reason: Option<String>) -> Result<(), ContractError> {
        admin.require_auth();
        require_non_zero_address(&env, &admin)?;
        if storage_get_admin(&env)? != admin {
            return Err(ContractError::NotAdmin);
        }
        set_paused(&env, true);
        set_pause_reason(&env, reason.clone());
        events::contract_paused(&env, &admin, reason);
        Ok(())
    }

    /// Unpauses the contract, restoring all state-changing operations.
    ///
    /// Only the admin may call this.
    ///
    /// # Errors
    /// - [`ContractError::InvalidAddress`] if `admin` is the zero address.
    /// - [`ContractError::NotAdmin`] if `admin` does not match the stored admin.
    /// - [`ContractError::NotPaused`] if the contract is not currently paused.
    pub fn unpause(env: Env, admin: Address) -> Result<(), ContractError> {
        admin.require_auth();
        require_non_zero_address(&env, &admin)?;
        if storage_get_admin(&env)? != admin {
            return Err(ContractError::NotAdmin);
        }
        if !is_paused(&env) {
            return Err(ContractError::NotPaused);
        }
        set_paused(&env, false);
        set_pause_reason(&env, None);
        events::contract_unpaused(&env, &admin);
        Ok(())
    }

    /// Returns whether the contract is currently paused.
    pub fn paused(env: Env) -> bool {
        is_paused(&env)
    }

    /// Returns the optional pause reason string stored on-chain.
    pub fn get_pause_reason(env: Env) -> Option<String> {
        // Call the storage accessor directly to avoid name collision.
        storage::get_pause_reason(&env)
    }

    /// Returns the full state of a proposal.
    ///
    /// # Errors
    /// - [`ContractError::ProposalNotFound`] if `proposal_id` does not exist.
    pub fn get_proposal(env: Env, proposal_id: u64) -> Result<Proposal, ContractError> {
        load_proposal(&env, proposal_id)
    }

    /// Returns the total number of proposals ever created.
    pub fn proposal_count(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::ProposalCount)
            .unwrap_or(0)
    }

    /// Returns whether an address has already voted on a given proposal.
    pub fn has_voted(env: Env, proposal_id: u64, voter: Address) -> Result<bool, ContractError> {
        require_non_zero_address(&env, &voter)?;
        load_proposal(&env, proposal_id)?;
        Ok(has_voted(&env, proposal_id, &voter))
    }

    /// Returns the contract version as a `(major, minor, patch)` semver tuple.
    pub fn get_version(env: Env) -> (u32, u32, u32) {
        get_version(&env)
    }

    /// Returns the contract lifecycle state.
    pub fn get_state(env: Env) -> ContractState {
        get_contract_state(&env)
    }

    /// Performs a one-time migration of on-chain storage from v1 -> v2.
    ///
    /// Callable by the admin after upgrading contract code. Validates that
    /// the previously-deployed storage layout is present before performing
    /// the migration. Safe to call multiple times (idempotent).
    pub fn migrate(env: Env, admin: Address) -> Result<(), ContractError> {
        // auth first
        admin.require_auth();
        require_non_zero_address(&env, &admin)?;
        if get_admin(&env)? != admin {
            return Err(ContractError::NotAdmin);
        }

        let old_version = get_version(&env);
        // Already migrated or newer: noop
        if old_version >= (2, 0, 0) {
            return Ok(());
        }

        // Basic validation: ensure the contract was properly initialized.
        if !env.storage().instance().has(&DataKey::Admin) {
            return Err(ContractError::MigrationFailed);
        }

        // Place migration logic here. Currently no structural key renames are
        // necessary; this is the canonical place to transform any values that
        // need reshaping between versions.

        // Bump version to v2.0.0 and emit event.
        set_version(&env, (2, 0, 0));
        events::migration_completed(&env, old_version, (2, 0, 0));
        Ok(())
    }

    /// Returns a paginated slice of proposals ordered by ascending proposal ID.
    ///
    /// # Parameters
    /// - `offset` – Zero-based index of the first proposal to return (i.e. skip the first
    ///   `offset` proposals). Pass `0` to start from the beginning.
    /// - `limit` – Maximum number of proposals to return. Capped internally at `50`; passing
    ///   a larger value silently uses `50`.
    ///
    /// # Returns
    /// A [`soroban_sdk::Vec<Proposal>`] containing up to `limit` proposals starting at
    /// `offset`. Returns an empty vector when `offset >= proposal_count`.
    ///
    /// # Example
    /// ```text
    /// // First page (proposals 1-10)
    /// list_proposals(env, 0, 10)
    ///
    /// // Second page (proposals 11-20)
    /// list_proposals(env, 10, 10)
    /// ```
    pub fn list_proposals(env: Env, offset: u64, limit: u64) -> soroban_sdk::Vec<Proposal> {        const MAX_LIMIT: u64 = 50;

        let total = env
            .storage()
            .instance()
            .get(&DataKey::ProposalCount)
            .unwrap_or(0);

        if offset >= total {
            return soroban_sdk::Vec::new(&env);
        }

        let effective_limit = if limit > MAX_LIMIT { MAX_LIMIT } else { limit };
        let start_id = offset + 1;
        let end_id = (offset + effective_limit).min(total);

        let mut proposals = soroban_sdk::Vec::new(&env);
        for id in start_id..=end_id {
            if let Ok(proposal) = load_proposal(&env, id) {
                proposals.push_back(proposal);
            }
        }

        proposals
    }

    /// Upgrades the contract to a new semantic version.
    ///
    /// Validates that the target version is strictly greater than the current
    /// version (major, minor, patch lexicographic order) to prevent downgrades.
    /// Only the admin may call this.
    ///
    /// # Errors
    /// - [`ContractError::InvalidAddress`] if `admin` is the zero address.
    /// - [`ContractError::NotAdmin`] if `admin` does not match the stored admin.
    /// - [`ContractError::DowngradeNotAllowed`] if `new_version` is not strictly
    ///   greater than the current version.
    pub fn upgrade(
        env: Env,
        admin: Address,
        new_version: (u32, u32, u32),
    ) -> Result<(), ContractError> {
        admin.require_auth();
        require_non_zero_address(&env, &admin)?;
        if storage_get_admin(&env)? != admin {
            return Err(ContractError::NotAdmin);
        }
        let current = get_version(&env);
        if new_version <= current {
            return Err(ContractError::DowngradeNotAllowed);
        }
        set_version(&env, new_version);
        events::contract_upgraded(&env, current, new_version);
        Ok(())
    }

    // =========================================================================
    // Multi-sig admin operations (SC-003)
    // =========================================================================

    /// Configures the multi-sig admin list and threshold.
    ///
    /// Can be called by the current single admin to bootstrap multi-sig, or
    /// via an approved `UpdateMultiSig` multi-sig action once multi-sig is active.
    ///
    /// # Errors
    /// - [`ContractError::NotAdmin`] if `caller` is not the current admin.
    /// - [`ContractError::EmptyAdminList`] if `admins` is empty.
    /// - [`ContractError::InvalidThreshold`] if `threshold` is 0 or > len(admins).
    pub fn initialize_multisig(
        env: Env,
        caller: Address,
        admins: Vec<Address>,
        threshold: u32,
    ) -> Result<(), ContractError> {
        caller.require_auth();
        require_non_zero_address(&env, &caller)?;
        if get_admin(&env)? != caller {
            return Err(ContractError::NotAdmin);
        }
        if admins.is_empty() {
            return Err(ContractError::EmptyAdminList);
        }
        let n = admins.len();
        if threshold == 0 || threshold > n {
            return Err(ContractError::InvalidThreshold);
        }
        let config = MultiSigConfig { admins, threshold };
        set_multisig_config(&env, &config);
        events::multisig_config_updated(&env, threshold);
        Ok(())
    }

    /// Proposes a privileged action that requires multi-sig approval.
    ///
    /// The proposer must be in the multi-sig admin list. Their approval is
    /// counted automatically (first approval).
    ///
    /// # Parameters
    /// - `action_type` — the type of action to perform.
    /// - `proposal_id` — relevant proposal ID (for Execute/Cancel actions; pass 0 otherwise).
    /// - `new_config` — new multi-sig config (for UpdateMultiSig action; pass `None` otherwise).
    ///
    /// # Returns
    /// The new action ID.
    ///
    /// # Errors
    /// - [`ContractError::MultiSigNotConfigured`] if multi-sig has not been set up.
    /// - [`ContractError::NotMultiSigAdmin`] if `proposer` is not in the admin list.
    pub fn propose_multisig_action(
        env: Env,
        proposer: Address,
        action_type: MultiSigActionType,
        proposal_id: u64,
        new_config: Option<MultiSigConfig>,
    ) -> Result<u64, ContractError> {
        proposer.require_auth();
        require_non_zero_address(&env, &proposer)?;
        if is_paused(&env) {
            return Err(ContractError::ContractPaused);
        }
        let config = get_multisig_config(&env).ok_or(ContractError::MultiSigNotConfigured)?;
        require_multisig_admin(&config, &proposer)?;

        let action_id = next_multisig_action_id(&env)?;
        let action = MultiSigAction {
            id: action_id,
            action_type: action_type.clone(),
            proposal_id,
            new_config,
            approvals: 1,
            executed: false,
        };
        save_multisig_action(&env, &action);
        set_multisig_approval(&env, action_id, &proposer);
        events::multisig_action_proposed(&env, action_id, &proposer, &action_type);
        events::multisig_action_approved(&env, action_id, &proposer, 1, config.threshold);

        // Auto-execute if threshold is 1.
        if config.threshold == 1 {
            execute_multisig_action(&env, action_id, &action_type, action.proposal_id, &action.new_config)?;
        }

        Ok(action_id)
    }

    /// Approves a pending multi-sig action.
    ///
    /// When the approval count reaches the configured threshold the action is
    /// executed immediately within the same transaction.
    ///
    /// # Errors
    /// - [`ContractError::MultiSigNotConfigured`] if multi-sig has not been set up.
    /// - [`ContractError::NotMultiSigAdmin`] if `approver` is not in the admin list.
    /// - [`ContractError::ActionNotFound`] if `action_id` does not exist.
    /// - [`ContractError::ActionAlreadyExecuted`] if the action was already executed.
    /// - [`ContractError::AlreadyApproved`] if `approver` has already approved this action.
    pub fn approve_multisig_action(
        env: Env,
        approver: Address,
        action_id: u64,
    ) -> Result<(), ContractError> {
        approver.require_auth();
        require_non_zero_address(&env, &approver)?;
        if is_paused(&env) {
            return Err(ContractError::ContractPaused);
        }
        let config = get_multisig_config(&env).ok_or(ContractError::MultiSigNotConfigured)?;
        require_multisig_admin(&config, &approver)?;

        let mut action = load_multisig_action(&env, action_id)?;
        if action.executed {
            return Err(ContractError::ActionAlreadyExecuted);
        }
        if has_multisig_approval(&env, action_id, &approver) {
            return Err(ContractError::AlreadyApproved);
        }

        action.approvals = action.approvals.checked_add(1).ok_or(ContractError::VoteTallyOverflow)?;
        set_multisig_approval(&env, action_id, &approver);
        save_multisig_action(&env, &action);
        events::multisig_action_approved(&env, action_id, &approver, action.approvals, config.threshold);

        if action.approvals >= config.threshold {
            execute_multisig_action(&env, action_id, &action.action_type, action.proposal_id, &action.new_config)?;
        }

        Ok(())
    }

    /// Returns the current multi-sig configuration, or `None` if not configured.
    pub fn get_multisig_config(env: Env) -> Option<MultiSigConfig> {
        get_multisig_config(&env)
    }
}
