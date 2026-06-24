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

use soroban_sdk::{contracterror, contracttype, Address, String, Vec};

/// All revert conditions for the governance contract.
#[contracterror]
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(u32)]
pub enum ContractError {
    /// 1 – Admin address is not set
    AdminNotSet = 1,
    /// 2 – Caller is not the admin
    NotAdmin = 2,
    /// 3 – Voting token address is not set
    VotingTokenNotSet = 3,
    /// 4 – Quorum must be greater than zero
    InvalidQuorum = 4,
    /// 5 – Duration must be greater than zero
    InvalidDuration = 5,
    /// 6 – Proposal with the given ID does not exist
    ProposalNotFound = 6,
    /// 7 – Proposal is not in Active status
    ProposalNotActive = 7,
    /// 8 – Voting period has already ended
    VotingPeriodEnded = 8,
    /// 9 – Voting period has not ended yet
    VotingStillOpen = 9,
    /// 10 – Voter has already cast a vote on this proposal
    AlreadyVoted = 10,
    /// 11 – Voter has no token balance (no voting power)
    NoVotingPower = 11,
    /// 12 – Proposal has not passed
    ProposalNotPassed = 12,
    /// 13 – Contract has already been initialized
    AlreadyInitialized = 13,
    /// 14 – Vote tally arithmetic overflow
    VoteTallyOverflow = 14,
    /// 15 – Proposer has insufficient token balance to create a proposal
    InsufficientBalance = 15,
    /// 16 – Proposer must wait for the cooldown period to expire
    ProposalCooldown = 16,
    /// 17 – Proposal title exceeds maximum byte length
    TitleTooLong = 17,
    /// 18 – Proposal description exceeds maximum byte length
    DescriptionTooLong = 18,
    /// 19 – Proposal title is empty or exceeds maximum byte length
    InvalidTitle = 19,
    /// 20 – Proposal description is empty or exceeds maximum byte length
    InvalidDescription = 20,
    /// 21 – Duration is outside the allowed [MIN_DURATION, MAX_DURATION] range
    InvalidDurationRange = 21,
    /// 22 – Quorum exceeds the total token supply
    QuorumExceedsSupply = 22,
    /// 23 – Voting period has not yet started
    VotingNotStarted = 23,
    /// 24 – New admin address is invalid (e.g. zero address)
    InvalidNewAdmin = 24,
    /// 25 – Admin is not permitted to vote on their own proposals
    AdminVoteRestricted = 25,
    /// 26 – Contract is paused; state-changing operations are blocked
    ContractPaused = 26,
    /// 27 – Contract is not paused
    NotPaused = 27,
    /// 28 – Address parameter is the zero/default address
    InvalidAddress = 28,
    /// 29 – Proposal ID counter overflowed (u64::MAX reached)
    ProposalCountOverflow = 29,
    /// 30 – Timelock period has not yet expired
    TimelockNotExpired = 30,
    /// 31 – No pending admin transfer has been proposed
    PendingAdminNotSet = 31,
    /// 32 – The admin transfer window has expired; propose again
    AdminTransferExpired = 32,
    /// 33 – Caller is not the pending admin
    NotPendingAdmin = 33,
    /// 34 – Target version is lower than or equal to the current version (downgrade rejected)
    DowngradeNotAllowed = 34,
    /// 35 – Proposal amendment is not allowed after the amendment window or once voting has started
    ProposalAmendmentNotAllowed = 35,
    /// 36 – Only the original proposer may amend the proposal
    NotProposalOwner = 36,
    /// 37 – Caller is not a multi-sig admin
    NotMultiSigAdmin = 37,
    /// 38 – Multi-sig action with the given ID does not exist
    ActionNotFound = 38,
    /// 39 – Multi-sig configuration has not been set
    MultiSigNotConfigured = 39,
    /// 40 – Admin list cannot be empty
    EmptyAdminList = 40,
    /// 41 – Invalid threshold value
    InvalidThreshold = 41,
}

