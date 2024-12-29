mod asset;
mod ema;
mod legacy;
mod owner;
mod upgrade;
mod utils;

pub use crate::asset::*;
pub use crate::ema::*;
use crate::legacy::*;
pub use crate::utils::*;

use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::collections::UnorderedMap;
use near_sdk::serde::{Deserialize, Serialize};
use near_sdk::{
    assert_one_yocto, env, ext_contract, log, near_bindgen, AccountId, Balance, BorshStorageKey,
    Gas, PanicOnDefault, Promise, Timestamp, 
};

const NO_DEPOSIT: Balance = 0;

const GAS_FOR_PROMISE: Gas = Gas(Gas::ONE_TERA.0 * 10);

pub type DurationSec = u32;

#[derive(BorshSerialize, BorshStorageKey)]
enum StorageKey {
    Assets,
}

#[near_bindgen]
#[derive(BorshDeserialize, BorshSerialize, PanicOnDefault)]
pub struct Contract {
    pub assets: UnorderedMap<AssetId, VAsset>,

    pub recency_duration_sec: DurationSec,

    pub owner_id: AccountId,
}

#[derive(Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct PriceData {
    #[serde(with = "u64_dec_format")]
    pub timestamp: Timestamp,
    pub recency_duration_sec: DurationSec,

    pub prices: Vec<AssetOptionalPrice>,
}

#[ext_contract(ext_price_receiver)]
pub trait ExtPriceReceiver {
    fn oracle_on_call(&mut self, sender_id: AccountId, data: PriceData, msg: String);
}

#[near_bindgen]
impl Contract {
    #[init]
    pub fn new(
        recency_duration_sec: DurationSec,
        owner_id: AccountId,
    ) -> Self {
        Self {
            assets: UnorderedMap::new(StorageKey::Assets),
            recency_duration_sec,
            owner_id,
        }
    }

    pub fn get_assets(&self, from_index: Option<u64>, limit: Option<u64>) -> Vec<(AssetId, Asset)> {
        unordered_map_pagination(&self.assets, from_index, limit)
    }

    pub fn get_asset(&self, asset_id: AssetId) -> Option<Asset> {
        self.internal_get_asset(&asset_id)
    }

    pub fn get_price_data(&self, asset_ids: Option<Vec<AssetId>>) -> PriceData {
        let asset_ids = asset_ids.unwrap_or_else(|| self.assets.keys().collect());
        let timestamp = env::block_timestamp();

        PriceData {
            timestamp,
            recency_duration_sec: self.recency_duration_sec,
            prices: asset_ids
                .into_iter()
                .map(|asset_id| {
                    // EMA for a specific asset, e.g. wrap.near#3600 is 1 hour EMA for wrap.near
                    if let Some((base_asset_id, _)) = asset_id.split_once('#') {
                        let asset = self.internal_get_asset(&base_asset_id.to_string());
                        AssetOptionalPrice {
                            asset_id,
                            price: asset.and_then(|asset| {
                                asset.median_price()
                            }),
                        }
                    } else {
                        let asset = self.internal_get_asset(&asset_id);
                        AssetOptionalPrice {
                            asset_id,
                            price: asset.and_then(|asset| {
                                asset.median_price()
                            }),
                        }
                    }
                })
                .collect(),
        }
    }

    pub fn report_prices(&mut self, prices: Vec<AssetPrice>) {
        assert!(!prices.is_empty());
        let oracle_id = env::predecessor_account_id();
        let timestamp = env::block_timestamp();

        // Updating prices
        for AssetPrice { asset_id, price } in prices {
            price.assert_valid();

            if self.internal_get_asset(&asset_id).is_none() {
                self.internal_set_asset(&asset_id, Asset::new());
            }
            
            if let Some(mut asset) = self.internal_get_asset(&asset_id) {
                asset.add_report(Report {
                    oracle_id: oracle_id.clone(),
                    timestamp,
                    price,
                });
                self.internal_set_asset(&asset_id, asset);
            } else {
                log!("Warning! Unknown asset ID: {}", asset_id);
            }
        }
    }

    #[payable]
    pub fn oracle_call(
        &mut self,
        receiver_id: AccountId,
        asset_ids: Option<Vec<AssetId>>,
        msg: String,
    ) -> Promise {
        self.assert_well_paid();

        let sender_id = env::predecessor_account_id();
        let price_data = self.get_price_data(asset_ids);
        let remaining_gas = env::prepaid_gas() - env::used_gas();
        assert!(remaining_gas >= GAS_FOR_PROMISE);

        ext_price_receiver::oracle_on_call(
            sender_id,
            price_data,
            msg,
            receiver_id,
            NO_DEPOSIT,
            remaining_gas - GAS_FOR_PROMISE,
        )
    }
}

impl Contract {
    pub fn assert_well_paid(&self) {
        assert_one_yocto();
    }
}

pub trait OraclePriceReceiver {
    fn oracle_on_call(&mut self, sender_id: AccountId, data: PriceData, msg: String);
}

#[near_bindgen]
impl OraclePriceReceiver for Contract {
    /// The method will execute a given list of actions in the msg using the prices from the `data`
    /// provided by the oracle on behalf of the sender_id.
    /// - Requires to be called by the oracle account ID.
    fn oracle_on_call(&mut self, sender_id: AccountId, data: PriceData, msg: String) {
        let mut prices: Vec<AssetPrice> = vec![];
        for price_data in data.prices {
            if price_data.price.is_some() {
                prices.push(AssetPrice {
                    asset_id: price_data.asset_id.clone(),
                    price: price_data.price.unwrap(),
                })
            }
        }
        if prices.len() > 0 {
            log!("Account {} triggers a {}", sender_id, msg);
            self.report_prices(prices);
        }
        
    }
}
