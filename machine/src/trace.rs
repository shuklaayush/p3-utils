use itertools::Itertools;
use p3_commit::{Pcs, PolynomialSpace};
use p3_field::{ExtensionField, Field};
use p3_interaction::{
    generate_permutation_trace, InteractionAir, InteractionAirBuilder, NUM_PERM_CHALLENGES,
};
use p3_matrix::{dense::RowMajorMatrix, Matrix};
use p3_uni_stark::{Com, Domain, StarkGenericConfig, Val};

use crate::{chip::ChipType, proof::PcsProverData};

#[derive(Clone)]
pub struct Trace<F, Domain>
where
    F: Field,
    Domain: PolynomialSpace,
{
    pub matrix: RowMajorMatrix<F>,
    pub domain: Domain,
    pub opening_index: usize,
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
            matrix: self.matrix.flatten_to_base(),
            domain: self.domain,
            opening_index: self.opening_index,
        }
    }
}

#[derive(Clone)]
pub struct ChipTrace<'a, Domain, EF>
where
    Domain: PolynomialSpace,
    EF: ExtensionField<Domain::Val>,
{
    pub chip: &'a ChipType,

    pub preprocessed: Option<Trace<Domain::Val, Domain>>,
    pub main: Option<Trace<Domain::Val, Domain>>,
    pub permutation: Option<Trace<EF, Domain>>,

    pub cumulative_sum: Option<EF>,

    pub quotient_chunks: Option<Trace<EF, Domain>>,
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
            quotient_chunks: None,
            cumulative_sum: None,
        }
    }

    // // 3. Calculate trace domains = max(preprocessed, main)
    // pub fn domain(&self) -> Domain {
    //     let trace_domains = pk
    //         .preprocessed_traces
    //         .iter()
    //         .zip_eq(main_traces.iter())
    //         .map(|traces| match traces {
    //             (Some(preprocessed_trace), Some(main_trace)) => {
    //                 let preprocessed_domain = preprocessed_trace.domain;
    //                 let main_domain = main_trace.domain;
    //                 if main_domain.size() > preprocessed_domain.size() {
    //                     Some(main_domain)
    //                 } else {
    //                     Some(preprocessed_domain)
    //                 }
    //             }
    //             (Some(preprocessed_trace), None) => Some(preprocessed_trace.domain),
    //             (None, Some(main_trace)) => Some(main_trace.domain),
    //             (None, None) => None,
    //         })
    //         .collect_vec();
    // }
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
    SC::Challenge: ExtensionField<Domain::Val>,
{
    fn load_preprocessed<P>(
        self,
        pcs: &P,
        traces: Vec<Option<RowMajorMatrix<Domain::Val>>>,
    ) -> Self
    where
        P: Pcs<SC::Challenge, SC::Challenger, Domain = Domain>,
        SC: StarkGenericConfig<Pcs = P>;

    fn load_main<P>(self, pcs: &P, traces: Vec<Option<RowMajorMatrix<Domain::Val>>>) -> Self
    where
        P: Pcs<SC::Challenge, SC::Challenger, Domain = Domain>,
        SC: StarkGenericConfig<Pcs = P>;

    fn generate_permutation<P, AB>(
        self,
        pcs: &P,
        perm_challenges: [SC::Challenge; NUM_PERM_CHALLENGES],
    ) -> Self
    where
        AB: InteractionAirBuilder<Expr = Domain::Val>,
        P: Pcs<SC::Challenge, SC::Challenger, Domain = Domain>,
        SC: StarkGenericConfig<Pcs = P>;
}