/// Lifecycle state of the governance contract itself.
///
/// - `Uninitialized`: the contract has been deployed but `initialize` has not
///   yet been called. No governance operations are possible.
/// - `Ready`: `initialize` completed successfully. The contract is fully
///   operational.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum ContractState {
    Uninitialized,
    Ready,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum ProposalState {
    Active,
    Passed,
    Rejected,
    Executed,
    Cancelled,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum Vote {
    Yes,
    No,
    Abstain,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Translation {
    pub title: String,
    pub description: String,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Proposal {
    pub id: u64,
    pub proposer: Address,
    pub title: String,
    pub description: String,
    pub votes_yes: i128,
    pub votes_no: i128,
    pub votes_abstain: i128,
    pub quorum: i128, // minimum total votes required to pass
    pub start_time: u64,
    pub end_time: u64,
    pub state: ProposalState,
    /// Earliest Unix timestamp at which the proposal may be executed.
    /// Set to `end_time + timelock_duration` when the proposal passes; 0 otherwise.
    pub execute_after: u64,
    /// Optional category tags (max 5, each max 32 chars).
    pub tags: Vec<String>,
}

/// Storage key enum for the governance contract.
///
/// Every storage entry is keyed by a variant of this enum.  Because Soroban
/// serialises the variant discriminant as part of the XDR key, each variant
/// occupies a completely separate key space — two variants with the same
/// payload can never collide.
///
/// ## Key-space map (SEC-006 collision analysis)
///
/// | Variant                          | Storage tier | Description                                         |
/// |----------------------------------|--------------|-----------------------------------------------------|
/// | `Proposal(u64)`                  | Persistent   | Full proposal struct, keyed by proposal ID          |
/// | `ProposalCount`                  | Instance     | Monotonic counter used to assign proposal IDs       |
/// | `HasVoted(u64, Address)`         | Persistent   | Boolean flag: has this voter voted on this proposal |
/// | `VoteRecord(u64, Address)`       | Persistent   | Detailed vote record (type + weight) per voter      |
/// | `VoterSnapshot(u64, Address)`    | Persistent   | Token-balance snapshot captured at vote time        |
/// | `LastProposal(Address)`          | Persistent   | Timestamp of a proposer's most recent proposal      |
/// | `Admin`                          | Instance     | Contract administrator address                      |
/// | `VotingToken`                    | Instance     | Governance token contract address                   |
/// | `MinProposalBalance`             | Instance     | Minimum token balance required to create a proposal |
/// | `ProposalCooldown`               | Instance     | Seconds a proposer must wait between proposals      |
/// | `ContractState`                  | Instance     | Lifecycle state (Uninitialized / Ready)             |
/// | `RestrictAdminVote`              | Instance     | Whether admin vote on own proposals is blocked      |
/// | `Paused`                         | Instance     | Whether the contract is currently paused            |
/// | `Version`                        | Instance     | Semver tuple `(major, minor, patch)`                |
///
/// ## Collision safety
///
/// Soroban serialises each `DataKey` variant by encoding the enum discriminant
/// **before** any payload into the XDR key.  This means:
///
/// - `HasVoted(id, addr)`, `VoteRecord(id, addr)`, and `VoterSnapshot(id, addr)`
///   share the same payload shape `(u64, Address)` but have distinct discriminants,
///   so they can never alias each other regardless of the argument values.
/// - Singleton variants (`Admin`, `VotingToken`, `ProposalCount`, …) have no
///   payload, so their keys are fixed and globally unique within this contract.
/// - No two distinct variants can produce the same serialised key because the
///   discriminant is always the first element of the encoding.
#[contracttype]
pub enum DataKey {
    /// Full [`Proposal`] struct, keyed by proposal ID (persistent storage).
    /// Key space: one entry per unique `u64` proposal ID.
    Proposal(u64),

    /// Monotonic counter used to derive the next proposal ID (instance storage).
    /// Key space: singleton — only one `ProposalCount` entry exists.
    ProposalCount,

    /// Boolean flag recording whether `voter` has voted on `proposal_id` (persistent storage).
    /// Key space: one entry per `(proposal_id, voter)` pair.
    /// Kept separate from `VoteRecord` so existence checks are cheap.
    HasVoted(u64, Address),

    /// Detailed vote record (vote type + weight) for `voter` on `proposal_id` (persistent storage).
    /// Key space: one entry per `(proposal_id, voter)` pair.
    VoteRecord(u64, Address),

    /// Contract administrator address (instance storage).
    /// Key space: singleton — only one `Admin` entry exists.
    Admin,

    /// Address of the governance token contract (instance storage).
    /// Key space: singleton — only one `VotingToken` entry exists.
    VotingToken,

    /// Minimum token balance a proposer must hold to create a proposal (instance storage).
    /// Key space: singleton — only one `MinProposalBalance` entry exists.
    MinProposalBalance,

    /// Minimum seconds a proposer must wait between consecutive proposals (instance storage).
    /// Key space: singleton — only one `ProposalCooldown` entry exists.
    ProposalCooldown,

    /// Lifecycle state of the governance contract (instance storage).
    /// Key space: singleton — only one `ContractState` entry exists.
    ContractState,

    /// Whether admin is restricted from voting on their own proposals (instance storage).
    /// Key space: singleton — only one `RestrictAdminVote` entry exists.
    RestrictAdminVote,

    /// Whether the contract is currently paused (instance storage).
    /// Key space: singleton — only one `Paused` entry exists.
    Paused,

    /// Optional reason string explaining why the contract was paused (instance storage).
    /// Key space: singleton — only one `PauseReason` entry exists.
    PauseReason,

    /// Timestamp (Unix seconds) of `proposer`'s most recent proposal (persistent storage).
    /// Key space: one entry per unique proposer address.
    LastProposal(Address),

    /// Contract version stored as a `(major, minor, patch)` semver tuple (instance storage).
    /// Key space: singleton — only one `Version` entry exists.
    Version,

    /// Token-balance snapshot for `voter` on `proposal_id`, captured at vote time (persistent storage).
    /// Key space: one entry per `(proposal_id, voter)` pair.
    /// Kept separate from `VoteRecord` to allow independent querying of vote weight.
    VoterSnapshot(u64, Address),

    /// Mandatory delay (seconds) between a proposal passing and it becoming executable (instance storage).
    /// Key space: singleton — only one `TimelockDuration` entry exists.
    TimelockDuration,

    /// Minimum allowed voting duration in seconds (instance storage).
    /// Key space: singleton — only one `MinDuration` entry exists.
    MinDuration,

    /// Maximum allowed voting duration in seconds (instance storage).
    /// Key space: singleton — only one `MaxDuration` entry exists.
    MaxDuration,

    /// Absolute vote weight threshold that rejects a proposal immediately when
    /// `votes_no >= veto_threshold`. Stored as instance storage.
    /// Key space: singleton — only one `VetoThreshold` entry exists.
    VetoThreshold,

    /// Address nominated to become the next admin (instance storage).
    /// Set by `propose_admin_transfer`; cleared on acceptance or expiry.
    PendingAdmin,

    /// Unix timestamp after which the pending admin nomination expires (instance storage).
    AdminTransferExpiry,

    /// Amendment window in seconds before voting begins.
    /// Key space: singleton — only one `AmendWindow` entry exists.
    AmendWindow,

    /// TTL bump amount for persistent storage entries (measured in ledgers).
    /// Controls how many ledgers into the future the TTL is extended on write operations.
    /// Key space: singleton — only one `PersistentStorageTTL` entry exists.
    PersistentStorageTTL,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct VoteRecord {
    pub vote_type: Vote,
    pub weight: i128,
}

/// Full contract configuration returned by [`get_config`].
#[contracttype]
#[derive(Clone, Debug)]
pub struct GovernanceConfig {
    pub voting_token: Address,
    pub min_proposal_balance: i128,
    pub proposal_cooldown: u64,
    pub min_duration: u64,
    pub max_duration: u64,
    pub restrict_admin_vote: bool,
    pub timelock_duration: u64,
    pub paused: bool,
    pub version: (u32, u32, u32),
    pub persistent_storage_ttl: u32,
}

/// Multi-signature admin configuration.
/// Specifies the list of admin addresses and the threshold required for actions.
#[contracttype]
#[derive(Clone, Debug)]
pub struct MultiSigConfig {
    pub admins: Vec<Address>,
    pub threshold: u32,
}

/// Action type for multi-sig operations.
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MultiSigActionType {
    ExecuteProposal,
    CancelProposal,
    UpdateMultiSig,
    Pause,
    Unpause,
}

/// Multi-signature action awaiting approval.
#[contracttype]
#[derive(Clone, Debug)]
pub struct MultiSigAction {
    pub id: u64,
    pub action_type: MultiSigActionType,
    pub proposal_id: u64,
    pub new_config: Option<MultiSigConfig>,
    pub executed: bool,
}
