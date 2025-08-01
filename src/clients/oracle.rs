use std::marker::PhantomData;

use alkahest_rs::{
    clients::oracle::OracleClient as InnerOracleClient, contracts::StringObligation,
};
use alloy::primitives::FixedBytes;
use pyo3::{pyclass, pymethods, PyAny, PyObject, PyResult, Python};
use pyo3_async_runtimes::tokio::future_into_py;

use crate::{
    clients::string_obligation::PyStringObligationData,
    error_handling::{map_eyre_to_pyerr, map_parse_to_pyerr, map_sol_decode_to_pyerr},
};
use alkahest_rs::clients::arbiters::TrustedOracleArbiter;

#[pyclass]
#[derive(Clone)]
pub struct OracleClient {
    inner: InnerOracleClient,
}

impl OracleClient {
    pub fn new(inner: InnerOracleClient) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl OracleClient {
    pub fn unsubscribe<'py>(
        &self,
        py: Python<'py>,
        local_id: String,
    ) -> PyResult<pyo3::Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        future_into_py(py, async move {
            let local_id: FixedBytes<32> = local_id.parse().map_err(map_parse_to_pyerr)?;
            inner
                .unsubscribe(local_id)
                .await
                .map_err(map_eyre_to_pyerr)?;
            Ok(())
        })
    }

    pub fn get_eas_address(&self) -> String {
        format!("{:?}", self.inner.addresses.eas)
    }

    pub fn get_trusted_oracle_arbiter_address(&self) -> String {
        format!("{:?}", self.inner.addresses.trusted_oracle_arbiter)
    }

    pub fn arbitrate_past_sync<'py>(
        &self,
        py: Python<'py>,
        fulfillment_params: PyFulfillmentParams,
        decision_func: PyObject,
        options: Option<PyArbitrateOptions>,
    ) -> PyResult<pyo3::Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        future_into_py(py, async move {
            let opts = options.unwrap_or_default();

            let rust_filter = fulfillment_params
                .filter
                .try_into()
                .map_err(|e| map_eyre_to_pyerr(eyre::eyre!("Failed to convert filter: {}", e)))?;

            let fulfillment = alkahest_rs::clients::oracle::FulfillmentParams {
                _obligation_data: PhantomData::<StringObligation::ObligationData>,
                filter: rust_filter,
            };

            let arbitrate_options = alkahest_rs::clients::oracle::ArbitrateOptions {
                require_oracle: opts.require_oracle,
                skip_arbitrated: opts.skip_arbitrated,
                require_request: opts.require_request,
                only_new: opts.only_new,
            };

            let arbitrate_func =
                |obligation_data: &StringObligation::ObligationData| -> Option<bool> {
                    Python::with_gil(|py| {
                        let py_obligation = pyo3::types::PyString::new(py, &obligation_data.item);

                        decision_func
                            .call1(py, (py_obligation,))
                            .ok()
                            .and_then(|result| {
                                result
                                    .extract::<bool>(py)
                                    .or_else(|_| result.is_truthy(py))
                                    .ok()
                            })
                    })
                };

            let decisions = inner
                .arbitrate_past_sync(&fulfillment, &arbitrate_func, &arbitrate_options)
                .await
                .map_err(map_eyre_to_pyerr)?;

            let py_decisions: Vec<PyDecision> = decisions
                .into_iter()
                .map(|decision| {
                    let attestation = PyOracleAttestation::from(&decision.attestation);
                    PyDecision::__new__(
                        attestation,
                        decision.decision,
                        format!(
                            "0x{}",
                            alloy::hex::encode(decision.receipt.transaction_hash.as_slice())
                        ),
                        Some(decision.obligation.item),
                        None,
                    )
                })
                .collect();

            let total_count = py_decisions.len();
            let successful_count = py_decisions.iter().filter(|d| d.decision).count();

            Ok(PyArbitrationResult::__new__(
                py_decisions,
                successful_count,
                total_count,
            ))
        })
    }

    pub fn arbitrate_past_for_escrow_sync<'py>(
        &self,
        py: Python<'py>,
        escrow_params: PyEscrowParams,
        fulfillment_params: PyFulfillmentParams,
        decision_func: PyObject,
        options: Option<PyArbitrateOptions>,
    ) -> PyResult<pyo3::Bound<'py, PyAny>> {
        let inner = self.inner.clone();

        future_into_py(py, async move {
            let opts = options.unwrap_or_default();

            let escrow_filter = escrow_params.filter.try_into().map_err(|e| {
                map_eyre_to_pyerr(eyre::eyre!("Failed to convert escrow filter: {}", e))
            })?;

            let rust_filter = fulfillment_params.filter.try_into().map_err(|e| {
                map_eyre_to_pyerr(eyre::eyre!("Failed to convert fulfillment filter: {}", e))
            })?;

            use alkahest_rs::clients::arbiters::TrustedOracleArbiter;
            use alloy::primitives::Bytes;
            use alloy::sol_types::SolValue;

            let demand_bytes = Bytes::from(escrow_params.demand_abi.clone());
            let demand_abi = TrustedOracleArbiter::DemandData::abi_decode(&demand_bytes)
                .map_err(map_sol_decode_to_pyerr)?;

            let escrow = alkahest_rs::clients::oracle::EscrowParams {
                filter: escrow_filter,
                _demand_data: PhantomData::<TrustedOracleArbiter::DemandData>,
            };

            let fulfillment = alkahest_rs::clients::oracle::FulfillmentParams {
                _obligation_data: PhantomData::<StringObligation::ObligationData>,
                filter: rust_filter,
            };

            let arbitrate_options = alkahest_rs::clients::oracle::ArbitrateOptions {
                require_oracle: opts.require_oracle,
                skip_arbitrated: opts.skip_arbitrated,
                require_request: opts.require_request,
                only_new: opts.only_new,
            };

            let arbitrate_func = |obligation_data: &StringObligation::ObligationData,
                                  demand_data: &TrustedOracleArbiter::DemandData|
             -> Option<bool> {
                Python::with_gil(|py| {
                    let py_obligation = pyo3::types::PyString::new(py, &obligation_data.item);

                    let demand_py = PyTrustedOracleArbiterDemandData::from(demand_data.clone());

                    decision_func
                        .call1(py, (py_obligation, demand_py))
                        .ok()
                        .and_then(|result| {
                            result
                                .extract::<bool>(py)
                                .or_else(|_| result.is_truthy(py))
                                .ok()
                        })
                })
            };

            let (decisions, escrow_result, _) = inner
                .arbitrate_past_for_escrow_sync(
                    &escrow,
                    &fulfillment,
                    arbitrate_func,
                    &arbitrate_options,
                )
                .await
                .map_err(map_eyre_to_pyerr)?;

            let py_decisions: Vec<PyDecision> = decisions
                .into_iter()
                .map(|decision| {
                    let attestation = PyOracleAttestation::from(&decision.attestation);
                    PyDecision::__new__(
                        attestation,
                        decision.decision,
                        format!(
                            "0x{}",
                            alloy::hex::encode(decision.receipt.transaction_hash.as_slice())
                        ),
                        Some(decision.obligation.item),
                        None,
                    )
                })
                .collect();

            let py_escrow_attestations: Vec<PyOracleAttestation> = escrow_result
                .into_iter()
                .map(|att| PyOracleAttestation::from(&att))
                .collect();

            let py_demands = vec![format!(
                "oracle: 0x{:x}, data: {} bytes",
                demand_abi.oracle,
                demand_abi.data.len()
            )];

            Ok(PyEscrowArbitrationResult::__new__(
                py_decisions,
                py_escrow_attestations,
                py_demands,
            ))
        })
    }

    pub fn listen_and_arbitrate_no_spawn<'py>(
        &self,
        py: Python<'py>,
        fulfillment_params: PyFulfillmentParams,
        decision_func: PyObject,
        callback_func: Option<PyObject>,
        options: Option<PyArbitrateOptions>,
        timeout_seconds: Option<f64>,
    ) -> PyResult<pyo3::Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        future_into_py(py, async move {
            let opts = options.unwrap_or_default();
            let timeout = timeout_seconds.map(|secs| std::time::Duration::from_secs_f64(secs));

            let rust_filter = fulfillment_params
                .filter
                .try_into()
                .map_err(|e| map_eyre_to_pyerr(eyre::eyre!("Failed to convert filter: {}", e)))?;

            let fulfillment = alkahest_rs::clients::oracle::FulfillmentParams {
                _obligation_data: PhantomData::<StringObligation::ObligationData>,
                filter: rust_filter,
            };

            let arbitrate_options = alkahest_rs::clients::oracle::ArbitrateOptions {
                require_oracle: opts.require_oracle,
                skip_arbitrated: opts.skip_arbitrated,
                require_request: opts.require_request,
                only_new: opts.only_new,
            };

            let arbitrate_func =
                |obligation_data: &StringObligation::ObligationData| -> Option<bool> {
                    Python::with_gil(|py| {
                        let py_obligation = pyo3::types::PyString::new(py, &obligation_data.item);

                        decision_func
                            .call1(py, (py_obligation,))
                            .ok()
                            .and_then(|result| {
                                result
                                    .extract::<bool>(py)
                                    .or_else(|_| result.is_truthy(py))
                                    .ok()
                            })
                    })
                };

            let callback = |decision: &alkahest_rs::clients::oracle::Decision<
                StringObligation::ObligationData,
                (),
            >| {
                if let Some(ref py_callback) = callback_func {
                    Python::with_gil(|py| {
                        let decision_info = format!(
                            "Decision: {} for obligation: '{}'",
                            decision.decision, decision.obligation.item
                        );

                        if let Err(e) = py_callback.call1(py, (decision_info,)) {
                            panic!("Python callback failed: {}", e);
                        }
                    });
                }

                Box::pin(async {})
            };

            let listen_result = inner
                .listen_and_arbitrate_no_spawn(
                    &fulfillment,
                    &arbitrate_func,
                    callback,
                    &arbitrate_options,
                    timeout,
                )
                .await
                .map_err(map_eyre_to_pyerr)?;

            let py_decisions: Vec<PyDecision> = listen_result
                .decisions
                .into_iter()
                .map(|decision| {
                    let attestation = PyOracleAttestation::from(&decision.attestation);
                    PyDecision::__new__(
                        attestation,
                        decision.decision,
                        format!(
                            "0x{}",
                            alloy::hex::encode(decision.receipt.transaction_hash.as_slice())
                        ),
                        Some(decision.obligation.item),
                        None,
                    )
                })
                .collect();

            Ok(py_decisions)
        })
    }

    pub fn listen_and_arbitrate_for_escrow_no_spawn<'py>(
        &self,
        py: Python<'py>,
        escrow_params: PyEscrowParams,
        fulfillment_params: PyFulfillmentParams,
        decision_func: PyObject,
        callback_func: Option<PyObject>,
        options: Option<PyArbitrateOptions>,
        timeout_secs: Option<u64>,
    ) -> PyResult<pyo3::Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        future_into_py(py, async move {
            let opts = options.unwrap_or_default();
            let timeout = timeout_secs.map(std::time::Duration::from_secs);

            let escrow_filter = escrow_params.filter.try_into().map_err(|e| {
                map_eyre_to_pyerr(eyre::eyre!("Failed to convert escrow filter: {}", e))
            })?;

            let rust_filter = fulfillment_params.filter.try_into().map_err(|e| {
                map_eyre_to_pyerr(eyre::eyre!("Failed to convert fulfillment filter: {}", e))
            })?;

            use alloy::primitives::Bytes;
            use alloy::sol_types::SolValue;

            let demand_bytes = Bytes::from(escrow_params.demand_abi.clone());
            let _demand_abi = TrustedOracleArbiter::DemandData::abi_decode(&demand_bytes)
                .map_err(map_sol_decode_to_pyerr)?;

            let escrow = alkahest_rs::clients::oracle::EscrowParams {
                filter: escrow_filter,
                _demand_data: PhantomData::<TrustedOracleArbiter::DemandData>,
            };

            let fulfillment = alkahest_rs::clients::oracle::FulfillmentParams {
                _obligation_data: PhantomData::<StringObligation::ObligationData>,
                filter: rust_filter,
            };

            let arbitrate_options = alkahest_rs::clients::oracle::ArbitrateOptions {
                require_oracle: opts.require_oracle,
                skip_arbitrated: opts.skip_arbitrated,
                require_request: opts.require_request,
                only_new: opts.only_new,
            };

            let arbitrate_func = |obligation_data: &StringObligation::ObligationData,
                                  demand_data: &TrustedOracleArbiter::DemandData|
             -> Option<bool> {
                Python::with_gil(|py| {
                    let py_obligation = pyo3::types::PyString::new(py, &obligation_data.item);

                    let demand_py = PyTrustedOracleArbiterDemandData::from(demand_data.clone());

                    decision_func
                        .call1(py, (py_obligation, demand_py))
                        .ok()
                        .and_then(|result| {
                            result
                                .extract::<bool>(py)
                                .or_else(|_| result.is_truthy(py))
                                .ok()
                        })
                })
            };

            let callback = |decision_info: &alkahest_rs::clients::oracle::Decision<
                StringObligation::ObligationData,
                TrustedOracleArbiter::DemandData,
            >| {
                if let Some(ref py_callback) = callback_func {
                    Python::with_gil(|py| {
                        let decision_info_str = format!(
                            "Decision: {} for obligation: '{}'",
                            decision_info.decision, decision_info.obligation.item
                        );

                        if let Err(e) = py_callback.call1(py, (decision_info_str,)) {
                            panic!("Python callback failed: {}", e);
                        }
                    });
                }

                Box::pin(async {})
            };

            let result = inner
                .listen_and_arbitrate_for_escrow_no_spawn(
                    &escrow,
                    &fulfillment,
                    &arbitrate_func,
                    callback,
                    &arbitrate_options,
                    timeout,
                )
                .await
                .map_err(map_eyre_to_pyerr)?;

            let py_decisions: Vec<PyDecision> = result
                .decisions
                .into_iter()
                .map(|decision| {
                    let py_attestation = PyOracleAttestation::from(&decision.attestation);
                    PyDecision::__new__(
                        py_attestation,
                        decision.decision,
                        format!("{:?}", decision.receipt.transaction_hash),
                        Some(decision.obligation.item.clone()),
                        decision
                            .demand
                            .map(|d| PyTrustedOracleArbiterDemandData::from(d).oracle),
                    )
                })
                .collect();

            let py_escrow_attestations: Vec<PyOracleAttestation> = result
                .escrow_attestations
                .into_iter()
                .map(|att| PyOracleAttestation::from(&att))
                .collect();

            let py_escrow_demands = vec![];

            Ok(PyEscrowArbitrationResult::__new__(
                py_decisions,
                py_escrow_attestations,
                py_escrow_demands,
            ))
        })
    }
}

