# TTL Bump Implementation - Quick Reference

## ✅ All Acceptance Criteria Met

| Criterion | Status | Implementation |
|-----------|--------|-----------------|
| TTL bumped on write to proposal storage | ✅ DONE | `bump_ttl_*()` functions called on `save_proposal()`, `mark_voted()`, `save_vote_record()`, etc. |
| TTL bump amount configurable | ✅ DONE | `get_ttl_bump_ledgers()` / `set_ttl_bump_ledgers()` in storage.rs |
| Tests verify entries survive expected ledger count | ✅ DONE | 6 test functions added to test.rs covering all scenarios |
| No unnecessary TTL bumps on read-only calls | ✅ DONE | Read functions unchanged, only write functions call TTL bump |

## What Was Implemented

### Core Components

#### 1. Configuration Storage (storage.rs)
- **Default TTL**: 518,400 ledgers (~60 days)
- **Retrieval**: `get_ttl_bump_ledgers(env)` 
- **Configuration**: `set_ttl_bump_ledgers(env, new_amount)`
- **Storage key**: `DataKey::TTLBumpLedgers` (instance storage)

#### 2. TTL Bump Functions (storage.rs)
Seven dedicated functions for bumping different entry types:
- `bump_ttl_proposal(proposal_id)`
- `bump_ttl_has_voted(proposal_id, voter)`
- `bump_ttl_vote_record(proposal_id, voter)`
- `bump_ttl_voter_snapshot(proposal_id, voter)`
- `bump_ttl_last_proposal(proposer)`
- `bump_ttl_multisig_action(action_id)`
- `bump_ttl_multisig_approval(action_id, approver)`

#### 3. Automatic TTL Bumping on Writes (storage.rs)
Eight write functions now call TTL bump:
- `save_proposal()` ➜ `bump_ttl_proposal()`
- `mark_voted()` ➜ `bump_ttl_has_voted()`
- `save_vote_record()` ➜ `bump_ttl_vote_record()`
- `save_voter_snapshot()` ➜ `bump_ttl_voter_snapshot()`
- `set_last_proposal()` ➜ `bump_ttl_last_proposal()`
- `save_multisig_action()` ➜ `bump_ttl_multisig_action()`
- `set_multisig_approval()` ➜ `bump_ttl_multisig_approval()`

#### 4. Comprehensive Tests (test.rs)
Six new test functions:
1. `test_ttl_bump_configuration()` - Configuration works
2. `test_ttl_bump_on_proposal_write()` - Proposals persist
3. `test_ttl_bump_on_vote_writes()` - Votes persist
4. `test_no_ttl_bump_on_read_only()` - Reads don't bump
5. `test_proposal_survives_expected_ledgers()` - Survives ledger advancement
6. `test_ttl_bump_on_last_proposal()` - Proposer records persist

#### 5. Documentation
- `TTL_BUMP_CONFIGURATION.md` - Deployment guide
- `TTL_BUMP_IMPLEMENTATION.md` - Technical summary
- This file - Quick reference

## Protected Entries

All persistent storage for proposals and voting is protected:

```
Proposal data          → Proposal(id)
Vote deduplication     → HasVoted(proposal_id, voter)
Vote audit trail       → VoteRecord(proposal_id, voter)
Voting weight snapshot → VoterSnapshot(proposal_id, voter)
Proposer cooldown      → LastProposal(proposer)
Multi-sig actions      → MultiSigAction(id)
Multi-sig approvals    → MultiSigApproval(action_id, approver)
```

## TTL Bump Strategy

### Algorithm
```
when writing to persistent storage:
  threshold = configured_ttl_bump_ledgers / 2
  bump_amount = configured_ttl_bump_ledgers
  extend_ttl(key, threshold, bump_amount)
```

### Rationale
- **Conservative**: Waits until 50% expired before extending (safety margin)
- **Lazy**: Only bumps when needed, not on every operation
- **Efficient**: Minimizes unnecessary storage operations

