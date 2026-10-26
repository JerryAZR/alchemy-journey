//! The effect IR and its pure planning half (DESIGN.md): flat instruction
//! vector + stateful sequential evaluator. Everything in this module is
//! pure — nothing here touches `CombatState`; the state-mutating half of
//! the evaluator (`apply_atom`, `resolve_spec`, `check_victory`) lives in
//! `engine.rs`, where `&mut CombatState` lives.

use crate::state::Card;

/// The atomic effect vocabulary, shared by potions and spells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Atom {
    Damage(u32),
    Block(u32),
}

/// A target *role*, resolved to concrete units at application time.
/// `Chosen` is the player's play-time pick — the default register value,
/// so `BindTarget(Chosen)` is also the way back from any other bind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetSpec {
    Chosen,
    Caster,
    AllEnemies,
    AllAllies,
    Everyone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Instruction {
    /// This cast applies up to N atoms (the default is 1). Potions lower
    /// with their exact atom count; "evaluate all" needs no special case.
    Eval(u32),
    /// Set the bind register: following atoms resolve against this role
    /// until another `BindTarget` replaces it.
    BindTarget(TargetSpec),
    /// Apply one atom to the resolved target.
    Apply(Atom),
}

/// Potion authoring type. The invariants (flat, single scope, predefined
/// order) are enforced here, at the authoring layer, not in the executor.
/// Single scope = one `TargetSpec` for the whole potion.
#[derive(Clone, Debug, PartialEq)]
pub struct PotionDef {
    pub target: TargetSpec,
    pub effects: Vec<Atom>,
}

impl PotionDef {
    /// Mechanical lowering into the IR (DESIGN.md). `Eval(k)` with the exact
    /// atom count makes the whole vector one cast; a non-`Chosen` target
    /// prepends a bind; `Chosen` needs none — it is the register's default.
    pub fn lower(&self) -> Vec<Instruction> {
        let mut out = vec![Instruction::Eval(self.effects.len() as u32)];
        if self.target != TargetSpec::Chosen {
            out.push(Instruction::BindTarget(self.target));
        }
        out.extend(self.effects.iter().copied().map(Instruction::Apply));
        out
    }
}

/// The plan of one cast, produced by pure simulation of the scan: the
/// atoms to apply with their bind roles folded in, where the cursor rests,
/// the bind register at rest, and whether any applied atom consumes the
/// chosen target. Shared by execution and the needs-target query so the
/// two can never diverge. Instruction-level bookkeeping (`Eval`,
/// `BindTarget`) is fully resolved here and never reaches the executor.
pub(crate) struct CastPlan {
    pub(crate) casts: Vec<(Atom, TargetSpec)>,
    pub(crate) resting_cursor: usize,
    pub(crate) final_bind: TargetSpec,
    pub(crate) needs_chosen: bool,
}

/// Simulates one cast of `card` without touching any state (DESIGN.md:
/// paused-stepping evaluation with grouping semantics).
///
/// Blocks and quotas: the unit of evaluation is the *block* — an atom, or
/// a group built by `Eval(N)` ("the next N blocks become one block"). A
/// cast consumes one root block. The scan tracks a quota stack: drawing an
/// atom or an `Eval` spends one block of the innermost quota; `Eval` then
/// pushes its own quota, which is how groups nest multiplicatively.
/// Register instructions (`BindTarget`, and later modifiers) ride free —
/// they are not blocks and never spend quota. A spent quota pops; when the
/// stack empties, the cast ends immediately.
///
/// The scan stops at the vector end unless the wrap budget (`def.unwrap`)
/// lets it pass slot 0; reaching the end rests the cursor at 0, which is
/// what makes the next cast cycle.
///
/// Termination: each iteration consumes one instruction index and advances
/// the cursor; each lap covers at most `len` indices and there are at most
/// `unwrap + 1` laps. Exceeding that bound is a logic bug, so it panics —
/// a test failure, never a hang.
pub(crate) fn plan_cast(card: &Card) -> CastPlan {
    let effects = &card.def.effects;
    let mut plan = CastPlan {
        casts: Vec::new(),
        resting_cursor: 0,
        final_bind: card.bind,
        needs_chosen: false,
    };
    let mut quotas: Vec<u32> = vec![1];
    let mut wraps_left = card.def.unwrap;
    let max_iterations = (card.def.unwrap as usize + 1) * effects.len();
    let mut iterations = 0;
    let mut i = card.cursor;
    loop {
        if quotas.is_empty() {
            break; // root block complete: cursor rests on the next instruction
        }
        if i == effects.len() {
            if wraps_left == 0 {
                break; // vector end: cast stops; cursor rests at 0
            }
            wraps_left -= 1;
            i = 0;
            continue;
        }
        iterations += 1;
        assert!(
            iterations <= max_iterations,
            "plan_cast exceeded its iteration bound ({max_iterations})"
        );
        match effects[i] {
            Instruction::Eval(n) => {
                // A group is one block of its parent, then pushes its own quota.
                spend_block(&mut quotas);
                quotas.push(n);
            }
            Instruction::BindTarget(spec) => {
                plan.final_bind = spec; // rides free: not a block
            }
            Instruction::Apply(atom) => {
                spend_block(&mut quotas);
                if plan.final_bind == TargetSpec::Chosen {
                    plan.needs_chosen = true;
                }
                plan.casts.push((atom, plan.final_bind));
            }
        }
        i += 1;
    }
    plan.resting_cursor = if i == effects.len() { 0 } else { i };
    plan
}

/// Spends one block from the innermost quota and pops spent groups.
fn spend_block(quotas: &mut Vec<u32>) {
    *quotas.last_mut().expect("a drawn block always has a quota") -= 1;
    while quotas.last() == Some(&0) {
        quotas.pop();
    }
}

/// Does the upcoming cast of this card consume the chosen target? A pure
/// query the UI builds its targeting flow on (DESIGN.md). Accepts the
/// card's current registers; `chosen` validation happens in `validate`.
pub fn cast_needs_target(card: &Card) -> bool {
    plan_cast(card).needs_chosen
}
