use p3_air::{Air, PairBuilder, PermutationAirBuilder, VirtualPairCol};
use p3_air::{ExtensionBuilder, TwoRowMatrixView};
use p3_field::{AbstractField, ExtensionField, Field, Powers};
use p3_matrix::dense::RowMajorMatrix;
use p3_matrix::{Matrix, MatrixRowSlices};
use p3_maybe_rayon::prelude::IntoParallelIterator;
use p3_uni_stark::{StarkGenericConfig, SymbolicAirBuilder, Val};

use crate::debug_builder::DebugConstraintBuilder;
use crate::folder::ProverConstraintFolder;
use crate::util::batch_multiplicative_inverse_allowing_zero;

#[derive(Clone, Debug)]
pub enum InteractionType {
    Send,
    Receive,
}

#[derive(Clone, Debug)]
pub struct Interaction<F: Field> {
    pub fields: Vec<VirtualPairCol<F>>,
    pub count: VirtualPairCol<F>,
    pub argument_index: usize,
}

pub trait Chip<F: Field> {
    fn generate_trace(&self) -> RowMajorMatrix<F>;

    fn sends(&self) -> Vec<Interaction<F>> {
        vec![]
    }

    fn receives(&self) -> Vec<Interaction<F>> {
        vec![]
    }

    fn all_interactions(&self) -> Vec<(Interaction<F>, InteractionType)> {
        let mut interactions: Vec<(Interaction<F>, InteractionType)> = vec![];
        interactions.extend(self.sends().into_iter().map(|i| (i, InteractionType::Send)));
        interactions.extend(
            self.receives()
                .into_iter()
                .map(|i| (i, InteractionType::Receive)),
        );
        interactions
    }
}

pub trait MachineChip<SC: StarkGenericConfig>: Chip<Val<SC>> + for<'a> Air<ProverConstraintFolder<'a, SC>>
    // + for<'a> Air<VerifierConstraintFolder<'a, SC>>
    + for<'a> Air<SymbolicAirBuilder<Val<SC>>>
    + for<'a> Air<DebugConstraintBuilder<'a, SC>>
{
    fn trace_width(&self) -> usize {
        self.width()
    }
}

/// Generate the permutation trace for a chip with the provided machine.
/// This is called only after `generate_trace` has been called on all chips.
pub fn generate_permutation_trace<SC: StarkGenericConfig, C: MachineChip<SC>>(
    chip: &C,
    main: &RowMajorMatrix<Val<SC>>,
    random_elements: Vec<SC::Challenge>,
) -> RowMajorMatrix<SC::Challenge> {
    let all_interactions = chip.all_interactions();
    let alphas = generate_rlc_elements(chip, &random_elements);
    let betas = random_elements[1].powers();

    let preprocessed = chip.preprocessed_trace();

    // Compute the reciprocal columns
    //
    // Row: | q_1 | q_2 | q_3 | ... | q_n | \phi |
    // * q_i = \frac{1}{\alpha^i + \sum_j \beta^j * f_{i,j}}
    // * f_{i,j} is the jth main trace column for the ith interaction
    // * \phi is the running sum
    //
    // Note: We can optimize this by combining several reciprocal columns into one (the
    // number is subject to a target constraint degree).
    let perm_width = all_interactions.len() + 1;
    let mut perm_values = Vec::with_capacity(main.height() * perm_width);

    for (n, main_row) in main.rows().enumerate() {
        let mut row = vec![SC::Challenge::zero(); perm_width];
        for (m, (interaction, _)) in all_interactions.iter().enumerate() {
            let alpha_m = alphas[interaction.argument_index];
            let preprocessed_row = if preprocessed.is_some() {
                preprocessed.as_ref().unwrap().row_slice(n)
            } else {
                &[]
            };
            row[m] = reduce_row(
                main_row,
                preprocessed_row,
                &interaction.fields,
                alpha_m,
                betas.clone(),
            );
        }
        perm_values.extend(row);
    }
    // TODO: Switch to batch_multiplicative_inverse (not allowing zero)?
    // Zero should be vanishingly unlikely if properly randomized?
    let perm_values = batch_multiplicative_inverse_allowing_zero(perm_values);
    let mut perm = RowMajorMatrix::new(perm_values, perm_width);

    // Compute the running sum column
    let mut phi = vec![SC::Challenge::zero(); perm.height()];
    for (n, (main_row, perm_row)) in main.rows().zip(perm.rows()).enumerate() {
        if n > 0 {
            phi[n] = phi[n - 1];
        }
        let preprocessed_row = if preprocessed.is_some() {
            preprocessed.as_ref().unwrap().row_slice(n)
        } else {
            &[]
        };
        for (m, (interaction, interaction_type)) in all_interactions.iter().enumerate() {
            let mult = interaction
                .count
                .apply::<Val<SC>, Val<SC>>(preprocessed_row, main_row);
            match interaction_type {
                InteractionType::Send => {
                    phi[n] += perm_row[m] * mult;
                }
                InteractionType::Receive => {
                    phi[n] -= perm_row[m] * mult;
                }
            }
        }
    }

    for (n, row) in perm.as_view_mut().rows_mut().enumerate() {
        *row.last_mut().unwrap() = phi[n];
    }

    perm
}

