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

use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};
use crate::{GovernanceContract, GovernanceContractClient};
use crate::types::Vote;
use crate::{GovernanceContract, GovernanceContractClient};
use soroban_sdk::{testutils::Address as _, Address, Env, String};

/// Returned by [`setup_env`] — holds every handle needed by a test.
pub struct TestEnv {
    pub env: Env,
    pub client: GovernanceContractClient<'static>,
    pub admin: Address,
    pub token_id: Address,
}

/// Initialise a test environment with both contracts deployed and the
/// governance contract initialized against the token.
///
/// The token is minted with 1_000_000 units assigned to `admin`.
pub fn setup_env() -> TestEnv {
    let env = Env::default();
    env.mock_all_auths();

    let gov_id = env.register(GovernanceContract, ());
    let client = GovernanceContractClient::new(&env, &gov_id);

    let admin = Address::generate(&env);

    let tok_id = env.register(votechain_token::TokenContract, ());
    let tok = votechain_token::TokenContractClient::new(&env, &tok_id);
    tok.initialize(&admin, &10_000_000);

    client.initialize(&admin, &tok_id, &0_i128, &0_u64, &60_u64, &2_592_000_u64, &false, &0_u64, &0_u64);

    TestEnv {
        env,
        client,
        admin,
        token_id: tok_id,
    }
}

/// Create a proposal with sensible defaults (quorum = 100, duration = 3600 s).
///
/// Returns the new proposal ID.
pub fn create_test_proposal(t: &TestEnv, proposer: &Address) -> u64 {
    t.client.create_proposal(
        proposer,
        &String::from_str(&t.env, "Test proposal"),
        &String::from_str(&t.env, "Test description"),
        &100,
        &3600,
        &Vec::new(&t.env),
    )
}

/// Mint `amount` tokens to `voter` and cast `vote` on `proposal_id`.
pub fn mint_and_vote(t: &TestEnv, voter: &Address, proposal_id: u64, vote: Vote, amount: i128) {
    let tok = votechain_token::TokenContractClient::new(&t.env, &t.token_id);
    tok.mint(&t.admin, voter, &amount);
    t.client.cast_vote(voter, &proposal_id, &vote);
}