impl<'a, Domain, SC> MachineTraceLoader<'a, Domain, SC> for MachineTrace<'a, Domain, SC::Challenge>
where
    Domain: PolynomialSpace,
    SC: StarkGenericConfig,
    SC::Challenge: ExtensionField<Domain::Val>,
{
    fn load_preprocessed<P>(
        mut self,
        pcs: &P,
        traces: Vec<Option<RowMajorMatrix<Domain::Val>>>,
    ) -> Self
    where
        P: Pcs<SC::Challenge, SC::Challenger, Domain = Domain>,
        SC: StarkGenericConfig<Pcs = P>,
    {
        let traces = load_traces::<SC, _>(pcs, traces);
        for (chip_trace, preprocessed) in self.iter_mut().zip_eq(traces) {
            chip_trace.preprocessed = preprocessed;
        }
        self
    }

    fn load_main<P>(mut self, pcs: &P, traces: Vec<Option<RowMajorMatrix<Domain::Val>>>) -> Self
    where
        P: Pcs<SC::Challenge, SC::Challenger, Domain = Domain>,
        SC: StarkGenericConfig<Pcs = P>,
    {
        let traces = load_traces::<SC, _>(pcs, traces);
        for (chip_trace, main) in self.iter_mut().zip_eq(traces) {
            chip_trace.main = main;
        }
        self
    }

    fn generate_permutation<P, AB>(
        mut self,
        pcs: &P,
        perm_challenges: [SC::Challenge; NUM_PERM_CHALLENGES],
    ) -> Self
    where
        AB: InteractionAirBuilder<Expr = Domain::Val>,
        P: Pcs<SC::Challenge, SC::Challenger, Domain = Domain>,
        SC: StarkGenericConfig<Pcs = P>,
    {
        let traces = self
            .iter()
            .map(|trace| {
                let matrix = generate_permutation_trace(
                    &trace.preprocessed.as_ref().map(|mt| mt.matrix.as_view()),
                    &trace.main.as_ref().map(|mt| mt.matrix.as_view()),
                    <ChipType as InteractionAir<AB>>::all_interactions(trace.chip).as_slice(),
                    perm_challenges,
                );
                matrix
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
}

pub trait MachineTraceCommiter<'a, Domain, EF>
where
    Domain: PolynomialSpace,
    EF: ExtensionField<Domain::Val>,
{
    fn commit_preprocessed<SC>(self, pcs: &SC::Pcs) -> (Option<Com<SC>>, Option<PcsProverData<SC>>)
    where
        SC: StarkGenericConfig,
        SC::Pcs: Pcs<SC::Challenge, SC::Challenger, Domain = Domain>;

    fn commit_main<SC>(self, pcs: &SC::Pcs) -> (Option<Com<SC>>, Option<PcsProverData<SC>>)
    where
        SC: StarkGenericConfig,
        SC::Pcs: Pcs<SC::Challenge, SC::Challenger, Domain = Domain>;

    fn commit_permutation<SC>(self, pcs: &SC::Pcs) -> (Option<Com<SC>>, Option<PcsProverData<SC>>)
    where
        SC: StarkGenericConfig,
        SC::Pcs: Pcs<SC::Challenge, SC::Challenger, Domain = Domain>;
}

impl<'a, Domain, EF> MachineTraceCommiter<'a, Domain, EF> for MachineTrace<'a, Domain, EF>
where
    Domain: PolynomialSpace,
    EF: ExtensionField<Domain::Val>,
{
    fn commit_preprocessed<SC>(self, pcs: &SC::Pcs) -> (Option<Com<SC>>, Option<PcsProverData<SC>>)
    where
        SC: StarkGenericConfig,
        SC::Pcs: Pcs<SC::Challenge, SC::Challenger, Domain = Domain>,
    {
        let traces = self
            .into_iter()
            .map(|trace| trace.preprocessed)
            .collect_vec();
        commit_traces::<SC>(pcs, traces)
    }

    fn commit_main<SC>(self, pcs: &SC::Pcs) -> (Option<Com<SC>>, Option<PcsProverData<SC>>)
    where
        SC: StarkGenericConfig,
        SC::Pcs: Pcs<SC::Challenge, SC::Challenger, Domain = Domain>,
    {
        let traces = self.into_iter().map(|trace| trace.main).collect_vec();
        commit_traces::<SC>(pcs, traces)
    }

    fn commit_permutation<SC>(self, pcs: &SC::Pcs) -> (Option<Com<SC>>, Option<PcsProverData<SC>>)
    where
        SC: StarkGenericConfig,
        SC::Pcs: Pcs<SC::Challenge, SC::Challenger, Domain = Domain>,
    {
        let traces = self
            .into_iter()
            .map(|trace| trace.permutation.map(|trace| trace.flatten_to_base()))
            .collect_vec();
        commit_traces::<SC>(pcs, traces)
    }
}

fn load_traces<SC, F>(
    pcs: &SC::Pcs,
    traces: Vec<Option<RowMajorMatrix<F>>>,
) -> Vec<Option<Trace<F, Domain<SC>>>>
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
                        let index = *count;
                        *count += 1;

                        Some(Trace {
                            matrix: trace,
                            domain,
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
    traces: Vec<Option<Trace<Val<SC>, Domain<SC>>>>,
) -> (Option<Com<SC>>, Option<PcsProverData<SC>>)
where
    SC: StarkGenericConfig,
{
    let domains_and_traces: Vec<_> = traces
        .into_iter()
        .flat_map(|mt| mt.map(|trace| (trace.domain, trace.matrix)))
        .collect();
    if !domains_and_traces.is_empty() {
        let (commit, data) = pcs.commit(domains_and_traces);
        (Some(commit), Some(data))
    } else {
        (None, None)
    }
}
