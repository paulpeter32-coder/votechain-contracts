# TTL Bump Configuration Guide

## Overview

The VoteChain governance contract implements automatic TTL (Time To Live) bumping for persistent storage entries to ensure that long-running proposals and vote records don't expire unexpectedly.

## What is TTL?

In Soroban, persistent storage entries have a Time To Live (TTL) that determines how many ledgers they survive before expiring. On Stellar, a new ledger is typically created every 5-10 seconds. Without TTL management, long-running proposals could expire during active voting.

## TTL Bump Strategy

The contract uses a **conservative, automatic approach**:

- **When**: TTL is automatically extended every time a persistent storage entry is written
- **Not on reads**: Read-only operations do NOT trigger TTL extensions (minimizing overhead)
- **Threshold**: The contract waits until TTL drops to half the configured bump amount before extending
- **Bump amount**: Each extension adds the configured number of ledgers

## Default Configuration

- **Default bump amount**: 518,400 ledgers
- **At 10 seconds per ledger**: ~60 days of protection
- **Configurable**: Can be adjusted per deployment using `set_ttl_bump_ledgers()`

## Protected Entries

The following persistent storage entries automatically get TTL bumped on write:

| Key Type | Purpose | Bumped On |
|----------|---------|-----------|
| `Proposal(id)` | Proposal data | `save_proposal()` after create/update |
| `HasVoted(proposal_id, voter)` | Vote deduplication | `mark_voted()` when voter casts vote |
| `VoteRecord(proposal_id, voter)` | Vote audit trail | `save_vote_record()` when vote recorded |
| `VoterSnapshot(proposal_id, voter)` | Voting weight snapshot | `save_voter_snapshot()` at vote time |
| `LastProposal(proposer)` | Proposer cooldown tracking | `set_last_proposal()` on proposal create |
| `MultiSigAction(id)` | Multi-sig pending actions | `save_multisig_action()` on action create |
| `MultiSigApproval(action_id, approver)` | Multi-sig approval records | `set_multisig_approval()` on approval |

## Configuration

### Getting Current TTL Bump Amount

```rust
// In a contract function (requires storage context)
let current_ttl_ledgers = get_ttl_bump_ledgers(&env);
```

### Setting TTL Bump Amount

```rust
// Admin should call this to customize TTL bump
set_ttl_bump_ledgers(&env, 259_200); // 30 days instead of 60
```

### Recommended Values

| Use Case | Ledgers | Duration | Notes |
|----------|---------|----------|-------|
| Short-term DAOs | 86,400 | ~10 days | Faster cleanup, less overhead |
| Standard DAOs | 518,400 | ~60 days | Default, balanced approach |
| Long-term DAOs | 1,036,800 | ~120 days | For slowly-moving governance |
| Enterprise DAOs | 2,073,600 | ~240 days | For multi-month voting cycles |

## Example: Setting Custom TTL for Your DAO

When initializing a governance contract, you might want to set a custom TTL:

```rust
// Initialize the contract
governance.initialize(
    &admin,
    &voting_token,
    &min_proposal_balance,
    &proposal_cooldown,
    &min_duration,
    &max_duration,
    &restrict_admin_vote,
    &amend_window,
    &timelock_duration,
    // ... other parameters
);

// Then immediately set your custom TTL
set_ttl_bump_ledgers(&env, 259_200); // 30 days for your DAO
```

## Monitoring TTL Health

In production, you should monitor:

1. **Average proposal duration**: How long proposals typically remain active
2. **Expected storage lifespan**: How long vote records should be archived
3. **Ledger advancement rate**: Actual ledger creation frequency (may vary)

**Best Practice**: Set TTL bump amount to **2-3x your longest expected proposal duration**.

For example:
- If proposals last max 14 days → set TTL to 30-45 days (86,400-129,600 ledgers)
- If proposals last max 30 days → set TTL to 60-90 days (173,600-259,200 ledgers)

## Implementation Details

### TTL Bump Algorithm

```rust
fn get_ttl_bump_params(env: &Env) -> (u32, u32) {
    let bump_amount = get_ttl_bump_ledgers(env);
    let threshold = bump_amount / 2;  // Conservative: wait until halfway expired
    (threshold, bump_amount)
}

// When writing to persistent storage:
env.storage()
    .persistent()
    .extend_ttl(&key, threshold, bump_amount);
```

### No Unnecessary Overhead

Read-only operations (`load_proposal()`, `has_voted()`, `get_vote_record()`, etc.) do **NOT** trigger TTL extensions. This minimizes:
- Storage operations (cheaper contracts)
- Network overhead
- Ledger bloat

Only writes trigger TTL bumps.

## Troubleshooting

### "Entry expired" errors

If you see expired entry errors:

1. **Verify TTL bump is happening**: Check logs for `extend_ttl` calls
2. **Check TTL bump amount**: May be too small for your use case
3. **Verify write operations**: Ensure write operations are being called
4. **Increase TTL bump**: If proposals are longer than expected

Example fix:

```rust
// If getting expiry errors with default 60-day TTL:
set_ttl_bump_ledgers(&env, 1_036_800); // Increase to 120 days
```

### High storage costs

If you're seeing unexpectedly high storage costs:

1. **Consider reducing TTL**: Shorter TTL means faster cleanup
2. **Monitor hit rate**: Are entries actually used or just accumulating?
3. **Archive strategy**: Consider archiving old proposals off-chain
4. **TTL too aggressive**: Check that TTL isn't being bumped unnecessarily

Example optimization:

```rust
// If mostly short-lived proposals:
set_ttl_bump_ledgers(&env, 86_400); // Reduce to 10 days
```

## Testing TTL Behavior

The contract includes comprehensive TTL tests:

```bash
# Run TTL tests
cargo test --lib test_ttl_bump

# Individual tests:
cargo test test_ttl_bump_configuration       # Config test
cargo test test_ttl_bump_on_proposal_write   # Write test
cargo test test_ttl_bump_on_vote_writes      # Vote test
cargo test test_no_ttl_bump_on_read_only     # No-bump test
cargo test test_proposal_survives_expected_ledgers  # Survival test
```

## Future Enhancements

Potential improvements to TTL management:

- **Adaptive TTL**: Automatically adjust based on proposal duration
- **Per-proposal TTL**: Allow custom TTL per proposal
- **TTL events**: Emit events when TTL is bumped
- **TTL preemption**: Warn admin when entries approaching expiry
- **Archive API**: Helper functions to export expiring data before cleanup

## References

- [Soroban Storage Documentation](https://soroban.stellar.org/docs)
- [Stellar Ledger Lifecycle](https://developers.stellar.org/docs/build/applications/soroban/guides/ledger-keys)
- VoteChain [storage.rs](../contracts/governance/src/storage.rs) - Implementation