## Default vs. Custom TTL

### Default (Production Ready)
- **Amount**: 518,400 ledgers
- **Duration**: ~60 days (at 10 sec/ledger)
- **Use case**: Standard DAOs with typical voting cycles

### Customization Examples
```rust
// Fast DAOs with quick voting
set_ttl_bump_ledgers(&env, 86_400);    // 10 days

// Standard DAOs  
set_ttl_bump_ledgers(&env, 518_400);   // 60 days (default)

// Enterprise with long voting
set_ttl_bump_ledgers(&env, 1_036_800); // 120 days

// Maximum protection
set_ttl_bump_ledgers(&env, 2_073_600); // 240 days
```

## Deployment Checklist

- [ ] Review TTL_BUMP_CONFIGURATION.md
- [ ] Determine optimal TTL for your DAO
- [ ] Initialize contract with admin
- [ ] (Optional) Call `set_ttl_bump_ledgers()` with custom value
- [ ] Monitor logs for TTL extension events
- [ ] Add TTL bump amount to deployment documentation

## Files Changed

```
contracts/governance/src/
├── types.rs              [+] DataKey variants, MultiSig types, error codes
├── storage.rs            [+] TTL functions, updated write functions
├── lib.rs                [+] Updated imports
└── test.rs               [+] 6 new test functions

docs/
└── TTL_BUMP_CONFIGURATION.md  [NEW] Deployment guide

project root/
└── TTL_BUMP_IMPLEMENTATION.md [NEW] Technical summary
```

## Key Design Decisions

1. ✅ **Write-only bumping** - Read operations don't trigger extensions (per requirement)
2. ✅ **Configurable TTL** - Admins can customize per deployment
3. ✅ **Conservative threshold** - Extends at 50% to prevent expiry
4. ✅ **Lazy evaluation** - Only bumps when entry is accessed for write
5. ✅ **Modular functions** - Separate function per key type
6. ✅ **Default is safe** - 60-day TTL handles most DAOs

## Testing & Verification

Run tests:
```bash
cargo test test_ttl_bump
```

Expected results:
- ✅ Configuration test passes
- ✅ Write test passes  
- ✅ Vote test passes
- ✅ No-bump-on-read test passes
- ✅ Survival test passes
- ✅ LastProposal test passes

## Performance Characteristics

| Operation | Impact | Notes |
|-----------|--------|-------|
| Create Proposal | +1 TTL extension | One `extend_ttl()` call |
| Cast Vote | +3 TTL extensions | HasVoted + VoteRecord + VoterSnapshot |
| Read Proposal | No TTL change | `load_proposal()` doesn't bump |
| Check Voted | No TTL change | `has_voted()` doesn't bump |
| Get Vote Record | No TTL change | `get_vote_record()` doesn't bump |

## Troubleshooting

### Problem: Entries expiring
**Solution**: Increase TTL bump amount
```rust
set_ttl_bump_ledgers(&env, 1_036_800); // Increase to 120 days
```

### Problem: High storage costs
**Solution**: Reduce TTL bump amount
```rust
set_ttl_bump_ledgers(&env, 259_200); // Decrease to 30 days
```

### Problem: Need to know current TTL
**Solution**: Query storage
```rust
let current = get_ttl_bump_ledgers(&env);
println!("TTL: {} ledgers", current);
```

## Related Documentation

- [TTL_BUMP_CONFIGURATION.md](docs/TTL_BUMP_CONFIGURATION.md) - Full deployment guide
- [TTL_BUMP_IMPLEMENTATION.md](TTL_BUMP_IMPLEMENTATION.md) - Technical details
- [contracts/governance/src/storage.rs](contracts/governance/src/storage.rs) - Implementation

## Support

For questions or issues:
1. Check TTL_BUMP_CONFIGURATION.md troubleshooting section
2. Review test.rs for usage examples
3. Examine storage.rs for implementation details
