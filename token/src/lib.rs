#![cfg_attr(not(feature = "std"), no_std)]
#![feature(min_specialization)]

/// This is a simple `PSP22` which will be used as a stable coin and a collateral token in our lending contract
#[openbrush::contract]
pub mod token {
    use openbrush::{
        contracts::psp22::extensions::{
            metadata::*,
            mintable::*,
        },
        traits::{
            Storage,
            String,
        },
    };

    /// Define the storage for PSP22 data and Metadata data
    #[ink(storage)]
    #[derive(Default, Storage)]
    pub struct TokenContract {
        #[storage_field]
        psp22: psp22::Data,
        #[storage_field]
        metadata: metadata::Data,
        #[storage_field]
        staking_contract: AccountId,
    }

    /// Implement PSP22 Trait for our coin
    impl PSP22 for Token {}

    /// Implement PSP22Metadata Trait for our coin
    impl PSP22Metadata for TokenContract {}

    /// implement PSP22Mintable Trait for our coin
    impl PSP22Mintable for TokenContract {}

    // It forces the compiler to check that you implemented all super traits
    impl StableCoin for TokenContract {}

    impl TokenContract {
        /// Constructor with name and symbol
        #[ink(constructor)]
        pub fn new(name: Option<String>, symbol: Option<String>,staking_contract: AccountId) -> Self {
            let mut instance = Self::default();

            instance.metadata.name = name;
            instance.metadata.symbol = symbol;
            instance.metadata.decimals = 18;
            let total_supply = 1_000_000 * 10_u128.pow(18);//1 billion
            assert!(instance._mint_to(Self::env().caller(), initial_supply * 30 / 100).is_ok());
            assert!(instance._mint_to(staking_contract, initial_supply * 70 / 100).is_ok());

            instance
        }
    }
}