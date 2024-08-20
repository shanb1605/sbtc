//! Testing utilities.
//! TODO(283, TBD): Use openapi generated client instead of bespoke methods.

use std::collections::HashMap;

use emily_handler::{
    api::models::{
        chainstate::Chainstate,
        common::Status,
        deposit::{
            requests::{CreateDepositRequestBody, UpdateDepositsRequestBody},
            responses::{CreateDepositResponse, GetDepositsResponse, UpdateDepositsResponse},
            Deposit, DepositInfo,
        },
        withdrawal::{
            requests::{CreateWithdrawalRequestBody, UpdateWithdrawalsRequestBody},
            responses::{
                CreateWithdrawalResponse, GetWithdrawalsResponse, UpdateWithdrawalsResponse,
            },
            Withdrawal, WithdrawalId, WithdrawalInfo,
        },
    },
    context::EmilyContext,
};
use error::TestError;
use reqwest::{Client, RequestBuilder};
use serde::{Deserialize, Serialize};

/// Test constants module.
pub mod constants;
/// Test errors modules.
pub mod error;

use constants::{
    ALL_STATUSES, EMILY_CHAINSTATE_ENDPOINT, EMILY_DEPOSIT_ENDPOINT, EMILY_TESTING_ENDPOINT,
    EMILY_WITHDRAWAL_ENDPOINT,
};

pub fn assert_eq_pretty<T>(actual: T, expected: T)
where
    T: Serialize + std::fmt::Debug + Eq,
{
    // Assert both objects equal with a prettier output string.
    assert_eq!(
        actual,
        expected,
        "Actual:\n{}\nExpected:\n{}",
        serde_json::to_string_pretty(&actual).unwrap(),
        serde_json::to_string_pretty(&expected).unwrap()
    );
}

/// Makes a test chainstate with a standard block hash that indicates what
/// fork and height it is.
pub fn test_chainstate(height: u64, fork_id: u32) -> Chainstate {
    Chainstate {
        stacks_block_hash: format!("stacks-block-{height}-hash-fork-{fork_id}"),
        stacks_block_height: height,
    }
}

/// Creates a standard test deposit request for uniform creation.
pub fn test_create_deposit_request(id_num: u64, output_index: u32) -> CreateDepositRequestBody {
    CreateDepositRequestBody {
        bitcoin_txid: format!("deposit-txid-{id_num}"),
        bitcoin_tx_output_index: output_index,
        reclaim: format!("deposit-txid-{id_num}:{output_index}-reclaim-script"),
        deposit: format!("deposit-txid-{id_num}:{output_index}-deposit-script"),
    }
}

/// Make a test context for Emily.
pub async fn test_context() -> EmilyContext {
    EmilyContext::local_test_instance()
        .await
        .expect("Failed to setup local test context for Emily integration test.")
}

/// Test client to make API calls within integration tests. This takes the place of what
/// will eventually be an autogenerated OpenAPI client before the OpenAPI client is
/// properly generated.
///
/// The existance of this class is tech-debt.
/// TODO(394): Use autogenerated OpenAPI client in test infrastructure.
pub struct TestClient {
    pub inner: Client,
}

/// Test client implementation.
impl TestClient {
    /// Create the test client.
    pub fn new() -> Self {
        TestClient { inner: Client::new() }
    }

    /// Sets up the test environment.
    pub async fn setup_test(&self) {
        self.reset_environment().await;
    }

    /// Sets up the test environment.
    pub async fn teardown(&self) {
        self.reset_environment().await;
    }

    /// Reset test environment.
    pub async fn reset_environment(&self) {
        let endpoint: String = format!("{EMILY_TESTING_ENDPOINT}/wipe");
        self.inner
            .post(&endpoint)
            .send()
            .await
            .expect(&format!("Failed to perform wipe api call: [{endpoint}]"));
    }

    /// Create deposit.
    pub async fn create_deposit(
        &self,
        request: &CreateDepositRequestBody,
    ) -> CreateDepositResponse {
        create_xyz(&self.inner, EMILY_DEPOSIT_ENDPOINT, request)
            .await
            .unwrap()
    }

