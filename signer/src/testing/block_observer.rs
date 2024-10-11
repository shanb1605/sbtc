//! Test utilities for the block observer

use std::collections::HashMap;

use crate::bitcoin::BitcoinInteract;
use crate::stacks::api::StacksInteract;
use bitcoin::BlockHash;
use bitcoin::Txid;
use blockstack_lib::chainstate::burn::ConsensusHash;
use blockstack_lib::chainstate::nakamoto::NakamotoBlock;
use blockstack_lib::chainstate::nakamoto::NakamotoBlockHeader;
use blockstack_lib::chainstate::stacks::StacksTransaction;
use blockstack_lib::net::api::getinfo::RPCPeerInfoData;
use blockstack_lib::net::api::getpoxinfo::RPCPoxEpoch;
use blockstack_lib::net::api::getpoxinfo::RPCPoxInfoData;
use blockstack_lib::net::api::gettenureinfo::RPCGetTenureInfo;
use blockstack_lib::types::chainstate::StacksAddress;
use blockstack_lib::types::chainstate::StacksBlockId;
use clarity::vm::costs::ExecutionCost;
use rand::seq::IteratorRandom;
use sbtc::deposits::CreateDepositRequest;

use crate::bitcoin::rpc::BitcoinTxInfo;
use crate::bitcoin::rpc::GetTxResponse;
use crate::bitcoin::utxo;
use crate::emily_client::EmilyInteract;
use crate::error::Error;
use crate::keys::PublicKey;
use crate::stacks::api::AccountInfo;
use crate::stacks::api::FeePriority;
use crate::stacks::api::SubmitTxResponse;
use crate::testing::dummy;
use crate::util::ApiFallbackClient;

/// A test harness for the block observer.
#[derive(Debug, Clone)]
pub struct TestHarness {
    bitcoin_blocks: Vec<bitcoin::Block>,
    /// This represents the Stacks blockchain. The bitcoin::BlockHash
    /// is used to identify tenures. That is, all NakamotoBlocks that
    /// have the same bitcoin::BlockHash occur within the same tenure.
    stacks_blocks: Vec<(StacksBlockId, NakamotoBlock, BlockHash)>,
    /// This represents deposit transactions
    deposits: HashMap<Txid, GetTxResponse>,
    /// This represents deposit requests that have not been processed, i.e.
    /// they are received from the Emily API.
    pending_deposits: Vec<CreateDepositRequest>,
}

impl TestHarness {
    /// Get the Bitcoin blocks in the test harness.
    pub fn bitcoin_blocks(&self) -> &[bitcoin::Block] {
        &self.bitcoin_blocks
    }

    /// Get the Stacks blocks in the test harness.
    pub fn stacks_blocks(&self) -> &[(StacksBlockId, NakamotoBlock, BlockHash)] {
        &self.stacks_blocks
    }

    /// Get the deposit transactions in the test harness.
    pub fn deposits(&self) -> &HashMap<Txid, GetTxResponse> {
        &self.deposits
    }

    /// Add a single deposit transaction to the test harness.
    pub fn add_deposit(&mut self, txid: Txid, response: GetTxResponse) {
        self.deposits.insert(txid, response);
    }

    /// Add multiple deposit transactions to the test harness.
    pub fn add_deposits(&mut self, deposits: &[(Txid, GetTxResponse)]) {
        for (txid, response) in deposits {
            self.add_deposit(*txid, response.clone());
        }
    }

    /// Get the pending deposit requests in the test harness.
    pub fn pending_deposits(&self) -> &[CreateDepositRequest] {
        &self.pending_deposits
    }

    /// Add a single pending deposit request to the test harness.
    pub fn add_pending_deposit(&mut self, deposit: CreateDepositRequest) {
        self.pending_deposits.push(deposit);
    }

    /// Add multiple pending deposit requests to the test harness.
    pub fn add_pending_deposits(&mut self, deposits: &[CreateDepositRequest]) {
        self.pending_deposits.extend(deposits.iter().cloned());
    }

