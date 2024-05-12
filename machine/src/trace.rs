use itertools::Itertools;
use p3_air::BaseAir;
use p3_commit::{Pcs, PolynomialSpace};
use p3_field::{AbstractField, ExtensionField, Field};
use p3_interaction::{
    generate_permutation_trace, InteractionAir, InteractionAirBuilder, NUM_PERM_CHALLENGES,
};
use p3_matrix::{dense::RowMajorMatrix, Matrix};
use p3_stark::symbolic::get_quotient_degree;
use p3_uni_stark::{Com, Domain, PackedChallenge, StarkGenericConfig, Val};

use crate::{chip::ChipType, proof::PcsProverData, quotient::quotient_values};

#[derive(Clone)]
pub struct Trace<F, Domain>
where
    F: Field,
    Domain: PolynomialSpace,
{
    pub value: RowMajorMatrix<F>,
    pub domain: Domain,
}

impl<EF, Domain> Trace<EF, Domain>
where
    EF: Field,
    Domain: PolynomialSpace,
{
    pub fn flatten_to_base<F: Field>(&self) -> Trace<F, Domain>
    where
        EF: ExtensionField<F>,
    {
        Trace {
            value: self.value.flatten_to_base(),
            domain: self.domain,
        }
    }
}

#[derive(Clone)]
pub struct IndexedTrace<F, Domain>
where
    F: Field,
    Domain: PolynomialSpace,
{
    pub trace: Trace<F, Domain>,
    pub opening_index: usize,
}

#[derive(Clone)]
pub struct QuotientTrace<Domain>
where
    Domain: PolynomialSpace,
{
    pub traces: Vec<Trace<Domain::Val, Domain>>,
    pub opening_index: usize,
}

#[derive(Clone)]
pub struct ChipTrace<'a, Domain, EF>
where
    Domain: PolynomialSpace,
    EF: ExtensionField<Domain::Val>,
{
    pub chip: &'a ChipType,

    pub preprocessed: Option<IndexedTrace<Domain::Val, Domain>>,
    pub main: Option<IndexedTrace<Domain::Val, Domain>>,
    pub permutation: Option<IndexedTrace<EF, Domain>>,

    pub cumulative_sum: Option<EF>,

    pub quotient_chunks: Option<QuotientTrace<Domain>>,
    pub quotient_degree: Option<usize>,
}

impl<'a, Domain, EF> ChipTrace<'a, Domain, EF>
where
    Domain: PolynomialSpace,
    EF: ExtensionField<Domain::Val>,
{
    pub fn new(chip: &'a ChipType) -> Self {
        Self {
            chip,
            preprocessed: None,
            main: None,
            permutation: None,
            cumulative_sum: None,
            quotient_chunks: None,
            quotient_degree: None,
        }
    }

    pub fn domain(&self) -> Option<Domain> {
        match (&self.preprocessed, &self.main) {
            (Some(preprocessed), Some(main)) => {
                let preprocessed_domain = preprocessed.trace.domain;
                let main_domain = main.trace.domain;
                if main_domain.size() > preprocessed_domain.size() {
                    Some(main_domain)
                } else {
                    Some(preprocessed_domain)
                }
            }
            (Some(preprocessed), None) => Some(preprocessed.trace.domain),
            (None, Some(main)) => Some(main.trace.domain),
            (None, None) => None,
        }
    }
}

pub type MachineTrace<'a, Domain, EF> = Vec<ChipTrace<'a, Domain, EF>>;

pub trait MachineTraceBuilder<'a> {
    fn new(chips: &'a [&ChipType]) -> Self;
}

impl<'a, Domain, EF> MachineTraceBuilder<'a> for MachineTrace<'a, Domain, EF>
where
    Domain: PolynomialSpace,
    EF: ExtensionField<Domain::Val>,
{
    fn new(chips: &'a [&ChipType]) -> Self {
        chips.iter().map(|chip| ChipTrace::new(chip)).collect_vec()
    }
}

