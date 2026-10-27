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

/// One node in a card's program. Cost is a property of the *node*
/// (DESIGN.md): a component's price travels with it, not with the wand —
/// a 6-damage component costs 1 wherever it is, and Eval/BindTarget are
/// components too. A cast costs the sum of the nodes it consumes, so
/// stepping wands price each cast by what it actually does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Instruction {
    pub cost: u32,
    pub kind: InstructionKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstructionKind {
    /// The next N blocks become one block (grouping).
    Eval(u32),
    /// Set the bind register: following atoms resolve against this role
    /// until another `BindTarget` replaces it.
    BindTarget(TargetSpec),
    /// Apply one atom to the resolved target.
    Apply(Atom),
}

impl Instruction {
    pub fn eval(n: u32, cost: u32) -> Self {
        Self {
            cost,
            kind: InstructionKind::Eval(n),
        }
    }

    pub fn bind(spec: TargetSpec, cost: u32) -> Self {
        Self {
            cost,
            kind: InstructionKind::BindTarget(spec),
        }
    }

    pub fn apply(atom: Atom, cost: u32) -> Self {
        Self {
            cost,
            kind: InstructionKind::Apply(atom),
        }
    }
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
    /// Potions are zero-cost by rule: every node lowers with cost 0 (their
    /// price is the run-level resource, not energy).
    pub fn lower(&self) -> Vec<Instruction> {
        let mut out = vec![Instruction::eval(self.effects.len() as u32, 0)];
        if self.target != TargetSpec::Chosen {
            out.push(Instruction::bind(self.target, 0));
        }
        out.extend(self.effects.iter().map(|&a| Instruction::apply(a, 0)));
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
    /// Energy the cast charges: the sum of the costs of the nodes it
    /// consumes. Per-cast pricing (DESIGN.md): a stepping wand's casts
    /// each cost what they do.
    pub(crate) cost: u32,
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
        cost: 0,
    };
    if effects.is_empty() {
        return plan;
    }
    let mut scan = ScanCursor::new(card.cursor, effects.len());
    let mut budget: u32 = 1;
    loop {
        let i = scan.position();
        let instruction = effects[i];
        plan.cost += instruction.cost;
        match instruction.kind {
            InstructionKind::Eval(n) => {
                // A group spends one block of its parent, then adds its own.
                budget = budget - 1 + n;
            }
            InstructionKind::BindTarget(spec) => {
                plan.final_bind = spec; // rides free: not a block
            }
            InstructionKind::Apply(atom) => {
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

/// What the upcoming cast of this card costs. A pure query the UI displays
/// on the card face, next to the cursor bracket.
pub fn cast_cost(card: &Card) -> u32 {
    plan_cast(card).cost
}