    /// Generate a new test harness with random data.
    pub fn generate(
        rng: &mut impl rand::RngCore,
        num_bitcoin_blocks: usize,
        num_stacks_blocks_per_bitcoin_block: std::ops::Range<usize>,
    ) -> Self {
        let mut bitcoin_blocks: Vec<_> = std::iter::repeat_with(|| dummy::block(&fake::Faker, rng))
            .take(num_bitcoin_blocks)
            .collect();

        for idx in 1..bitcoin_blocks.len() {
            bitcoin_blocks[idx].header.prev_blockhash = bitcoin_blocks[idx - 1].block_hash();
        }

        let first_header = NakamotoBlockHeader::empty();
        let stacks_blocks: Vec<(StacksBlockId, NakamotoBlock, BlockHash)> = bitcoin_blocks
            .iter()
            .scan(first_header, |previous_stx_block_header, btc_block| {
                let num_blocks = num_stacks_blocks_per_bitcoin_block
                    .clone()
                    .choose(rng)
                    .unwrap_or_default();
                let initial_state = previous_stx_block_header.clone();
                let stacks_blocks: Vec<(StacksBlockId, NakamotoBlock, BlockHash)> =
                    std::iter::repeat_with(|| dummy::stacks_block(&fake::Faker, rng))
                        .take(num_blocks)
                        .scan(initial_state, |last_stx_block_header, mut stx_block| {
                            stx_block.header.parent_block_id = last_stx_block_header.block_id();
                            stx_block.header.chain_length = last_stx_block_header.chain_length + 1;
                            *last_stx_block_header = stx_block.header.clone();
                            Some((stx_block.block_id(), stx_block, btc_block.block_hash()))
                        })
                        .collect();

                if let Some((_, stx_block, _)) = stacks_blocks.last() {
                    *previous_stx_block_header = stx_block.header.clone()
                };

                Some(stacks_blocks)
            })
            .flatten()
            .collect();

        Self {
            bitcoin_blocks,
            stacks_blocks,
            deposits: HashMap::new(),
            pending_deposits: Vec::new(),
        }
    }

    /// Spawn a Bitcoin block hash stream for testing.
    pub fn spawn_block_hash_stream(
        &self,
    ) -> tokio_stream::wrappers::ReceiverStream<Result<bitcoin::BlockHash, Error>> {
        let headers: Vec<_> = self
            .bitcoin_blocks
            .iter()
            .map(|block| Ok(block.block_hash()))
            .collect();

        let (tx, rx) = tokio::sync::mpsc::channel(128);

        tokio::spawn(async move {
            for header in headers {
                tx.send(header).await.expect("failed to send header");
            }
        });

        rx.into()
    }
}

impl TryFrom<TestHarness> for ApiFallbackClient<TestHarness> {
    type Error = Error;
    fn try_from(value: TestHarness) -> Result<Self, Error> {
        ApiFallbackClient::new(vec![value]).map_err(Error::FallbackClient)
    }
}

impl BitcoinInteract for TestHarness {
    async fn get_tx(&self, txid: &bitcoin::Txid) -> Result<Option<GetTxResponse>, Error> {
        Ok(self.deposits.get(txid).cloned())
    }

    async fn get_tx_info(&self, _: &Txid, _: &BlockHash) -> Result<Option<BitcoinTxInfo>, Error> {
        unimplemented!()
    }

    async fn get_block(
        &self,
        block_hash: &bitcoin::BlockHash,
    ) -> Result<Option<bitcoin::Block>, Error> {
        Ok(self
            .bitcoin_blocks
            .iter()
            .find(|block| &block.block_hash() == block_hash)
            .cloned())
    }

    async fn estimate_fee_rate(&self) -> Result<f64, Error> {
        unimplemented!()
    }

    async fn get_last_fee(&self, _utxo: bitcoin::OutPoint) -> Result<Option<utxo::Fees>, Error> {
        unimplemented!()
    }