pub trait MachineTraceLoader<'a, Domain, SC>
where
    Domain: PolynomialSpace,
    SC: StarkGenericConfig,
    SC::Pcs: Pcs<SC::Challenge, SC::Challenger, Domain = Domain>,
{
    fn generate_preprocessed(self, pcs: &SC::Pcs) -> Self;

    fn load_preprocessed(
        self,
        pcs: &SC::Pcs,
        traces: Vec<Option<RowMajorMatrix<Domain::Val>>>,
    ) -> Self;

    fn load_main(self, pcs: &SC::Pcs, traces: Vec<Option<RowMajorMatrix<Domain::Val>>>) -> Self;

    fn generate_permutation<AB>(
        self,
        pcs: &SC::Pcs,
        perm_challenges: [SC::Challenge; NUM_PERM_CHALLENGES],
    ) -> Self
    where
        AB: InteractionAirBuilder<Expr = Domain::Val>;

    fn generate_quotient(
        self,
        pcs: &SC::Pcs,
        preprocessed_data: Option<PcsProverData<SC>>,
        main_data: Option<PcsProverData<SC>>,
        permutation_data: Option<PcsProverData<SC>>,
        perm_challenges: [SC::Challenge; NUM_PERM_CHALLENGES],
        alpha: SC::Challenge,
    ) -> Self;
}

