use p3_air::{
    AirBuilder, AirBuilderWithPublicValues, ExtensionBuilder, PairBuilder, PermutationAirBuilder,
};
use p3_field::AbstractField;
use p3_interaction::{InteractionAirBuilder, NUM_PERM_CHALLENGES};
use p3_uni_stark::{StarkGenericConfig, Val};

use crate::folders::ViewPair;

pub struct VerifierConstraintFolder<'a, SC: StarkGenericConfig> {
    pub preprocessed: ViewPair<'a, SC::Challenge>,
    pub main: ViewPair<'a, SC::Challenge>,
    pub perm: ViewPair<'a, SC::Challenge>,
    pub perm_challenges: [SC::Challenge; NUM_PERM_CHALLENGES],
    pub public_values: &'a [Val<SC>],
    pub cumulative_sum: SC::Challenge,
    pub is_first_row: SC::Challenge,
    pub is_last_row: SC::Challenge,
    pub is_transition: SC::Challenge,
    pub alpha: SC::Challenge,
    pub accumulator: SC::Challenge,
}

impl<'a, SC: StarkGenericConfig> AirBuilder for VerifierConstraintFolder<'a, SC> {
    type F = Val<SC>;
    type Expr = SC::Challenge;
    type Var = SC::Challenge;
    type M = ViewPair<'a, SC::Challenge>;

    fn main(&self) -> Self::M {
        self.main
    }

    fn is_first_row(&self) -> Self::Expr {
        self.is_first_row
    }

    fn is_last_row(&self) -> Self::Expr {
        self.is_last_row
    }

    fn is_transition_window(&self, size: usize) -> Self::Expr {
        if size == 2 {
            self.is_transition
        } else {
            panic!("uni-stark only supports a window size of 2")
        }
    }

    fn assert_zero<I: Into<Self::Expr>>(&mut self, x: I) {
        let x: SC::Challenge = x.into();
        self.accumulator *= self.alpha;
        self.accumulator += x;
    }
}

impl<'a, SC> PairBuilder for VerifierConstraintFolder<'a, SC>
where
    SC: StarkGenericConfig,
{
    fn preprocessed(&self) -> Self::M {
        self.preprocessed
    }
}

impl<'a, SC: StarkGenericConfig> AirBuilderWithPublicValues for VerifierConstraintFolder<'a, SC> {
    type PublicVar = Self::F;

    fn public_values(&self) -> &[Self::PublicVar] {
        self.public_values
    }
}

impl<'a, SC> ExtensionBuilder for VerifierConstraintFolder<'a, SC>
where
    SC: StarkGenericConfig,
{
    type EF = SC::Challenge;
    type ExprEF = SC::Challenge;
    type VarEF = SC::Challenge;

    fn assert_zero_ext<I>(&mut self, x: I)
    where
        I: Into<Self::ExprEF>,
    {
        let x: SC::Challenge = x.into();
        self.accumulator *= SC::Challenge::from_f(self.alpha);
        self.accumulator += x;
    }
}

impl<'a, SC> PermutationAirBuilder for VerifierConstraintFolder<'a, SC>
where
    SC: StarkGenericConfig,
{
    type MP = ViewPair<'a, SC::Challenge>;

    type RandomVar = SC::Challenge;

    fn permutation(&self) -> Self::MP {
        self.perm
    }

    fn permutation_randomness(&self) -> &[Self::RandomVar] {
        &self.perm_challenges
    }
}

impl<'a, SC: StarkGenericConfig> InteractionAirBuilder for VerifierConstraintFolder<'a, SC> {
    fn cumulative_sum(&self) -> Self::RandomVar {
        self.cumulative_sum
    }
}