    async fn broadcast_transaction(&self, _tx: &bitcoin::Transaction) -> Result<(), Error> {
        unimplemented!()
    }
}

impl StacksInteract for TestHarness {
    async fn get_current_signer_set(
        &self,
        _contract_principal: &StacksAddress,
    ) -> Result<Vec<PublicKey>, Error> {
        // issue #118
        todo!()
    }
    async fn get_account(&self, _address: &StacksAddress) -> Result<AccountInfo, Error> {
        // issue #118
        todo!()
    }

    async fn submit_tx(&self, _tx: &StacksTransaction) -> Result<SubmitTxResponse, Error> {
        // issue #118
        todo!()
    }

    async fn get_block(&self, block_id: StacksBlockId) -> Result<NakamotoBlock, Error> {
        self.stacks_blocks
            .iter()
            .skip_while(|(id, _, _)| &block_id != id)
            .map(|(_, block, _)| block)
            .next()
            .cloned()
            .ok_or(Error::MissingBlock)
    }
    async fn get_tenure(&self, block_id: StacksBlockId) -> Result<Vec<NakamotoBlock>, Error> {
        let (stx_block_id, stx_block, btc_block_id) = self
            .stacks_blocks
            .iter()
            .find(|(id, _, _)| &block_id == id)
            .ok_or(Error::MissingBlock)?;

        let blocks: Vec<NakamotoBlock> = self
            .stacks_blocks
            .iter()
            .skip_while(|(_, _, block_id)| block_id != btc_block_id)
            .take_while(|(block_id, _, _)| block_id != stx_block_id)
            .map(|(_, block, _)| block)
            .chain(std::iter::once(stx_block))
            .cloned()
            .collect();

        Ok(blocks)
    }
    async fn get_tenure_info(&self) -> Result<RPCGetTenureInfo, Error> {
        let (_, _, btc_block_id) = self.stacks_blocks.last().unwrap();

        Ok(RPCGetTenureInfo {
            consensus_hash: ConsensusHash([0; 20]),
            tenure_start_block_id: self
                .stacks_blocks
                .iter()
                .find(|(_, _, block_id)| block_id == btc_block_id)
                .map(|(stx_block_id, _, _)| *stx_block_id)
                .unwrap(),
            parent_consensus_hash: ConsensusHash([0; 20]),
            parent_tenure_start_block_id: StacksBlockId::first_mined(),
            tip_block_id: self
                .stacks_blocks
                .last()
                .map(|(block_id, _, _)| *block_id)
                .unwrap(),
            tip_height: self.stacks_blocks.len() as u64,
            reward_cycle: 0,
        })
    }

    async fn estimate_fees<T>(&self, _: &T, _: FeePriority) -> Result<u64, Error>
    where
        T: crate::stacks::contracts::AsTxPayload,
    {
        Ok(500_000)
    }

    async fn get_pox_info(&self) -> Result<RPCPoxInfoData, Error> {
        let nakamoto_start_height = self
            .stacks_blocks
            .first()
            .map(|(_, block, _)| block.header.chain_length)
            .unwrap_or_default();
        let data = get_pox_info_data();

        let result = RPCPoxInfoData {
            epochs: vec![RPCPoxEpoch {
                epoch_id: clarity::types::StacksEpochId::Epoch30,
                start_height: nakamoto_start_height,
                end_height: 9223372036854776000,
                network_epoch: 11,
                block_limit: ExecutionCost {
                    write_length: 15_000_000,
                    write_count: 15_000,
                    read_length: 100_000_000,
                    read_count: 15_000,
                    runtime: 5_000_000_000,
                },
            }],
            ..data
        };

        Ok(result)
    }

    async fn get_node_info(&self) -> Result<RPCPeerInfoData, Error> {
        let data = get_node_info_data();

        let result = RPCPeerInfoData {
            burn_block_height: self.bitcoin_blocks.len() as u64,
            stacks_tip_height: self.stacks_blocks.len() as u64,
            ..data
        };

        Ok(result)
    }
}

