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
/// Blocks and budgets: the unit of evaluation is the *block* — an atom, or
/// a group built by `Eval(N)` ("the next N blocks become one block"). A
/// cast consumes one root block. Conceptually the scan keeps a quota stack
/// (drawing an atom or an `Eval` spends one block of the innermost quota,
/// `Eval` pushes its own quota, spent quotas pop, and the cast ends the
/// moment the stack empties). Since spending is always innermost and
/// quotas commute, the stack's *sum* is an equivalent single counter —
/// that is what `budget` is. (When block-scoped modifiers are designed,
/// the frame representation is decided then; the counter forecloses
/// nothing.)
///
/// All scan-termination policy — the block budget *and* the wrap limit —
/// is the single break condition below. [`ScanCursor`] is a policy-free
/// walker: it advances (wrapping past the vector end, counting laps) and
/// reports its position; when the loop breaks, that position *is* the
/// resting cursor — the next unconsumed cell, or 0 after a wrap, with no
/// normalization rule.
pub(crate) fn plan_cast(card: &Card) -> CastPlan {
    let effects = &card.def.effects;
    let mut plan = CastPlan {
        casts: Vec::new(),
        resting_cursor: 0,
        final_bind: card.bind,
        needs_chosen: false,
    };
    if effects.is_empty() {
        return plan;
    }
    let mut scan = ScanCursor::new(card.cursor, effects.len());
    let mut budget: u32 = 1;
    loop {
        let i = scan.position();
        match effects[i] {
            Instruction::Eval(n) => {
                // A group spends one block of its parent, then adds its own.
                budget = budget - 1 + n;
            }
            Instruction::BindTarget(spec) => {
                plan.final_bind = spec; // rides free: not a block
            }
            Instruction::Apply(atom) => {
                budget -= 1;
                if plan.final_bind == TargetSpec::Chosen {
                    plan.needs_chosen = true;
                }
                plan.casts.push((atom, plan.final_bind));
            }
        }
        scan.advance();
        if budget == 0 || scan.wraps() > u32::from(card.def.unwrap) {
            break;
        }
    }
    plan.resting_cursor = scan.position();
    plan
}

/// A policy-free walker over the effect vector. `advance` moves to the
/// next cell (wrapping past the end) and counts **laps**: how often the
/// walk has returned to the cell the scan started from. (Laps are counted
/// relative to the start, not slot 0: a sparse wand's cast must reach
/// cells *before* its start without spending wrap budget — otherwise
/// content there starves and every other cast misfires.) `position` is
/// always the current cell; `wraps` is the lap count. Finite scans are the
/// *caller's* policy (budget and wrap limit). When `Empty` slots land,
/// gap-skipping goes in `advance` — and the resting position automatically
/// becomes the next non-empty cell (DESIGN.md, "Sparse vectors").
struct ScanCursor {
    len: usize,
    pos: usize,
    /// The cell this scan started from: advancing onto it completes a lap.
    initial: usize,
    wraps: u32,
}

impl ScanCursor {
    /// Starts at `start`. `len` must be non-zero.
    fn new(start: usize, len: usize) -> Self {
        Self {
            len,
            pos: start,
            initial: start,
            wraps: 0,
        }
    }

    fn position(&self) -> usize {
        self.pos
    }

    fn wraps(&self) -> u32 {
        self.wraps
    }

    fn advance(&mut self) {
        self.pos += 1;
        if self.pos == self.len {
            self.pos = 0;
        }
        if self.pos == self.initial {
            self.wraps += 1;
        }
    }
}

/// Does the upcoming cast of this card consume the chosen target? A pure
/// query the UI builds its targeting flow on (DESIGN.md). Accepts the
/// card's current registers; `chosen` validation happens in `validate`.
pub fn cast_needs_target(card: &Card) -> bool {
    plan_cast(card).needs_chosen
}
