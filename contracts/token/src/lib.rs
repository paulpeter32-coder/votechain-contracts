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
mod test;

use soroban_sdk::{contract, contractimpl, Address, Env};
use storage::*;
use types::ContractError;

// SEC-004: Stellar null/zero address used as the sentinel for invalid inputs.
const ZERO_ADDRESS: &str = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF";

// SEC-004: Rejects the Stellar zero/default address on any address parameter.
fn require_non_zero_address(env: &Env, addr: &Address) -> Result<(), ContractError> {
    if *addr == Address::from_str(env, ZERO_ADDRESS) {
        return Err(ContractError::InvalidAddress);
    }
    Ok(())
}

#[contract]
pub struct TokenContract;

#[contractimpl]
impl TokenContract {
    /// Initialises the token contract, minting the entire initial supply to the admin.
    ///
    /// Must be called once before any other function.
    ///
    /// # Parameters
    /// - `env` – Soroban execution environment.
    /// - `admin` – Address that receives the initial supply and gains admin privileges.
    /// - `initial_supply` – Total tokens minted to `admin` at initialisation.
    ///
    /// # Errors
    /// - [`ContractError::InvalidAddress`] if `admin` is the zero address.
    pub fn initialize(env: Env, admin: Address, initial_supply: i128) -> Result<(), ContractError> {
        // SEC-005: auth first.
        admin.require_auth();
        // SEC-004: reject zero address before any state change.
        require_non_zero_address(&env, &admin)?;
        set_admin(&env, &admin);
        set_balance(&env, &admin, initial_supply);
        set_total_supply(&env, initial_supply);
        set_version(&env, (1, 0, 0));
        Ok(())
    }

    /// Returns the total token supply in circulation.
    ///
    /// # Parameters
    /// - `env` – Soroban execution environment.
    ///
    /// # Returns
    /// Total supply as `i128`.
    pub fn total_supply(env: Env) -> i128 {
        total_supply(&env)
    }

    /// Returns the token balance of an address.
    ///
    /// # Parameters
    /// - `env` – Soroban execution environment.
    /// - `owner` – Address to query.
    ///
    /// # Returns
    /// Balance as `i128`. Returns `0` if the address has never held tokens.
    pub fn balance(env: Env, owner: Address) -> i128 {
        balance_of(&env, &owner)
    }

    /// Returns the token balance of an address (alias for [`balance`]).
    ///
    /// # Parameters
    /// - `env` – Soroban execution environment.
    /// - `owner` – Address to query.
    ///
    /// # Returns
    /// Balance as `i128`. Returns `0` if the address has never held tokens.
    pub fn balance_of(env: Env, owner: Address) -> i128 {
        balance_of(&env, &owner)
    }

    /// Transfers tokens from one address to another.
    ///
    /// # Parameters
    /// - `env` – Soroban execution environment.
    /// - `from` – Sender address; must authorise the call.
    /// - `to` – Recipient address.
    /// - `amount` – Number of tokens to transfer; must be positive.
    ///
    /// # Errors
    /// - [`ContractError::InvalidAddress`] if `from` or `to` is the zero address.
    /// - [`ContractError::InvalidAmount`] if `amount` is zero or negative.
    /// - [`ContractError::InsufficientBalance`] if `from` has fewer tokens than `amount`.
    pub fn transfer(
        env: Env,
        from: Address,
        to: Address,
        amount: i128,
    ) -> Result<(), ContractError> {
        // SEC-005: auth first.
        from.require_auth();
        // SEC-004: reject zero addresses.
        require_non_zero_address(&env, &from)?;
        require_non_zero_address(&env, &to)?;
        if amount <= 0 {
            return Err(ContractError::InvalidAmount);
        }
        // Transfer to self is a no-op: auth is still required but no state changes occur.
        if from == to { return Ok(()); }
        if is_frozen(&env, &from) || is_frozen(&env, &to) { return Err(ContractError::AccountFrozen); }
        let b = balance_of(&env, &from);
        if b < amount {
            return Err(ContractError::InsufficientBalance);
        }
        set_balance(&env, &from, b - amount);
        set_balance(&env, &to, balance_of(&env, &to) + amount);
        events::transferred(&env, &from, &to, amount);
        Ok(())
    }

