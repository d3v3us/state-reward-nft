#![cfg_attr(not(feature = "std"), no_std)]
#![feature(min_specialization)]

#[openbrush::contract]
pub mod staking_pool_contract {

    use ink_prelude::{collections::BTreeMap, vec::Vec};
    use openbrush::contracts::traits::psp22::PSP22Ref;
    use openbrush::{
        contract::{contract, Contract},
        prelude::*,
        traits::{OnStake, OnUnstake},
    };

    #[derive(Debug, PartialEq, Eq, scale::Encode, scale::Decode)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum Error {
        NotAdmin,
        AmountShouldBeGreaterThanZero,
        InsufficientFunds,
        NotEnoughAllowance,
        TokenTransferFailed,
        Overflow,
        StakingStillInProgress,
    }

    #[derive(Default)]
    pub struct StakingContract {
        balances: BTreeMap<AccountId, u128>,
        staked_token: AccountId,
        reward_token: AccountId,
        total_staked: u128,
        staking_deadline: u64,
        rewards: Vec<u128>,
        reward_per_token: u128,
        last_update: u64,
    }

    impl OnStake<OpenbrushPSP22> for StakingContract {
        fn on_stake(&mut self, account: AccountId, amount: u128) {
            self.rewards.push(self.get_reward_per_token());
            self.balances
                .entry(account)
                .and_modify(|balance| *balance += amount)
                .or_insert(amount);
            self.total_staked += amount;
            self.last_update = Self::env().block_timestamp();
        }
    }

    impl OnUnstake<OpenbrushPSP22> for StakingContract {
        fn on_unstake(&mut self, account: AccountId, amount: u128) {
            self.rewards.push(self.get_reward_per_token());
            self.balances
                .entry(account)
                .and_modify(|balance| *balance -= amount)
                .expect("account is unstaking more than they have staked");
            self.total_staked -= amount;
            self.last_update = Self::env().block_timestamp();
        }
    }

    #[contract(env = "OpenbrushPSP22")]
    pub struct StakingPool {
        staking_contract: StakingContract,
    }

    impl Contract for StakingPool {}

    impl StakingPool {
        #[ink(constructor)]
        pub fn new(reward_token: AccountId, staked_token: AccountId) -> Self {
            Self {
                staking_contract: StakingContract {
                    reward_token,
                    staked_token,
                    staking_deadline: Self::env().block_timestamp() + 365 * 24 * 60 * 60, // 1 year
                    rewards: vec![],
                    reward_per_token: 0,
                    ..Default::default()
                },
            }
        }

        #[ink(message)]
        pub fn stake(&mut self, amount: u128) -> bool {
            let account = Self::caller();
            let result=self.transfer_from(account, self.env().account_id(), self.staked_token, amount)?;
            if result {
                let staking_contract = &mut self.staking_contract;
                let remaining_days = (staking_contract.staking_deadline
                    - Self::env().block_timestamp())
                    / (24 * 60 * 60);
                let reward = amount * staking_contract.rewards.last().copied().unwrap_or(0)
                    / 10_000
                    / remaining_days as u128;
                staking_contract.reward_per_token +=
                    reward * 10_000 / staking_contract.total_staked;
                staking_contract.on_stake(account, amount);
                true
            } else {
                false
            }
        }

        #[ink(message)]
        pub fn unstake(&mut self, amount: u128) -> bool {
            let account = Self::caller();
            let staking_contract = &mut self.staking_contract;
            let balance = staking_contract
                .balances
                .get(&account)
                .copied()
                .unwrap_or(0);
            if balance < amount {
                return false;
            }
            let remaining_days = (staking_contract.staking_deadline
                - Self::env().block_timestamp())
                / (24 * 60 * 60);
            let reward = amount * staking_contract.rewards.last().copied().unwrap_or(0)
                / 10_000
                / remaining_days as u128;
            staking_contract.reward_per_token += reward * 10_000 / staking_contract.total_staked;
            staking_contract.on_unstake(account, amount);
            self.transfer_from(account, self.env().account_id(), self.staked_token, amount)?;

            true
        }

        #[ink(message)]
        pub fn claim_rewards(&mut self) -> bool {
            let account = Self::caller();
            let staking_contract = &mut self.staking_contract;
            let balance = staking_contract
                .balances
                .get(&account)
                .copied()
                .unwrap_or(0);
            let reward_per_token = staking_contract.get_reward_per_token();
            let earned = balance * reward_per_token / 10_000
                - staking_contract.rewards.last().copied().unwrap_or(0);
            if earned == 0 {
                return false;
            }
            staking_contract.rewards.push(earned);
            self.transfer_from(account, self.env().account_id(), self.reward_token, earned)?;
            true
        }

        #[ink(message)]
        pub fn get_reward_per_token(&self) -> u128 {
            let staking_contract = &self.staking_contract;
            if staking_contract.total_staked == 0 {
                return staking_contract.reward_per_token;
            }
            let reward = staking_contract.rewards.last().copied().unwrap_or(0);
            let remaining_days = (staking_contract.staking_deadline
                - Self::env().block_timestamp())
                / (24 * 60 * 60);
            let new_reward = reward / 2;
            if remaining_days == 0 {
                return staking_contract.reward_per_token
                    + new_reward * 10_000 / staking_contract.total_staked;
            }
            let reward_per_day = new_reward / remaining_days as u128;
            staking_contract.reward_per_token
                + reward_per_day * 10_000 / staking_contract.total_staked
        }

        #[ink(message)]
        pub fn get_total_staked(&self) -> u128 {
            self.staking_contract.total_staked
        }

        #[ink(message)]
        pub fn get_staking_deadline(&self) -> u64 {
            self.staking_contract.staking_deadline
        }

        fn transfer(&self, to: AccountId, token: AccountId, amount: Balance) -> Result<(), Error> {
            PSP22Ref::transfer(&token, to, amount, vec![]).unwrap_or_else(|error| {
                panic!("Failed to transfer PSP22 2 tokens to caller : {:?}", error)
            });

            Ok(())
        }

        fn transfer_from(
            &self,
            from: AccountId,
            to: AccountId,
            token: AccountId,
            amount: Balance,
        ) -> Result<(), Error> {
            // checking the balance of the sender to see if the sender has enough balance to run this transfer
            let user_current_balance = PSP22Ref::balance_of(&token, from);

            if user_current_balance < amount {
                return Err(Error::InsufficientFunds);
            }

            // checking if enough allowance has been made for this operation
            let staking_contract_allowance = PSP22Ref::allowance(&token, from, to);

            if staking_contract_allowance < amount {
                return Err(Error::NotEnoughAllowance);
            }

            let staking_contract_initial_balance = PSP22Ref::balance_of(&token, to);

            // making the transfer call to the token contract
            if PSP22Ref::transfer_from_builder(&token, from, to, amount, vec![])
                .call_flags(CallFlags::default().set_allow_reentry(true))
                .try_invoke()
                .expect("Transfer failed")
                .is_err()
            {
                return Err(Error::TokenTransferFailed);
            }

            let staking_contract_balance_after_transfer = PSP22Ref::balance_of(&token, to);

            let mut actual_token_staked: Balance = 0;

            // calculating the actual amount that came in to the contract, some token might have taxes, just confirming transfer for economic safety
            match staking_contract_balance_after_transfer
                .checked_sub(staking_contract_initial_balance)
            {
                Some(result) => {
                    actual_token_staked = result;
                }
                None => {
                    return Err(Error::Overflow);
                }
            };

            Ok(())
        }
    }
}