#[pyclass]
#[derive(Clone)]
pub struct PyOracleAddresses {
    #[pyo3(get)]
    pub eas: String,
    #[pyo3(get)]
    pub trusted_oracle_arbiter: String,
}

#[pymethods]
impl PyOracleAddresses {
    #[new]
    pub fn __new__(eas: String, trusted_oracle_arbiter: String) -> Self {
        Self {
            eas,
            trusted_oracle_arbiter,
        }
    }

    pub fn __str__(&self) -> String {
        format!(
            "PyOracleAddresses(eas={}, trusted_oracle_arbiter={})",
            self.eas, self.trusted_oracle_arbiter
        )
    }

    pub fn __repr__(&self) -> String {
        self.__str__()
    }
}

impl TryFrom<PyOracleAddresses> for alkahest_rs::clients::oracle::OracleAddresses {
    type Error = eyre::Error;

    fn try_from(value: PyOracleAddresses) -> eyre::Result<Self> {
        Ok(Self {
            eas: value.eas.parse()?,
            trusted_oracle_arbiter: value.trusted_oracle_arbiter.parse()?,
        })
    }
}

#[pyclass]
#[derive(Clone)]
pub struct PyAttestationFilter {
    #[pyo3(get, set)]
    pub attester: Option<String>,
    #[pyo3(get, set)]
    pub recipient: Option<String>,
    #[pyo3(get, set)]
    pub schema_uid: Option<String>,
    #[pyo3(get, set)]
    pub uid: Option<String>,
    #[pyo3(get, set)]
    pub ref_uid: Option<String>,
    #[pyo3(get, set)]
    pub from_block: Option<u64>,
    #[pyo3(get, set)]
    pub to_block: Option<u64>,
}