    /// Get a single deposit.
    pub async fn get_deposit(
        &self,
        bitcoin_txid: &String,
        bitcoin_tx_output_index: u32,
    ) -> Deposit {
        get_xyz::<Deposit>(
            &self.inner,
            format!("{EMILY_DEPOSIT_ENDPOINT}/{bitcoin_txid}/{bitcoin_tx_output_index}").as_str(),
        )
        .await
        .expect("Get deposit in test failed.")
    }

    /// Executes an update deposits request.
    pub async fn update_deposits(
        &self,
        request: &UpdateDepositsRequestBody,
    ) -> UpdateDepositsResponse {
        update_xyz(&self.inner, &EMILY_DEPOSIT_ENDPOINT, request)
            .await
            .expect("Update deposits in test failed.")
    }

    /// Create withdrawal.
    pub async fn create_withdrawal(
        &self,
        request: &CreateWithdrawalRequestBody,
    ) -> CreateWithdrawalResponse {
        create_xyz(&self.inner, EMILY_WITHDRAWAL_ENDPOINT, request)
            .await
            .unwrap()
    }

    /// Get a single withdrawal.
    pub async fn get_withdrawal(&self, request_id: &WithdrawalId) -> Withdrawal {
        get_xyz::<Withdrawal>(
            &self.inner,
            format!("{EMILY_WITHDRAWAL_ENDPOINT}/{request_id}").as_str(),
        )
        .await
        .expect("Get withdrawal in test failed.")
    }

    /// Executes an update withdrawals request.
    pub async fn update_withdrawals(
        &self,
        request: &UpdateWithdrawalsRequestBody,
    ) -> UpdateWithdrawalsResponse {
        update_xyz(&self.inner, &EMILY_WITHDRAWAL_ENDPOINT, request)
            .await
            .expect("Update withdrawals in test failed.")
    }

    /// Create chainstate.
    pub async fn create_chainstate(&self, request: &Chainstate) -> Chainstate {
        create_xyz(&self.inner, EMILY_CHAINSTATE_ENDPOINT, request)
            .await
            .unwrap()
    }

    /// Gets the chain tip.
    pub async fn get_chaintip(&self) -> Chainstate {
        get_xyz(&self.inner, &format!("{EMILY_CHAINSTATE_ENDPOINT}"))
            .await
            .unwrap()
    }

    /// Update chainstate.
    pub async fn update_chainstate(&self, request: &Chainstate) -> Chainstate {
        update_xyz(&self.inner, EMILY_CHAINSTATE_ENDPOINT, request)
            .await
            .unwrap()
    }

    /// Get all withdrawals.
    pub async fn get_all_withdrawals(&self) -> Vec<WithdrawalInfo> {
        let mut all_withdrawals: Vec<WithdrawalInfo> = Vec::new();
        for status in ALL_STATUSES {
            all_withdrawals.extend(
                self.get_all_withdrawals_with_status(status)
                    .await
                    .into_iter(),
            );
        }
        all_withdrawals
    }

    /// Gets all withdrawals with a specified status.
    pub async fn get_all_withdrawals_with_status(&self, status: &Status) -> Vec<WithdrawalInfo> {
        // Get all withdrawals with the given status.
        get_all_xyz_with_status::<GetWithdrawalsResponse, WithdrawalInfo>(
            &self.inner,
            EMILY_WITHDRAWAL_ENDPOINT,
            base_query_from_status(status),
            |response: &GetWithdrawalsResponse| response.next_token.clone(),
            |response: &GetWithdrawalsResponse| response.withdrawals.clone(),
        )
        .await
    }

    /// Get all deposits.
    pub async fn get_all_deposits(&self) -> Vec<DepositInfo> {
        let mut all_deposits: Vec<DepositInfo> = Vec::new();
        for status in ALL_STATUSES {
            all_deposits.extend(self.get_all_deposits_with_status(status).await.into_iter());
        }
        all_deposits
    }

    /// Gets all deposits with a specified status.
    pub async fn get_all_deposits_with_status(&self, status: &Status) -> Vec<DepositInfo> {
        // Get all deposits with the given status.
        get_all_xyz_with_status::<GetDepositsResponse, DepositInfo>(
            &self.inner,
            EMILY_DEPOSIT_ENDPOINT,
            base_query_from_status(status),
            |response: &GetDepositsResponse| response.next_token.clone(),
            |response: &GetDepositsResponse| response.deposits.clone(),
        )
        .await
    }
}