pub fn eval_permutation_constraints<C, SC, AB>(chip: &C, builder: &mut AB, cumulative_sum: AB::EF)
where
    C: MachineChip<SC>,
    SC: StarkGenericConfig,
    AB: PairBuilder<F = Val<SC>> + PermutationAirBuilder<F = Val<SC>, EF = SC::Challenge>,
{
    let rand_elems = builder.permutation_randomness().to_vec();

    let main = builder.main();
    let main_local: &[AB::Var] = main.row_slice(0);
    let main_next: &[AB::Var] = main.row_slice(1);

    let preprocessed = builder.preprocessed();
    let preprocessed_local = preprocessed.row_slice(0);
    let preprocessed_next = preprocessed.row_slice(1);

    let perm = builder.permutation();
    let perm_width = perm.width();
    let perm_local: &[AB::VarEF] = perm.row_slice(0);
    let perm_next: &[AB::VarEF] = perm.row_slice(1);

    let phi_local = perm_local[perm_width - 1];
    let phi_next = perm_next[perm_width - 1];

    let all_interactions = chip.all_interactions();

    let alphas = generate_rlc_elements(chip, &rand_elems);
    let betas = rand_elems[1].powers();

    let lhs = phi_next.into() - phi_local.into();
    let mut rhs = AB::ExprEF::zero();
    let mut phi_0 = AB::ExprEF::zero();
    for (m, (interaction, interaction_type)) in all_interactions.iter().enumerate() {
        // Reciprocal constraints
        let mut rlc = AB::ExprEF::zero();
        for (field, beta) in interaction.fields.iter().zip(betas.clone()) {
            let elem = field.apply::<AB::Expr, AB::Var>(preprocessed_local, main_local);
            rlc += AB::ExprEF::from_f(beta) * elem;
        }
        rlc += AB::ExprEF::from_f(alphas[interaction.argument_index]);
        println!("rlc {:?}", rlc.clone() * perm_local[m].into());
        builder.assert_one_ext(rlc * perm_local[m].into());

        let mult_local = interaction
            .count
            .apply::<AB::Expr, AB::Var>(preprocessed_local, main_local);
        let mult_next = interaction
            .count
            .apply::<AB::Expr, AB::Var>(preprocessed_next, main_next);

        // Build the RHS of the permutation constraint
        match interaction_type {
            InteractionType::Send => {
                phi_0 += perm_local[m].into() * mult_local;
                rhs += perm_next[m].into() * mult_next;
            }
            InteractionType::Receive => {
                phi_0 -= perm_local[m].into() * mult_local;
                rhs -= perm_next[m].into() * mult_next;
            }
        }
    }

    // Running sum constraints
    builder.when_transition().assert_eq_ext(lhs, rhs);
    builder
        .when_first_row()
        .assert_eq_ext(*perm_local.last().unwrap(), phi_0);
    builder.when_last_row().assert_eq_ext(
        *perm_local.last().unwrap(),
        AB::ExprEF::from_f(cumulative_sum),
    );
}