#[pymethods]
impl PyAttestationFilter {
    #[new]
    #[pyo3(signature = (attester=None, recipient=None, schema_uid=None, uid=None, ref_uid=None, from_block=None, to_block=None))]
    pub fn __new__(
        attester: Option<String>,
        recipient: Option<String>,
        schema_uid: Option<String>,
        uid: Option<String>,
        ref_uid: Option<String>,
        from_block: Option<u64>,
        to_block: Option<u64>,
    ) -> Self {
        Self {
            attester,
            recipient,
            schema_uid,
            uid,
            ref_uid,
            from_block,
            to_block,
        }
    }

    pub fn __str__(&self) -> String {
        format!(
            "PyAttestationFilter(attester={:?}, recipient={:?}, schema_uid={:?}, uid={:?}, ref_uid={:?}, from_block={:?}, to_block={:?})",
            self.attester, self.recipient, self.schema_uid, self.uid, self.ref_uid, self.from_block, self.to_block
        )
    }

    pub fn __repr__(&self) -> String {
        self.__str__()
    }
}

#[pyclass]
#[derive(Clone)]
pub struct PyArbitrateOptions {
    #[pyo3(get, set)]
    pub require_oracle: bool,
    #[pyo3(get, set)]
    pub skip_arbitrated: bool,
    #[pyo3(get, set)]
    pub require_request: bool,
    #[pyo3(get, set)]
    pub only_new: bool,
}