// Reqwest client wrapper functions.
// -----------------------------------------------------------------------------

/// Generic create function.
async fn create_xyz<T, R>(client: &Client, endpoint: &str, request: &T) -> Result<R, TestError>
where
    T: Serialize,
    R: for<'de> Deserialize<'de>,
{
    do_xyz(client.post(endpoint).json(request), endpoint).await
}

/// Generic update function.
async fn update_xyz<T, R>(client: &Client, endpoint: &str, request: &T) -> Result<R, TestError>
where
    T: Serialize,
    R: for<'de> Deserialize<'de>,
{
    do_xyz(client.put(endpoint).json(request), endpoint).await
}

/// Generic update function.
async fn get_xyz<R>(client: &Client, endpoint: &str) -> Result<R, TestError>
where
    R: for<'de> Deserialize<'de>,
{
    do_xyz(client.get(endpoint), endpoint).await
}

/// Generic function that handles building and launching a request.
async fn do_xyz<R>(request_builder: RequestBuilder, endpoint: &str) -> Result<R, TestError>
where
    R: for<'de> Deserialize<'de>,
{
    let response = request_builder
        .send()
        .await
        .map_err(|e| TestError::Request {
            endpoint: endpoint.to_string(),
            source: e,
        })?;

    let response_text = response.text().await.map_err(|e| TestError::Request {
        endpoint: endpoint.to_string(),
        source: e,
    })?;

    serde_json::from_str(&response_text).map_err(|e| TestError::Deserialization {
        endpoint: endpoint.to_string(),
        source: e,
        response_text,
    })
}

// Get Many
// -----------------------------------------------------------------------------

/// Generic get all function that will get all of the items from a specific API query
/// with a given status.
async fn get_all_xyz_with_status<R, I>(
    client: &Client,
    endpoint: &str,
    base_query: HashMap<String, String>,
    extract_token: fn(&R) -> Option<String>,
    extract_items: fn(&R) -> Vec<I>,
) -> Vec<I>
where
    R: for<'de> Deserialize<'de>,
{
    // Aggregate list to get accumulate items.
    let mut all_items: Vec<I> = Vec::new();
    // Make initial query.
    let mut response = client
        .get(endpoint)
        .query(&base_query.clone().into_iter().collect::<Vec<_>>())
        .send()
        .await
        .expect(&format!(
            "Failed to perform get many Emily API call: [{endpoint}, {base_query:?}]"
        ))
        .json()
        .await
        .expect(&format!(
            "Failed to deserialize response from get many Emily API call: [{endpoint}, {base_query:?}]"
        ));
    // Add items from latest response to accumulator list.
    all_items.extend(extract_items(&response).into_iter());
    // Loop until the `next_token` is null.
    while let Some(next_token) = extract_token(&response) {
        // Add next token to the query.
        let mut query = base_query.clone();
        query.insert("nextToken".to_string(), next_token.clone());
        response = client
            .get(endpoint)
            .query(&query.into_iter().collect::<Vec<_>>())
            .send()
            .await
            .expect(&format!(
                "Failed to perform get many Emily API call: [{endpoint}, {base_query:?}]"
            ))
            .json()
            .await
            .map_err(|error| {
                eprintln!("{:?}", error);
                error
            })
            .expect(&format!(
                "Failed to deserialize response from get many Emily API call: [{endpoint}, {base_query:?}]"
            ));
        // Add items from latest response to accumulator list.
        all_items.extend(extract_items(&response).into_iter());
    }
    all_items
}

/// Creates a base query from a provided status.
fn base_query_from_status(status: &Status) -> HashMap<String, String> {
    let mut base_query: HashMap<String, String> = HashMap::new();
    base_query.insert("status".to_string(), serialized_status(status));
    base_query
}

/// Creates a serialized status.
pub fn serialized_status(status: &Status) -> String {
    serde_json::to_string(status)
        .expect(&format!(
            "Status param {status:?} impossibly failed serialization."
        ))
        // Trim the quotes on either side of the serialization so that
        // there don't end up being multiple quotes in the serialization
        // of the `status` enum.
        .trim_matches('"')
        .to_string()
}