    /// Approves `spender` to transfer up to `amount` tokens on behalf of `owner`.
    ///
    /// Overwrites any existing allowance. Stored in temporary storage (expires with the ledger).
    ///
    /// # Parameters
    /// - `env` – Soroban execution environment.
    /// - `owner` – Token owner granting the allowance; must authorise the call.
    /// - `spender` – Address permitted to spend on behalf of `owner`.
    /// - `amount` – Maximum tokens the spender may transfer.
    ///
    /// # Errors
    /// - [`ContractError::InvalidAddress`] if `owner` or `spender` is the zero address.
    pub fn approve(
        env: Env,
        owner: Address,
        spender: Address,
        amount: i128,
    ) -> Result<(), ContractError> {
        // SEC-005: auth first.
        owner.require_auth();
        // SEC-004: reject zero addresses.
        require_non_zero_address(&env, &owner)?;
        require_non_zero_address(&env, &spender)?;
        set_allowance(&env, &owner, &spender, amount);
        Ok(())
    }

    /// Transfers tokens on behalf of `from` using a pre-approved allowance.
    ///
    /// # Parameters
    /// - `env` – Soroban execution environment.
    /// - `spender` – Address spending the allowance; must authorise the call.
    /// - `from` – Token owner whose balance is debited.
    /// - `to` – Recipient address.
    /// - `amount` – Number of tokens to transfer; must not exceed the allowance.
    ///
    /// # Errors
    /// - [`ContractError::InvalidAddress`] if `spender`, `from`, or `to` is the zero address.
    /// - [`ContractError::AllowanceExceeded`] if `amount` exceeds the current allowance.
    /// - [`ContractError::InsufficientBalance`] if `from` has fewer tokens than `amount`.
    pub fn transfer_from(
        env: Env,
        spender: Address,
        from: Address,
        to: Address,
        amount: i128,
    ) -> Result<(), ContractError> {
        // SEC-005: auth first.
        spender.require_auth();
        // SEC-004: reject zero addresses.
        require_non_zero_address(&env, &spender)?;
        require_non_zero_address(&env, &from)?;
        require_non_zero_address(&env, &to)?;
        let allowed = allowance(&env, &from, &spender);
        if allowed < amount { return Err(ContractError::AllowanceExceeded); }
        if is_frozen(&env, &from) || is_frozen(&env, &to) { return Err(ContractError::AccountFrozen); }
        let b = balance_of(&env, &from);
        if b < amount {
            return Err(ContractError::InsufficientBalance);
        }
        set_allowance(&env, &from, &spender, allowed - amount);
        set_balance(&env, &from, b - amount);
        set_balance(&env, &to, balance_of(&env, &to) + amount);
        events::transferred(&env, &from, &to, amount);
        Ok(())
    }

    /// Mints new tokens to an address, increasing the total supply.
    ///
    /// Only the admin may mint tokens.
    ///
    /// # Parameters
    /// - `env` – Soroban execution environment.
    /// - `admin` – Admin address; must authorise the call.
    /// - `to` – Address that receives the newly minted tokens.
    /// - `amount` – Number of tokens to mint; must be positive.
    ///
    /// # Errors
    /// - [`ContractError::InvalidAddress`] if `admin` or `to` is the zero address.
    /// - [`ContractError::NotAdmin`] if `admin` does not match the stored admin.
    /// - [`ContractError::InvalidAmount`] if `amount` is zero or negative.
    pub fn mint(env: Env, admin: Address, to: Address, amount: i128) -> Result<(), ContractError> {
        // SEC-005: auth first.
        admin.require_auth();
        // SEC-004: reject zero addresses.
        require_non_zero_address(&env, &admin)?;
        require_non_zero_address(&env, &to)?;
        if get_admin(&env)? != admin { return Err(ContractError::NotAdmin); }
        if amount <= 0 { return Err(ContractError::InvalidAmount); }
        if is_frozen(&env, &to) { return Err(ContractError::AccountFrozen); }
        set_balance(&env, &to, balance_of(&env, &to) + amount);
        set_total_supply(&env, total_supply(&env) + amount);
        events::minted(&env, &to, amount);
        Ok(())
    }