#[pymethods]
impl PyArbitrateOptions {
    #[new]
    #[pyo3(signature = (require_oracle=false, skip_arbitrated=false, require_request=false, only_new=false))]
    pub fn __new__(
        require_oracle: bool,
        skip_arbitrated: bool,
        require_request: bool,
        only_new: bool,
    ) -> Self {
        Self {
            require_oracle,
            skip_arbitrated,
            require_request,
            only_new,
        }
    }

    pub fn __str__(&self) -> String {
        format!(
            "PyArbitrateOptions(require_oracle={}, skip_arbitrated={}, require_request={}, only_new={})",
            self.require_oracle, self.skip_arbitrated, self.require_request, self.only_new
        )
    }

    pub fn __repr__(&self) -> String {
        self.__str__()
    }
}

impl Default for PyArbitrateOptions {
    fn default() -> Self {
        Self {
            require_oracle: false,
            skip_arbitrated: false,
            require_request: false,
            only_new: false,
        }
    }
}

#[pyclass]
#[derive(Clone)]
pub struct PyFulfillmentParams {
    #[pyo3(get, set)]
    pub obligation_abi: PyStringObligationData,
    #[pyo3(get, set)]
    pub filter: PyAttestationFilter,
}

#[pymethods]
impl PyFulfillmentParams {
    #[new]
    pub fn __new__(obligation_abi: PyStringObligationData, filter: PyAttestationFilter) -> Self {
        Self {
            obligation_abi,
            filter,
        }
    }

