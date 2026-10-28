# Devlog 003 — Step 2: Effects, and the Evaluator That Got Reviewed Into Shape

*A series on building a Slay-the-Spire-like deckbuilder (with Noita-style spell
assembly) in Bevy, designed and written in collaboration with an AI assistant.*

## What Step 2 is

Cards that do things:

- Damage and block, lethal damage, a terminal victory state
- Targeting: click a card to arm it, click a living enemy to fire
- **Stepping wands**: a card is a little program that evaluates one *block*
  per play, with its cursor and registers persisting between plays. The
  starter Strike Draught cycles 6-9-15 damage (costing 1-1-2 energy);
  the Ward Draught cycles 5-12-25 block (costing 1-2-3)
- Potions lowered from an authoring type (`PotionDef { target, effects }`)
  into the same instruction vector spells will use
- Per-cast energy pricing computed from the card's program

Underneath: the effect IR and its evaluator — the piece this game will
ultimately stand or fall with, since spell assembly compiles down to it.
This devlog is the story of that evaluator, because almost everything about
its final shape came from the human reviewing the AI's code and finding it
wanting.

## The tree we didn't build

The AI's first proposal was a tree IR: `Atom | Sequence | WithTarget`, with a
tree-walking interpreter. Textbook compiler design. The human demolished it
with one example:

> Suppose two "gain 5 shield" nodes in slots 1 and 3. I add a "double the next
> shield gain" node in slot 2. What happens? Slots 1 and 3 are no longer
> siblings. [...] You'd still do DFS traversal, right? What's the resulting
> order? Isn't it just the vector scan order?

It is. The tree was *derived data* — recompiled from the vector after every
edit, then flattened right back into scan order by the walk. The editing
surface is a vector; the IR should mirror the authoring model. Final design:
a flat instruction vector (`Eval` / `BindTarget` / `Apply`) plus a stateful
sequential evaluator with registers (cursor, bind, budget) that pause on the
card instance between casts. No tree, no compiler, one evaluation path.

This was the second time the human had to talk the AI out of an imported
pattern (the first was event-sourcing, in devlog 002). Compiler IRs are for
compiler problems.

## "Predictable" is not "static"

The AI also argued against Noita's per-cast stepping on genre grounds: cards
in a deckbuilder are commitments, "what does this card do?" needs a static
answer. The human's correction was precise: the planning loop needs a
*predictable* answer, not a static one — and even that's too strong, since
StS cards literally roll dice. A visible cursor cycling deterministically is
predictable. Per-card state is fun (StS is full of it). And potions don't
become an exception: they just lower with `Eval(k)` covering their whole
vector, same engine.

The stepping model is now the heart of the game's texture, and it's better
than what the AI was defending.

## The targeting saga, or: don't rubber-stamp

Targeting went through four designs, and the embarrassing part is the middle
two. The AI proposed bind-instructions only. The human counter-proposed a
three-rule resolution (node's built-in target > active bind > chosen target).
The AI... immediately "adopted" it, without evaluation. The human called it
out: *"You didn't explain why you adopted mine. You didn't evaluate the pros
& cons."*

Forced to actually compare, the AI then overcorrected into "built-in targets
only, no binds" — and justified it with a wrong principle ("features that
make expansion free delete tradeoffs and are rejected"). The human again:
built-in targets aren't free expansion, they're a *flexibility-for-slot-
efficiency* tradeoff, and pricing expansion through slots is the whole game
in a wand-builder. Final answer: bind-or-chosen now, built-ins parked as a
deliberate future lever.

Later, the human spotted that `Chosen` was awkwardly special-cased (an
`Option<TargetSpec>` register plus a `ClearBind` instruction), and that the
potion `Scope` enum and the IR's `TargetSpec` were the same concept written
twice. Unifying them deleted an instruction, a type, a wrapper, and a
resolution rule. The duplication existed because the two vocabularies were
designed in separate conversations and never diffed.

## The Eval bug, red test first

The human's review of the evaluator found a live bug: in
`Damage(1) - Eval(2) - Damage(1) - Damage(1)`, the first cast dealt 3. The
scan's break check only ran at `Apply` instructions, so it walked past its
spent budget into the `Eval(2)`, which *retroactively* reset the budget.

The mandate came with the bug report: **a red test first.** The failing test
went in, showed `left: 28, right: 29`, and only then the fix.

The fix surfaced a design gap neither side had aligned on: what *is*
`Eval(N)`? The AI had implemented "reset this cast's budget." The human meant
Noita's multicast: "the next N blocks become one block," nesting
multiplicatively — `Eval(2)[Eval(2)[D,D], Eval(2)[D,D]]` deals 4. Grouping
semantics, implemented with a quota stack in the flat scan. No tree required;
the stack is the tree's ghost.

Then the stack itself got reviewed away: quota spending is always innermost
and quotas commute, so `Vec<u32>` is equivalent to a single counter. Keeping
the stack "for future modifiers" was speculative structure — the frame shape
gets designed when modifiers are designed. Then the loop arithmetic got
unified into a policy-free walker (`ScanCursor`: advance, position, wrap
count — all termination policy in one break condition), and the wrap rule
moved from "passing slot 0" to "returning to the cast's starting cell,"
because the former made sparse wands starve: content before the cursor could
never be reached, and an 8-slot wand with one spell would misfire every
other cast.

Each of these is a paragraph because each was a real defect or a real design
decision. The evaluator that survives is ~80 lines and has been unusually
adversarially reviewed.

## Cost moves to the nodes

Last design change: energy cost. The AI had a per-card cost field. The
human's objection: with stepping wands, casts can be wildly different — what
fixed cost is "fair" for a card that alternates a poke and a finisher?
There isn't one. Noita's answer is mana per cast, and our plan-computation
already knows what each cast consumes, so cost moved onto instruction nodes
(authored per component — a balance lever, not a formula) and a cast charges
the sum of the nodes it consumes. `CardDef.cost` no longer exists; the card
face shows the *next cast's* cost.

One recorded limitation: cost is plan-time and state-free, so it can't
depend on the target. StS's "if target satisfies X, recover Y energy" is
expressible as an execution-time refund effect instead — charge-then-refund,
not dynamic pricing. Known, deliberate, written down.

## Process lessons from this round

1. **The doc system worked.** DESIGN.md was amended *before* code at every
   decision point; when the AI rubber-stamped or smuggled a wrong rationale
   into the docs, the human caught it in the diff. "Unapproved designs get
   reverted" never had to trigger — discussion caught everything first.
2. **Red test first is worth mandating.** The Eval bug got a failing test
   before a fix; the fix's semantics (grouping) got three example sequences
   as named specification tests. The suite is now the semantics document.
3. **"Don't cover impossible scenarios in the normal path."** `i >= len`
   when only `==` is reachable; defensive `cursor.min(...)`; unbounded loops
   that hang tests instead of failing them. Small discipline, compounding
   returns.
4. **The human's review superpower is premise-checking.** The AI writes
   plausible code fast; the human asks "isn't DFS just scan order?", "isn't
   the quota stack just a counter?", "where does the cursor rest?" — and the
   answers keep dissolving structure that didn't earn its place.

## Stats

27 core tests, all driving the engine through the `validate`/`process` door
with no test-side game state. The lockstep test is now an adaptive script
that plays a real game to victory on two engines and diffs their streams.
The combat core split into five modules (wire / effect / state / engine /
content) with a DAG dependency graph, at the human's instigation — "all in
`lib.rs`" was, correctly, a red flag.

Next: the enemy stops doing nothing (step 3, intent), or the UI gets its
BSN rewrite. The engine is ready for either.