    /// Burns tokens from an address, reducing the total supply.
    ///
    /// Only the admin may burn tokens.
    ///
    /// # Parameters
    /// - `env` – Soroban execution environment.
    /// - `admin` – Admin address; must authorise the call.
    /// - `from` – Address whose tokens are burned.
    /// - `amount` – Number of tokens to burn.
    ///
    /// # Errors
    /// - [`ContractError::InvalidAddress`] if `admin` or `from` is the zero address.
    /// - [`ContractError::NotAdmin`] if `admin` does not match the stored admin.
    /// - [`ContractError::InsufficientBalance`] if `from` has fewer tokens than `amount`.
    pub fn burn(
        env: Env,
        admin: Address,
        from: Address,
        amount: i128,
    ) -> Result<(), ContractError> {
        // SEC-005: auth first.
        admin.require_auth();
        // SEC-004: reject zero addresses.
        require_non_zero_address(&env, &admin)?;
        require_non_zero_address(&env, &from)?;
        if get_admin(&env)? != admin {
            return Err(ContractError::NotAdmin);
        }
        let b = balance_of(&env, &from);
        if b < amount { return Err(ContractError::InsufficientBalance); }
        if is_frozen(&env, &from) { return Err(ContractError::AccountFrozen); }
        set_balance(&env, &from, b - amount);
        set_total_supply(&env, total_supply(&env) - amount);
        events::burned(&env, &from, amount);
        Ok(())
    }

    /// Freezes `target`, preventing it from sending or receiving tokens.
    ///
    /// Only the admin may freeze addresses. Frozen addresses can still vote (balance is readable).
    ///
    /// # Errors
    /// - [`ContractError::NotAdmin`] if `admin` is not the stored admin.
    pub fn freeze(env: Env, admin: Address, target: Address) -> Result<(), ContractError> {
        admin.require_auth();
        require_non_zero_address(&env, &target)?;
        if get_admin(&env)? != admin { return Err(ContractError::NotAdmin); }
        set_frozen(&env, &target, true);
        events::frozen(&env, &target);
        Ok(())
    }

    /// Unfreezes `target`, restoring its ability to send and receive tokens.
    ///
    /// Only the admin may unfreeze addresses.
    ///
    /// # Errors
    /// - [`ContractError::NotAdmin`] if `admin` is not the stored admin.
    pub fn unfreeze(env: Env, admin: Address, target: Address) -> Result<(), ContractError> {
        admin.require_auth();
        require_non_zero_address(&env, &target)?;
        if get_admin(&env)? != admin { return Err(ContractError::NotAdmin); }
        set_frozen(&env, &target, false);
        events::unfrozen(&env, &target);
        Ok(())
    }

    /// Returns `true` if `target` is currently frozen.
    pub fn is_frozen(env: Env, target: Address) -> bool {
        is_frozen(&env, &target)
    }

    /// Transfers admin rights to a new address. Only the current admin may call this.
    ///
    /// The old admin loses all privileges immediately upon successful transfer.
    ///
    /// # Parameters
    /// - `env` – Soroban execution environment.
    /// - `admin` – Current admin address; must authorise the call.
    /// - `new_admin` – Address that will become the new admin.
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
        if get_admin(&env)? != admin {
            return Err(ContractError::NotAdmin);
        }
        set_admin(&env, &new_admin);
        events::admin_transferred(&env, &admin, &new_admin);
        Ok(())
    }

    /// Returns the contract version as a `(major, minor, patch)` semver tuple.
    pub fn get_version(env: Env) -> (u32, u32, u32) {
        get_version(&env)
    }
}