    pub fn __str__(&self) -> String {
        format!(
            "PyFulfillmentParams(obligation_abi={:?}, filter={})",
            self.obligation_abi,
            self.filter.__str__()
        )
    }

    pub fn __repr__(&self) -> String {
        self.__str__()
    }
}

#[pyclass]
#[derive(Clone)]
pub struct PyEscrowParams {
    #[pyo3(get, set)]
    pub demand_abi: Vec<u8>,
    #[pyo3(get, set)]
    pub filter: PyAttestationFilter,
}

#[pymethods]
impl PyEscrowParams {
    #[new]
    pub fn __new__(demand_abi: Vec<u8>, filter: PyAttestationFilter) -> Self {
        Self { demand_abi, filter }
    }

    pub fn __str__(&self) -> String {
        format!(
            "PyEscrowParams(demand_abi={} bytes, filter={})",
            self.demand_abi.len(),
            self.filter.__str__()
        )
    }

    pub fn __repr__(&self) -> String {
        self.__str__()
    }
}

#[pyclass]
#[derive(Clone)]
pub struct PyOracleAttestation {
    #[pyo3(get)]
    pub uid: String,
    #[pyo3(get)]
    pub schema: String,
    #[pyo3(get)]
    pub ref_uid: String,
    #[pyo3(get)]
    pub time: u64,
    #[pyo3(get)]
    pub expiration_time: u64,
    #[pyo3(get)]
    pub revocation_time: u64,
    #[pyo3(get)]
    pub recipient: String,
    #[pyo3(get)]
    pub attester: String,
    #[pyo3(get)]
    pub revocable: bool,
    #[pyo3(get)]
    pub data: String,
}

#[pymethods]
impl PyOracleAttestation {
    #[new]
    pub fn __new__(
        uid: String,
        schema: String,
        ref_uid: String,
        time: u64,
        expiration_time: u64,
        revocation_time: u64,
        recipient: String,
        attester: String,
        revocable: bool,
        data: String,
    ) -> Self {
        Self {
            uid,
            schema,
            ref_uid,
            time,
            expiration_time,
            revocation_time,
            recipient,
            attester,
            revocable,
            data,
        }
    }