impl EmilyInteract for TestHarness {
    async fn get_deposits(&self) -> Result<Vec<CreateDepositRequest>, Error> {
        Ok(self.pending_deposits.clone())
    }
}

fn get_pox_info_data() -> RPCPoxInfoData {
    let raw_json_response = r#"
    {
        "contract_id": "ST000000000000000000002AMW42H.pox-4",
        "pox_activation_threshold_ustx": 700073322473389,
        "first_burnchain_block_height": 0,
        "current_burnchain_block_height": 1880,
        "prepare_phase_block_length": 5,
        "reward_phase_block_length": 15,
        "reward_slots": 30,
        "rejection_fraction": null,
        "total_liquid_supply_ustx": 70007332247338910,
        "current_cycle": {
            "id": 94,
            "min_threshold_ustx": 583400000000000,
            "stacked_ustx": 5250510000000000,
            "is_pox_active": true
        },
        "next_cycle": {
            "id": 95,
            "min_threshold_ustx": 583400000000000,
            "min_increment_ustx": 8750916530917,
            "stacked_ustx": 5250510000000000,
            "prepare_phase_start_block_height": 1895,
            "blocks_until_prepare_phase": 15,
            "reward_phase_start_block_height": 1900,
            "blocks_until_reward_phase": 20,
            "ustx_until_pox_rejection": null
        },
        "epochs": [],
        "min_amount_ustx": 583400000000000,
        "prepare_cycle_length": 5,
        "reward_cycle_id": 94,
        "reward_cycle_length": 20,
        "rejection_votes_left_required": null,
        "next_reward_cycle_in": 20,
        "contract_versions": []
    }"#;

    serde_json::from_str::<RPCPoxInfoData>(raw_json_response).unwrap()
}

fn get_node_info_data() -> RPCPeerInfoData {
    let raw_json_response = r#"
    {
        "peer_version": 4207599114,
        "pox_consensus": "daf212e6103309e3918de4b2bf39ae2399109d9a",
        "burn_block_height": 2083,
        "stable_pox_consensus": "11fc12900b1f1369098f1099bcb2708ea78ea3b4",
        "stable_burn_block_height": 2082,
        "server_version": "stacks-node 0.0.1 (:c87c0eb6c050688340b975b0b42fb0a1ae378afa, debug build, linux [x86_64])",
        "network_id": 2147483648,
        "parent_network_id": 3669344250,
        "stacks_tip_height": 9520,
        "stacks_tip": "ffd652ff665bb1b07b19e537a5a007d44ea1e8cd0ddfd8753d9f95f915aaee41",
        "stacks_tip_consensus_hash": "11fc12900b1f1369098f1099bcb2708ea78ea3b4",
        "genesis_chainstate_hash": "74237aa39aa50a83de11a4f53e9d3bb7d43461d1de9873f402e5453ae60bc59b",
        "unanchored_tip": null,
        "unanchored_seq": null,
        "exit_at_block_height": null,
        "is_fully_synced": true,
        "node_public_key": "035379aa40c02890d253cfa577964116eb5295570ae9f7287cbae5f2585f5b2c7c",
        "node_public_key_hash": "1dc27eba0247f8cc9575e7d45e50a0bc7e72427d",
        "affirmations": {
            "heaviest": "nnnnnnnnnn",
            "stacks_tip": "nnnnnnnnnnp",
            "sortition_tip": "nnnnnnnnnnp",
            "tentative_best": "nnnnnnnnnnp"
        },
        "last_pox_anchor": {
            "anchor_block_hash": "4f57cfdc7fe6cc7cfa5b7caa5791993cd01a9fa3162326d6cc74f34007ded99b",
            "anchor_block_txid": "f96a10160fbece3070aed33ffe6afeb3540fa6a13e0d0c4f88b43ee8ebb68f9d"
        },
        "stackerdbs": []
    }"#;

    serde_json::from_str::<RPCPeerInfoData>(raw_json_response).unwrap()
}