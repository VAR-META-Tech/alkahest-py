use crate::{
    types::PyAddressConfig,
    PyAlkahestClient,
};
use alkahest_rs::{
    types::WalletProvider,
    utils::{setup_test_environment, MockAddresses, TestContext},
};
use pyo3::{pyclass, pymethods, PyResult};

#[pyclass]
#[derive(Clone)]

pub struct PyWalletProvider {
    pub inner: WalletProvider,
}
#[pyclass]
#[derive(Clone)]
pub struct PyMockAddresses {
    #[pyo3(get)]
    pub erc20_a: String,
    #[pyo3(get)]
    pub erc20_b: String,
    #[pyo3(get)]
    pub erc721_a: String,
    #[pyo3(get)]
    pub erc721_b: String,
    #[pyo3(get)]
    pub erc1155_a: String,
    #[pyo3(get)]
    pub erc1155_b: String,
}

impl From<&MockAddresses> for PyMockAddresses {
    fn from(m: &MockAddresses) -> Self {
        Self {
            erc20_a: format!("{:?}", m.erc20_a),
            erc20_b: format!("{:?}", m.erc20_b),
            erc721_a: format!("{:?}", m.erc721_a),
            erc721_b: format!("{:?}", m.erc721_b),
            erc1155_a: format!("{:?}", m.erc1155_a),
            erc1155_b: format!("{:?}", m.erc1155_b),
        }
    }
}

#[pyclass]
pub struct PyTestEnvManager {
    inner: TestContext, // Optional: keep TestContext for internal Rust usage
    runtime: tokio::runtime::Runtime,

    #[pyo3(get)]
    pub rpc_url: String,
    #[pyo3(get)]
    pub god: String,
    #[pyo3(get)]
    pub alice: String,
    #[pyo3(get)]
    pub bob: String,
    #[pyo3(get)]
    pub addresses: PyAddressConfig,
    #[pyo3(get)]
    pub mock_addresses: PyMockAddresses,
    #[pyo3(get)]
    pub alice_client: PyAlkahestClient,
    #[pyo3(get)]
    pub bob_client: PyAlkahestClient,
    #[pyo3(get)]
    pub god_wallet_provider: PyWalletProvider,
}

#[pymethods]
impl PyTestEnvManager {
    #[new]
    pub fn new() -> PyResult<Self> {
        let rt = tokio::runtime::Runtime::new()?;
        let ctx = rt
            .block_on(setup_test_environment())
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

        Ok(Self {
            runtime: rt,
            rpc_url: ctx.anvil.ws_endpoint_url().to_string(),
            god: ctx.god.address().to_string(),
            alice: ctx.alice.address().to_string(),
            bob: ctx.bob.address().to_string(),
            addresses: PyAddressConfig::from(&ctx.addresses),
            mock_addresses: PyMockAddresses::from(&ctx.mock_addresses),
            alice_client: PyAlkahestClient::from_client(ctx.alice_client.clone()),
            bob_client: PyAlkahestClient::from_client(ctx.bob_client.clone()),
            god_wallet_provider: PyWalletProvider {
                inner: ctx.god_provider.clone(),
            },
            inner: ctx,
        })
    }
}