    pub fn __str__(&self) -> String {
        format!(
            "PyOracleAttestation(uid={}, schema={}, attester={}, recipient={})",
            self.uid, self.schema, self.attester, self.recipient
        )
    }

    pub fn __repr__(&self) -> String {
        self.__str__()
    }
}

impl From<&alkahest_rs::contracts::IEAS::Attestation> for PyOracleAttestation {
    fn from(att: &alkahest_rs::contracts::IEAS::Attestation) -> Self {
        Self::__new__(
            format!("0x{}", alloy::hex::encode(att.uid.as_slice())),
            format!("0x{}", alloy::hex::encode(att.schema.as_slice())),
            format!("0x{}", alloy::hex::encode(att.refUID.as_slice())),
            att.time,
            att.expirationTime,
            att.revocationTime,
            format!("0x{:x}", att.recipient),
            format!("0x{:x}", att.attester),
            att.revocable,
            format!("0x{}", alloy::hex::encode(&att.data)),
        )
    }
}

impl From<alkahest_rs::contracts::IEAS::Attestation> for PyOracleAttestation {
    fn from(att: alkahest_rs::contracts::IEAS::Attestation) -> Self {
        Self::from(&att)
    }
}

#[pyclass]
#[derive(Clone)]
pub struct PyDecision {
    #[pyo3(get)]
    pub attestation: PyOracleAttestation,
    #[pyo3(get)]
    pub decision: bool,
    #[pyo3(get)]
    pub transaction_hash: String,
    #[pyo3(get)]
    pub obligation_data: Option<String>,
    #[pyo3(get)]
    pub demand_data: Option<String>,
}

#[pymethods]
impl PyDecision {
    #[new]
    #[pyo3(signature = (attestation, decision, transaction_hash, obligation_data=None, demand_data=None))]
    pub fn __new__(
        attestation: PyOracleAttestation,
        decision: bool,
        transaction_hash: String,
        obligation_data: Option<String>,
        demand_data: Option<String>,
    ) -> Self {
        Self {
            attestation,
            decision,
            transaction_hash,
            obligation_data,
            demand_data,
        }
    }

    pub fn __str__(&self) -> String {
        format!(
            "PyDecision(decision={}, tx_hash={})",
            self.decision, self.transaction_hash
        )
    }

    pub fn __repr__(&self) -> String {
        self.__str__()
    }
}

// ===== RESULT TYPES =====

#[pyclass]
#[derive(Clone)]
pub struct PyArbitrationResult {
    #[pyo3(get)]
    pub decisions: Vec<PyDecision>,
    #[pyo3(get)]
    pub successful_count: usize,
    #[pyo3(get)]
    pub total_count: usize,
}

#[pymethods]
impl PyArbitrationResult {
    #[new]
    pub fn __new__(
        decisions: Vec<PyDecision>,
        successful_count: usize,
        total_count: usize,
    ) -> Self {
        Self {
            decisions,
            successful_count,
            total_count,
        }
    }

    pub fn __str__(&self) -> String {
        format!(
            "PyArbitrationResult(successful={}/{}, decisions={})",
            self.successful_count,
            self.total_count,
            self.decisions.len()
        )
    }

    pub fn __repr__(&self) -> String {
        self.__str__()
    }
}

#[pyclass]
#[derive(Clone)]
pub struct PySubscriptionResult {
    #[pyo3(get)]
    pub subscription_id: String,
    #[pyo3(get)]
    pub initial_decisions: Vec<PyDecision>,
}

#[pymethods]
impl PySubscriptionResult {
    #[new]
    pub fn __new__(subscription_id: String, initial_decisions: Vec<PyDecision>) -> Self {
        Self {
            subscription_id,
            initial_decisions,
        }
    }

    pub fn __str__(&self) -> String {
        format!(
            "PySubscriptionResult(id={}, initial_decisions={})",
            self.subscription_id,
            self.initial_decisions.len()
        )
    }

    pub fn __repr__(&self) -> String {
        self.__str__()
    }
}

#[pyclass]
#[derive(Clone)]
pub struct PyEscrowResult {
    #[pyo3(get)]
    pub escrow_attestations: Vec<PyOracleAttestation>,
    #[pyo3(get)]
    pub escrow_demands: Vec<String>,
}

