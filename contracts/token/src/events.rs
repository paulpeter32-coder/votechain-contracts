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

use soroban_sdk::{symbol_short, Address, Env};

pub fn minted(env: &Env, to: &Address, amount: i128) {
    env.events()
        .publish((symbol_short!("mint"), to.clone()), amount);
}

pub fn transferred(env: &Env, from: &Address, to: &Address, amount: i128) {
    env.events().publish(
        (symbol_short!("transfer"), from.clone(), to.clone()),
        amount,
    );
}

pub fn burned(env: &Env, from: &Address, amount: i128) {
    env.events()
        .publish((symbol_short!("burn"), from.clone()), amount);
}

/// Emits an `admxfer` event when admin rights are transferred.
///
/// Topics: `("admxfer",)`  
/// Data: `(old_admin: Address, new_admin: Address)`
pub fn admin_transferred(env: &Env, old_admin: &Address, new_admin: &Address) {
    env.events().publish(
        (symbol_short!("admxfer"),),
        (old_admin.clone(), new_admin.clone()),
    );
}

pub fn frozen(env: &Env, addr: &Address) {
    env.events().publish((symbol_short!("freeze"), addr.clone()), ());
}

pub fn unfrozen(env: &Env, addr: &Address) {
    env.events().publish((symbol_short!("unfreeze"), addr.clone()), ());
}
