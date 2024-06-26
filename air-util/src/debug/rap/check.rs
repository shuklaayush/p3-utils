use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;
use core::borrow::Borrow;

use p3_field::{ExtensionField, Field};
use p3_interaction::{Bus, InteractionType, Rap, NUM_PERM_CHALLENGES};
use p3_matrix::dense::RowMajorMatrixView;
use p3_matrix::stack::VerticalPair;
use p3_matrix::Matrix;
use p3_maybe_rayon::prelude::IntoParallelIterator;

use crate::folders::rap::DebugConstraintBuilder;

/// Check that all constraints vanish on the subgroup.
pub fn check_constraints<F, EF, A>(
    air: &A,
    preprocessed: &Option<RowMajorMatrixView<F>>,
    main: &Option<RowMajorMatrixView<F>>,
    perm: &Option<RowMajorMatrixView<EF>>,
    perm_challenges: [EF; NUM_PERM_CHALLENGES],
    cumulative_sum: Option<EF>,
    public_values: &[F],
) where
    F: Field,
    EF: ExtensionField<F>,
    A: for<'a> Rap<DebugConstraintBuilder<'a, F, EF>>,
{
    let height = match (main.as_ref(), preprocessed.as_ref()) {
        (Some(main), Some(preprocessed)) => core::cmp::max(main.height(), preprocessed.height()),
        (Some(main), None) => main.height(),
        (None, Some(preprocessed)) => preprocessed.height(),
        (None, None) => 0,
    };

    if let Some(perm) = perm {
        assert_eq!(perm.height(), height);
    }

    // Check that constraints are satisfied.
    (0..height).into_par_iter().for_each(|i| {
        let i_next = (i + 1) % height;

        let (preprocessed_local, preprocessed_next) = preprocessed
            .as_ref()
            .map(|preprocessed| {
                (
                    preprocessed.row_slice(i).to_vec(),
                    preprocessed.row_slice(i_next).to_vec(),
                )
            })
            .unwrap_or((vec![], vec![]));
        let (main_local, main_next) = main
            .as_ref()
            .map(|main| (main.row_slice(i).to_vec(), main.row_slice(i_next).to_vec()))
            .unwrap_or((vec![], vec![]));
        let (perm_local, perm_next) = perm
            .as_ref()
            .map(|perm| (perm.row_slice(i).to_vec(), perm.row_slice(i_next).to_vec()))
            .unwrap_or((vec![], vec![]));

        let mut builder = DebugConstraintBuilder {
            row_index: i,
            preprocessed: VerticalPair::new(
                RowMajorMatrixView::new_row(preprocessed_local.as_slice()),
                RowMajorMatrixView::new_row(preprocessed_next.as_slice()),
            ),
            main: VerticalPair::new(
                RowMajorMatrixView::new_row(&*main_local),
                RowMajorMatrixView::new_row(&*main_next),
            ),
            permutation: VerticalPair::new(
                RowMajorMatrixView::new_row(perm_local.as_slice()),
                RowMajorMatrixView::new_row(perm_next.as_slice()),
            ),
            perm_challenges,
            public_values,
            cumulative_sum: cumulative_sum.unwrap_or_default(),
            is_first_row: F::zero(),
            is_last_row: F::zero(),
            is_transition: F::one(),
        };
        if i == 0 {
            builder.is_first_row = F::one();
        }
        if i == height - 1 {
            builder.is_last_row = F::one();
            builder.is_transition = F::zero();
        }

        air.eval_all(&mut builder);
    });
}

// TODO: Check number of virtual columns in bus are same
pub fn check_cumulative_sums<F, EF, A, B>(
    airs: &[A],
    preprocessed: &[Option<RowMajorMatrixView<F>>],
    main: &[Option<RowMajorMatrixView<F>>],
    permutation: &[Option<RowMajorMatrixView<EF>>],
) where
    F: Field,
    EF: ExtensionField<F>,
    A: for<'a> Rap<DebugConstraintBuilder<'a, F, EF>>,
    B: Bus,
{
    let mut sums = BTreeMap::new();
    for (i, air) in airs.iter().enumerate() {
        for (j, (interaction, interaction_type)) in air.all_interactions().iter().enumerate() {
            if let Some(permutation) = permutation[i].as_ref() {
                for (n, perm_row) in permutation.rows().enumerate() {
                    let preprocessed_row = preprocessed[i]
                        .as_ref()
                        .map(|preprocessed| {
                            let row = preprocessed.row_slice(n);
                            let row: &[_] = (*row).borrow();
                            row.to_vec()
                        })
                        .unwrap_or_default();
                    let main_row = main[i]
                        .as_ref()
                        .map(|main| {
                            let row = main.row_slice(n);
                            let row: &[_] = (*row).borrow();
                            row.to_vec()
                        })
                        .unwrap_or_default();
                    let perm_row: Vec<_> = perm_row.collect();
                    let mult = interaction
                        .count
                        .apply::<F, F>(preprocessed_row.as_slice(), main_row.as_slice());
                    let val = match interaction_type {
                        InteractionType::Send => perm_row[j] * mult,
                        InteractionType::Receive => -perm_row[j] * mult,
                    };
                    sums.entry(interaction.argument_index)
                        .and_modify(|c| *c += val)
                        .or_insert(val);
                }
            }
        }
    }
    for (i, sum) in sums {
        assert_eq!(
            sum,
            EF::zero(),
            "{} bus cumulative sum is not zero",
            B::from(i)
        );
    }

    // Check cumulative sums
    let sum: EF = permutation
        .iter()
        .flatten()
        .map(|perm| *perm.row_slice(perm.height() - 1).last().unwrap())
        .sum();
    assert_eq!(sum, EF::zero());
}