#[pymethods]
impl PyEscrowResult {
    #[new]
    pub fn __new__(
        escrow_attestations: Vec<PyOracleAttestation>,
        escrow_demands: Vec<String>,
    ) -> Self {
        Self {
            escrow_attestations,
            escrow_demands,
        }
    }

    pub fn __str__(&self) -> String {
        format!(
            "PyEscrowResult(attestations={}, demands={})",
            self.escrow_attestations.len(),
            self.escrow_demands.len()
        )
    }

    pub fn __repr__(&self) -> String {
        self.__str__()
    }
}

#[pyclass]
#[derive(Clone)]
pub struct PyEscrowArbitrationResult {
    #[pyo3(get)]
    pub decisions: Vec<PyDecision>,
    #[pyo3(get)]
    pub escrow_attestations: Vec<PyOracleAttestation>,
    #[pyo3(get)]
    pub escrow_demands: Vec<String>, // JSON or hex representation
}

#[pymethods]
impl PyEscrowArbitrationResult {
    #[new]
    pub fn __new__(
        decisions: Vec<PyDecision>,
        escrow_attestations: Vec<PyOracleAttestation>,
        escrow_demands: Vec<String>,
    ) -> Self {
        Self {
            decisions,
            escrow_attestations,
            escrow_demands,
        }
    }

    pub fn __str__(&self) -> String {
        format!(
            "PyEscrowArbitrationResult(decisions={}, escrows={}, demands={})",
            self.decisions.len(),
            self.escrow_attestations.len(),
            self.escrow_demands.len()
        )
    }

    pub fn __repr__(&self) -> String {
        self.__str__()
    }
}

#[pyclass]
#[derive(Clone)]
pub struct PyEscrowSubscriptionResult {
    #[pyo3(get)]
    pub initial_decisions: Vec<PyDecision>,
    #[pyo3(get)]
    pub escrow_attestations: Vec<PyOracleAttestation>,
    #[pyo3(get)]
    pub escrow_subscription_id: String,
    #[pyo3(get)]
    pub fulfillment_subscription_id: String,
}

#[pymethods]
impl PyEscrowSubscriptionResult {
    #[new]
    pub fn __new__(
        initial_decisions: Vec<PyDecision>,
        escrow_attestations: Vec<PyOracleAttestation>,
        escrow_subscription_id: String,
        fulfillment_subscription_id: String,
    ) -> Self {
        Self {
            initial_decisions,
            escrow_attestations,
            escrow_subscription_id,
            fulfillment_subscription_id,
        }
    }

    pub fn __str__(&self) -> String {
        format!(
            "PyEscrowSubscriptionResult(decisions={}, escrows={}, escrow_id={}, fulfillment_id={})",
            self.initial_decisions.len(),
            self.escrow_attestations.len(),
            self.escrow_subscription_id,
            self.fulfillment_subscription_id
        )
    }

    pub fn __repr__(&self) -> String {
        self.__str__()
    }
}

// ===== TYPE CONVERSIONS =====

impl TryFrom<PyAttestationFilter> for alkahest_rs::clients::oracle::AttestationFilter {
    type Error = eyre::Error;