impl<'a, Domain, SC> MachineTraceLoader<'a, Domain, SC> for MachineTrace<'a, Domain, SC::Challenge>
where
    Domain: PolynomialSpace,
    SC: StarkGenericConfig,
    SC::Pcs: Pcs<SC::Challenge, SC::Challenger, Domain = Domain>,
{
    fn generate_preprocessed(mut self, pcs: &SC::Pcs) -> Self {
        let traces = self
            .iter()
            .map(|trace| trace.chip.preprocessed_trace())
            .collect_vec();
        let traces = load_traces::<SC, _>(pcs, traces);
        for (chip_trace, preprocessed) in self.iter_mut().zip_eq(traces) {
            chip_trace.preprocessed = preprocessed;
        }
        self
    }

    fn load_preprocessed(
        mut self,
        pcs: &SC::Pcs,
        traces: Vec<Option<RowMajorMatrix<Domain::Val>>>,
    ) -> Self {
        let traces = load_traces::<SC, _>(pcs, traces);
        for (chip_trace, preprocessed) in self.iter_mut().zip_eq(traces) {
            chip_trace.preprocessed = preprocessed;
        }
        self
    }

    fn load_main(
        mut self,
        pcs: &SC::Pcs,
        traces: Vec<Option<RowMajorMatrix<Domain::Val>>>,
    ) -> Self {
        let traces = load_traces::<SC, _>(pcs, traces);
        for (chip_trace, main) in self.iter_mut().zip_eq(traces) {
            chip_trace.main = main;
        }
        self
    }

    fn generate_permutation<AB>(
        mut self,
        pcs: &SC::Pcs,
        perm_challenges: [SC::Challenge; NUM_PERM_CHALLENGES],
    ) -> Self
    where
        AB: InteractionAirBuilder<Expr = Domain::Val>,
    {
        let traces = self
            .iter()
            .map(|trace| {
                let preprocessed = trace
                    .preprocessed
                    .as_ref()
                    .map(|mt| mt.trace.value.as_view());
                let main = trace.main.as_ref().map(|mt| mt.trace.value.as_view());
                let interactions = <ChipType as InteractionAir<AB>>::all_interactions(trace.chip);

                generate_permutation_trace(&preprocessed, &main, &interactions, perm_challenges)
            })
            .collect_vec();
        let cumulative_sums = traces
            .iter()
            .map(|mt| {
                mt.as_ref().map(|trace| {
                    let row = trace.row_slice(trace.height() - 1);
                    let cumulative_sum = row.last().unwrap();
                    *cumulative_sum
                })
            })
            .collect_vec();
        let traces = load_traces::<SC, _>(pcs, traces);
        for ((chip_trace, permutation), cumulative_sum) in self
            .iter_mut()
            .zip_eq(traces.into_iter())
            .zip_eq(cumulative_sums.into_iter())
        {
            chip_trace.permutation = permutation;
            chip_trace.cumulative_sum = cumulative_sum;
        }
        self
    }

    fn generate_quotient(
        mut self,
        pcs: &SC::Pcs,
        preprocessed_data: Option<PcsProverData<SC>>,
        main_data: Option<PcsProverData<SC>>,
        permutation_data: Option<PcsProverData<SC>>,
        perm_challenges: [SC::Challenge; NUM_PERM_CHALLENGES],
        alpha: SC::Challenge,
    ) -> Self {
        let perm_challenges = perm_challenges.map(PackedChallenge::<SC>::from_f);
        let alpha = PackedChallenge::<SC>::from_f(alpha);

        let mut count = 0;
        for chip_trace in self.iter_mut() {
            let quotient_degree = get_quotient_degree::<Val<SC>, _>(chip_trace.chip, 0);
            let trace_domain = chip_trace.domain();

            if let Some(trace_domain) = trace_domain {
                let quotient_domain =
                    trace_domain.create_disjoint_domain(trace_domain.size() * quotient_degree);

                let preprocessed_trace_on_quotient_domains =
                    if let Some(preprocessed) = &chip_trace.preprocessed {
                        pcs.get_evaluations_on_domain(
                            preprocessed_data.as_ref().unwrap(),
                            preprocessed.opening_index,
                            quotient_domain,
                        )
                        .to_row_major_matrix()
                    } else {
                        RowMajorMatrix::new_col(vec![Val::<SC>::zero(); quotient_domain.size()])
                    };
                let main_trace_on_quotient_domains = if let Some(main) = &chip_trace.main {
                    pcs.get_evaluations_on_domain(
                        main_data.as_ref().unwrap(),
                        main.opening_index,
                        quotient_domain,
                    )
                    .to_row_major_matrix()
                } else {
                    RowMajorMatrix::new_col(vec![Val::<SC>::zero(); quotient_domain.size()])
                };
                let perm_trace_on_quotient_domains =
                    if let Some(permutation) = &chip_trace.permutation {
                        pcs.get_evaluations_on_domain(
                            permutation_data.as_ref().unwrap(),
                            permutation.opening_index,
                            quotient_domain,
                        )
                        .to_row_major_matrix()
                    } else {
                        RowMajorMatrix::new_col(vec![Val::<SC>::zero(); quotient_domain.size()])
                    };

                let cumulative_sum = chip_trace
                    .cumulative_sum
                    .map(PackedChallenge::<SC>::from_f)
                    .unwrap_or_default();

                let quotient_values = quotient_values::<SC, _, _>(
                    chip_trace.chip,
                    trace_domain,
                    quotient_domain,
                    preprocessed_trace_on_quotient_domains,
                    main_trace_on_quotient_domains,
                    perm_trace_on_quotient_domains,
                    perm_challenges,
                    alpha,
                    cumulative_sum,
                );
                let quotient_flat = RowMajorMatrix::new_col(quotient_values).flatten_to_base();

                let chunks = quotient_domain.split_evals(quotient_degree, quotient_flat);
                let chunk_domains = quotient_domain.split_domains(quotient_degree);
                let traces = chunk_domains
                    .into_iter()
                    .zip(chunks.into_iter())
                    .map(|(domain, chunk)| Trace {
                        value: chunk,
                        domain,
                    })
                    .collect();

                chip_trace.quotient_chunks = Some(QuotientTrace {
                    traces,
                    opening_index: count,
                });
                count += 1;
            }
        }

        self
    }
}

