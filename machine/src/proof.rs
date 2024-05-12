use p3_commit::Pcs;
use p3_matrix::dense::RowMajorMatrix;
use p3_stark::{ChipProof, Commitments};
use p3_uni_stark::{Com, PcsProof, StarkGenericConfig, Val};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub type PcsProverData<SC> = <<SC as StarkGenericConfig>::Pcs as Pcs<
    <SC as StarkGenericConfig>::Challenge,
    <SC as StarkGenericConfig>::Challenger,
>>::ProverData;

#[derive(Serialize, Deserialize)]
#[serde(bound = "SC::Challenge: Serialize + DeserializeOwned")]
pub struct MachineProof<SC: StarkGenericConfig> {
    pub commitments: Commitments<Com<SC>>,
    pub opening_proof: PcsProof<SC>,
    pub chip_proofs: Vec<ChipProof<SC::Challenge>>,
}

pub struct ProverPreprocessedData<SC: StarkGenericConfig> {
    pub traces: Vec<Option<RowMajorMatrix<Val<SC>>>>,
    pub data: Option<PcsProverData<SC>>,
    pub commitment: Option<Com<SC>>,
}

#[derive(Serialize, Deserialize)]
pub struct VerifierPreprocessedData<SC: StarkGenericConfig> {
    pub commitment: Com<SC>,
    pub degrees: Vec<usize>,
}

pub struct ProvingKey<SC: StarkGenericConfig> {
    pub preprocessed: ProverPreprocessedData<SC>,
}

#[derive(Serialize, Deserialize)]
pub struct VerifyingKey<SC: StarkGenericConfig> {
    pub preprocessed: Option<VerifierPreprocessedData<SC>>,
}