fn generate_rlc_elements<SC: StarkGenericConfig, C: MachineChip<SC>>(
    chip: &C,
    random_elements: &[SC::Challenge],
) -> Vec<SC::Challenge> {
    random_elements[0]
        .powers()
        .skip(1)
        .take(
            chip.sends()
                .into_iter()
                .chain(chip.receives())
                .map(|interaction| interaction.argument_index)
                .max()
                .unwrap_or(0)
                + 1,
        )
        .collect::<Vec<_>>()
}

// TODO: Use Var and Expr type bounds in place of concrete fields so that
// this function can be used in `eval_permutation_constraints`.
fn reduce_row<F, EF>(
    main_row: &[F],
    preprocessed_row: &[F],
    fields: &[VirtualPairCol<F>],
    alpha: EF,
    betas: Powers<EF>,
) -> EF
where
    F: Field,
    EF: ExtensionField<F>,
{
    let mut rlc = EF::zero();
    for (columns, beta) in fields.iter().zip(betas) {
        rlc += beta * columns.apply::<F, F>(preprocessed_row, main_row)
    }
    rlc += alpha;
    rlc
}

/// Check that all constraints vanish on the subgroup.
pub fn check_constraints<C, SC>(
    chip: &C,
    main: &RowMajorMatrix<Val<SC>>,
    perm: &RowMajorMatrix<SC::Challenge>,
    perm_challenges: &[SC::Challenge],
    public_values: &Vec<Val<SC>>,
) where
    C: MachineChip<SC>,
    SC: StarkGenericConfig,
{
    assert_eq!(main.height(), perm.height());
    let height = main.height();
    if height == 0 {
        return;
    }

    let preprocessed = chip.preprocessed_trace();

    let cumulative_sum = *perm.row_slice(perm.height() - 1).last().unwrap();

    // Check that constraints are satisfied.
    (0..height).into_par_iter().for_each(|i| {
        let i_next = (i + 1) % height;

        let main_local = main.row_slice(i);
        let main_next = main.row_slice(i_next);
        let preprocessed_local = if preprocessed.is_some() {
            preprocessed.as_ref().unwrap().row_slice(i)
        } else {
            &[]
        };
        let preprocessed_next = if preprocessed.is_some() {
            preprocessed.as_ref().unwrap().row_slice(i_next)
        } else {
            &[]
        };
        let perm_local = perm.row_slice(i);
        let perm_next = perm.row_slice(i_next);

        let mut builder = DebugConstraintBuilder {
            row_index: i,
            main: TwoRowMatrixView {
                local: &main_local,
                next: &main_next,
            },
            preprocessed: TwoRowMatrixView {
                local: &preprocessed_local,
                next: &preprocessed_next,
            },
            perm: TwoRowMatrixView {
                local: &perm_local,
                next: &perm_next,
            },
            perm_challenges,
            public_values,
            is_first_row: Val::<SC>::zero(),
            is_last_row: Val::<SC>::zero(),
            is_transition: Val::<SC>::one(),
        };
        if i == 0 {
            builder.is_first_row = Val::<SC>::one();
        }
        if i == height - 1 {
            builder.is_last_row = Val::<SC>::one();
            builder.is_transition = Val::<SC>::zero();
        }

        chip.eval(&mut builder);
        eval_permutation_constraints(chip, &mut builder, cumulative_sum);
    });
}

/// Check that the combined cumulative sum across all lookup tables is zero.
pub fn check_cumulative_sums<Challenge: Field>(perms: &[RowMajorMatrix<Challenge>]) {
    let sum: Challenge = perms
        .iter()
        .map(|perm| *perm.row_slice(perm.height() - 1).last().unwrap())
        .sum();
    assert_eq!(sum, Challenge::zero());
}
