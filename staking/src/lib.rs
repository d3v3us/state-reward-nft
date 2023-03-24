#![cfg_attr(not(feature = "std"), no_std)]
#![feature(min_specialization)]

#[openbrush::contract]
pub mod staking_pool_contract {

    use ink_prelude::{collections::BTreeMap, vec::Vec};
    use openbrush::contracts::traits::psp22::PSP22Ref;

    use openbrush::{
        contract::{contract, Contract},
        contracts::{
            ownable::*,
            psp37::extensions::{burnable::*, mintable::*},
        },
        modifiers,
        prelude::*,
        traits::Storage,
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
        last_update: u64,
        halving_period: u64,
        start_time: u64,
        reputation: BTreeMap<AccountId, u128>,
    }

    impl StakingContract {
        fn on_stake(&mut self, account: AccountId, amount: u128) {
            self.balances
                .entry(account)
                .and_modify(|balance| *balance += amount)
                .or_insert(amount);
            self.total_staked += amount;
            self.last_update = Self::env().block_timestamp();
        }
        fn on_unstake(&mut self, account: AccountId, amount: u128) {
            self.balances
                .entry(account)
                .and_modify(|balance| *balance -= amount)
                .expect("account is unstaking more than they have staked");
            self.total_staked -= amount;
            self.last_update = Self::env().block_timestamp();
        }
        pub fn calculate_reputation(&self, account: &AccountId) -> u128 {
            let balance = self.balances.get(account).copied().unwrap_or(0);
            let staking_duration = self.last_update - self.start_time;
            let days_staked = staking_duration / (24 * 60 * 60);
            balance * days_staked
        }

        pub fn claim_reputation(&mut self, account: AccountId) {
            let reputation = self.calculate_reputation(&account);
            self.reputation
                .entry(account)
                .and_modify(|balance| *balance += reputation)
                .or_insert(reputation);
        }

        pub fn reputation_of(&self, owner: AccountId) -> u128 {
            *self.reputation.get(&owner).unwrap_or(&0)
        }
    }

    #[ink(storage)]
    pub struct StakingPool {
        staking_contract: StakingContract,
        psp37: psp37::Data,
    }

    impl Contract for StakingPool {}
    impl PSP37 for StakingPool {}

    impl PSP37Mintable for StakingPool {
        #[ink(message)]
        fn mint(
            &mut self,
            to: AccountId,
            ids_amounts: Vec<(Id, Balance)>,
        ) -> Result<(), PSP37Error> {
            self._mint_to(to, ids_amounts)
        }
    }

    impl StakingPool {
        #[ink(constructor)]
        pub fn new(
            reward_token: AccountId,
            staked_token: AccountId,
            halving_period: u64,
            start_time: u64,
        ) -> Self {
            Self {
                staking_contract: StakingContract {
                    reward_token,
                    staked_token,
                    staking_deadline: Self::env().block_timestamp() + 365 * 24 * 60 * 60, // 1 year
                    halving_period,
                    start_time,
                    ..Default::default()
                },
            }
        }

        #[ink(message)]
        pub fn stake(&mut self, amount: u128) -> bool {
            let account = Self::caller();
            let result =
                self.transfer_from(account, self.env().account_id(), self.staked_token, amount)?;
            if result {
                let staking_contract = &mut self.staking_contract;
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
           
            staking_contract.on_unstake(account, amount);
            self.transfer_from(account, self.env().account_id(), self.staked_token, amount)?;

            true
        }

        #[ink(message)]
        pub fn distribute_tokens(&mut self) {
            let account = Self::caller();
            let now = Self::env().block_timestamp();
            let days_passed = (now - self.start_time) / (24 * 60 * 60);
            let halvings_passed = days_passed / self.staking_contract.halving_period;

            let mut percentage = 50;

            for _ in 0..halvings_passed {
                percentage /= 2;
            }

            let unlocked = (self.get_total_staked() * percentage as u128 / 100) as Balance;

            self.transfer_from(account, self.env().account_id(), self.staked_token, unlocked)?;
            self.claim_reputation();
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
        //reputation
        #[ink(message)]
        pub fn claim_reputation(&mut self) {
            let account = Self::caller();
            self.staking_contract.claim_reputation(account);
            self.mint_tokens()
        }

         #[ink(message)]
        pub fn mint_tokens(&mut self amount: Balance) -> Result<(), PSP37Error> {
            let id="generate_id";
            self.mint(Self::env().caller(), vec![(id, amount)])
        }

        #[ink(message)]
        pub fn reputation_of(&self, owner: AccountId) -> u128 {
            self.staking_contract.reputation_of(owner)
        }
    }
}