    fn try_from(py_filter: PyAttestationFilter) -> eyre::Result<Self> {
        use alloy::{
            eips::BlockNumberOrTag,
            primitives::{Address, FixedBytes},
            rpc::types::{FilterBlockOption, ValueOrArray},
        };

        let block_option = if py_filter.from_block.is_some() || py_filter.to_block.is_some() {
            Some(FilterBlockOption::Range {
                from_block: py_filter.from_block.map(|b| BlockNumberOrTag::Number(b)),
                to_block: py_filter.to_block.map(|b| BlockNumberOrTag::Number(b)),
            })
        } else {
            None
        };

        let attester = if let Some(addr_str) = py_filter.attester {
            Some(ValueOrArray::Value(addr_str.parse::<Address>()?))
        } else {
            None
        };

        let recipient = if let Some(addr_str) = py_filter.recipient {
            Some(ValueOrArray::Value(addr_str.parse::<Address>()?))
        } else {
            None
        };

        let schema_uid = if let Some(uid_str) = py_filter.schema_uid {
            let bytes = alloy::hex::decode(uid_str.strip_prefix("0x").unwrap_or(&uid_str))?;
            if bytes.len() != 32 {
                return Err(eyre::eyre!("Schema UID must be 32 bytes"));
            }
            let mut fixed_bytes = [0u8; 32];
            fixed_bytes.copy_from_slice(&bytes);
            Some(ValueOrArray::Value(FixedBytes::from(fixed_bytes)))
        } else {
            None
        };

        let uid = if let Some(uid_str) = py_filter.uid {
            let bytes = alloy::hex::decode(uid_str.strip_prefix("0x").unwrap_or(&uid_str))?;
            if bytes.len() != 32 {
                return Err(eyre::eyre!("UID must be 32 bytes"));
            }
            let mut fixed_bytes = [0u8; 32];
            fixed_bytes.copy_from_slice(&bytes);
            Some(ValueOrArray::Value(FixedBytes::from(fixed_bytes)))
        } else {
            None
        };

        let ref_uid = if let Some(uid_str) = py_filter.ref_uid {
            let bytes = alloy::hex::decode(uid_str.strip_prefix("0x").unwrap_or(&uid_str))?;
            if bytes.len() != 32 {
                return Err(eyre::eyre!("Ref UID must be 32 bytes"));
            }
            let mut fixed_bytes = [0u8; 32];
            fixed_bytes.copy_from_slice(&bytes);
            Some(ValueOrArray::Value(FixedBytes::from(fixed_bytes)))
        } else {
            None
        };

        Ok(alkahest_rs::clients::oracle::AttestationFilter {
            block_option,
            attester,
            recipient,
            schema_uid,
            uid,
            ref_uid,
        })
    }
}

#[pyclass]
#[derive(Clone)]
pub struct PyTrustedOracleArbiterDemandData {
    #[pyo3(get)]
    pub oracle: String,
    #[pyo3(get)]
    pub data: Vec<u8>,
}

#[pymethods]
impl PyTrustedOracleArbiterDemandData {
    #[new]
    pub fn new(oracle: String, data: Vec<u8>) -> Self {
        Self { oracle, data }
    }

    fn __repr__(&self) -> String {
        format!(
            "PyTrustedOracleArbiterDemandData(oracle='{}', data={:?})",
            self.oracle, self.data
        )
    }

    #[staticmethod]
    pub fn decode(demand_bytes: Vec<u8>) -> eyre::Result<PyTrustedOracleArbiterDemandData> {
        use alkahest_rs::clients::arbiters::TrustedOracleArbiter;
        use alloy::primitives::Bytes;
        use alloy::sol_types::SolValue;

        let bytes = Bytes::from(demand_bytes);
        let decoded = TrustedOracleArbiter::DemandData::abi_decode(&bytes)?;
        Ok(decoded.into())
    }

    #[staticmethod]
    pub fn encode(demand_data: &PyTrustedOracleArbiterDemandData) -> eyre::Result<Vec<u8>> {
        use alkahest_rs::clients::arbiters::{ArbitersClient, TrustedOracleArbiter};
        use alloy::primitives::{Address, Bytes};

        let oracle: Address = demand_data.oracle.parse()?;
        let data = Bytes::from(demand_data.data.clone());

        let rust_demand_data = TrustedOracleArbiter::DemandData { oracle, data };
        let encoded = ArbitersClient::encode_trusted_oracle_arbiter_demand(&rust_demand_data);
        Ok(encoded.to_vec())
    }

    pub fn encode_self(&self) -> eyre::Result<Vec<u8>> {
        PyTrustedOracleArbiterDemandData::encode(self)
    }
}

impl From<alkahest_rs::clients::arbiters::TrustedOracleArbiter::DemandData>
    for PyTrustedOracleArbiterDemandData
{
    fn from(data: alkahest_rs::clients::arbiters::TrustedOracleArbiter::DemandData) -> Self {
        Self {
            oracle: format!("{:?}", data.oracle),
            data: data.data.to_vec(),
        }
    }
}

impl TryFrom<PyTrustedOracleArbiterDemandData>
    for alkahest_rs::clients::arbiters::TrustedOracleArbiter::DemandData
{
    type Error = eyre::Error;

    fn try_from(py_data: PyTrustedOracleArbiterDemandData) -> eyre::Result<Self> {
        use alloy::primitives::{Address, Bytes};

        let oracle: Address = py_data.oracle.parse()?;
        let data = Bytes::from(py_data.data);

        Ok(Self { oracle, data })
    }
}