pub trait MachineTraceCommiter<'a, Domain, SC>
where
    Domain: PolynomialSpace,
    SC: StarkGenericConfig,
    SC::Pcs: Pcs<SC::Challenge, SC::Challenger, Domain = Domain>,
{
    fn commit_preprocessed(self, pcs: &SC::Pcs) -> (Option<Com<SC>>, Option<PcsProverData<SC>>);

    fn commit_main(self, pcs: &SC::Pcs) -> (Option<Com<SC>>, Option<PcsProverData<SC>>);

    fn commit_permutation(self, pcs: &SC::Pcs) -> (Option<Com<SC>>, Option<PcsProverData<SC>>);

    fn commit_quotient(self, pcs: &SC::Pcs) -> (Option<Com<SC>>, Option<PcsProverData<SC>>);
}

impl<'a, Domain, SC> MachineTraceCommiter<'a, Domain, SC>
    for MachineTrace<'a, Domain, SC::Challenge>
where
    Domain: PolynomialSpace,
    SC: StarkGenericConfig,
    SC::Pcs: Pcs<SC::Challenge, SC::Challenger, Domain = Domain>,
{
    fn commit_preprocessed(self, pcs: &SC::Pcs) -> (Option<Com<SC>>, Option<PcsProverData<SC>>) {
        let traces = self
            .into_iter()
            .flat_map(|trace| trace.preprocessed.map(|preprocessed| preprocessed.trace))
            .collect_vec();
        commit_traces::<SC>(pcs, traces)
    }

    fn commit_main(self, pcs: &SC::Pcs) -> (Option<Com<SC>>, Option<PcsProverData<SC>>) {
        let traces = self
            .into_iter()
            .flat_map(|trace| trace.main.map(|main| main.trace))
            .collect_vec();
        commit_traces::<SC>(pcs, traces)
    }

    fn commit_permutation(self, pcs: &SC::Pcs) -> (Option<Com<SC>>, Option<PcsProverData<SC>>) {
        let traces = self
            .into_iter()
            .flat_map(|trace| {
                trace
                    .permutation
                    .map(|permutation| permutation.trace.flatten_to_base())
            })
            .collect_vec();
        commit_traces::<SC>(pcs, traces)
    }

    fn commit_quotient(self, pcs: &SC::Pcs) -> (Option<Com<SC>>, Option<PcsProverData<SC>>) {
        let traces = self
            .into_iter()
            .flat_map(|trace| trace.quotient_chunks.map(|quotient| quotient.traces))
            .flatten()
            .collect_vec();
        commit_traces::<SC>(pcs, traces)
    }
}

fn load_traces<SC, F>(
    pcs: &SC::Pcs,
    traces: Vec<Option<RowMajorMatrix<F>>>,
) -> Vec<Option<IndexedTrace<F, Domain<SC>>>>
where
    F: Field,
    SC: StarkGenericConfig,
{
    traces
        .into_iter()
        .scan(0usize, |count, mt| {
            Some({
                if let Some(trace) = mt {
                    let degree = trace.height();
                    if degree > 0 {
                        let domain = pcs.natural_domain_for_degree(degree);
                        let trace = Trace {
                            value: trace,
                            domain,
                        };
                        let index = *count;
                        *count += 1;

                        Some(IndexedTrace {
                            trace,
                            opening_index: index,
                        })
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
        })
        .collect()
}

fn commit_traces<SC>(
    pcs: &SC::Pcs,
    traces: Vec<Trace<Val<SC>, Domain<SC>>>,
) -> (Option<Com<SC>>, Option<PcsProverData<SC>>)
where
    SC: StarkGenericConfig,
{
    let domains_and_traces: Vec<_> = traces
        .into_iter()
        .map(|trace| (trace.domain, trace.value))
        .collect();
    if !domains_and_traces.is_empty() {
        let (commit, data) = pcs.commit(domains_and_traces);
        (Some(commit), Some(data))
    } else {
        (None, None)
    }
}
